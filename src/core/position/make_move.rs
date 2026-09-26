//! 着手の適用と巻き戻し、およびその記録。

use super::Position;
use super::lion_trigger::LionTrigger;
use super::zobrist::{ZobristKeys, zobrist_keys};
use crate::core::board::{Bitboard, Square};
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::promotion::{PromotionChoice, in_promotion_zone};
use crate::core::rules::{MoveRules, PromotionRule};

/// 着手で取られた駒の記録。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct CapturedPiece {
    /// 駒が取られた升。
    pub square: Square,
    /// 取られた駒。
    pub piece: PieceCode,
}

/// [`Position::make_move_unchecked`](crate::Position::make_move_unchecked)の巻き戻しに必要な情報。
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct Undo {
    /// 適用した着手。
    pub(crate) mv: Move,
    /// 移動前(成る前)の駒。
    pub(crate) moved_piece_before: PieceCode,
    /// 取られた駒の記録(最大2枚)。
    pub(crate) captured: [Option<CapturedPiece>; 2],
    /// 着手前の先獅子トリガー(獅子捕獲升と麒麟成りフラグ)。
    pub(crate) previous_lion_taken: Option<LionTrigger>,
    /// 着手前のzobristハッシュ。
    pub(crate) previous_zobrist: u64,
    /// 着手前の成り権保留中の駒の集合。
    pub(crate) previous_promotion_deferred: Bitboard,
    /// 着手前のP1成り権保留状態のzobristハッシュ。
    pub(crate) previous_rights_zobrist: u64,
}

/// [`Position::make_null_move`]の巻き戻しに必要な情報。
pub(crate) struct NullUndo {
    /// 手番パス前の先獅子トリガー。
    previous_lion_taken: Option<LionTrigger>,
}

impl Position {
    /// 手番を反転し、zobristハッシュを更新する。
    #[inline]
    fn flip_side_to_move(&mut self, keys: &ZobristKeys) {
        self.side_to_move = self.side_to_move.opposite();
        self.zobrist ^= keys.side_to_move;
    }

    /// 探索用の手番パスを適用し、巻き戻し用トークンを返す。
    ///
    /// 盤上の駒と成り権保留状態(P1・P2・P5)は変更せず、手番を反転して、相手の
    /// 直後の1手だけに適用される先獅子状態を消滅させる(第15条第4・5項)。
    pub(crate) fn make_null_move(&mut self) -> NullUndo {
        let previous_lion_taken = self.lion_taken_by_non_lion;
        let keys = zobrist_keys();
        self.flip_side_to_move(keys);
        self.zobrist ^=
            keys.lion_trigger_state(previous_lion_taken) ^ keys.lion_trigger_state(None);
        self.lion_taken_by_non_lion = None;
        NullUndo {
            previous_lion_taken,
        }
    }

    /// `undo`に記録された手番パスを巻き戻す。
    ///
    /// トークンはこの局面に対する[`Position::make_null_move`]が返した
    /// ものでなければならず、適用と逆の順序で巻き戻す必要がある。
    pub(crate) fn unmake_null_move(&mut self, undo: NullUndo) {
        let keys = zobrist_keys();
        self.flip_side_to_move(keys);
        self.zobrist ^= keys.lion_trigger_state(self.lion_taken_by_non_lion)
            ^ keys.lion_trigger_state(undo.previous_lion_taken);
        self.lion_taken_by_non_lion = undo.previous_lion_taken;
    }

    /// 手番を指定した複製を返す。
    ///
    /// 先獅子トリガーは意図的にそのまま保持する。対局管理層の利き調査は、
    /// どちらの合法手を問うかが変わるだけで、直前の着手による一時状態は
    /// 変わらないという解釈を採るためである。
    pub(crate) fn clone_with_side_to_move(&self, side_to_move: Color) -> Self {
        let mut position = self.clone();
        if position.side_to_move != side_to_move {
            position.flip_side_to_move(zobrist_keys());
        }
        position
    }

    /// 合法性を検査せずに`mv`を適用し、[`Undo`]トークンを返す。
    ///
    /// 呼び出し側は、まさにこの局面に対して、`rules`と同じ規則の
    /// [`MoveGenerator::generate_moves`](crate::MoveGenerator::generate_moves)
    /// が生成した着手だけを渡さなければならない。それ以外の着手を渡すと、
    /// panicするか、zobristハッシュや先獅子トリガーを含めて局面を静かに
    /// 壊すことがある。合法性検査付きの適用には
    /// [`Position::try_make_move`]を使う。
    pub(crate) fn make_move_unchecked(&mut self, mv: Move, rules: MoveRules) -> Undo {
        self.make_move_with_captures_unchecked(mv, rules, self.captured_squares(mv))
    }

    /// 生成済みの捕獲升で合法手を適用する。
    ///
    /// 設計書movegen-speedup-2.md「段階5」に従い、成り判定にも受け取った捕獲升を使う。
    /// 呼び出し側は、この局面の合法手と、それが実際に取る升を渡す必要がある。
    pub(crate) fn make_move_with_captures_unchecked(
        &mut self,
        mv: Move,
        rules: MoveRules,
        capture_squares: [Option<Square>; 2],
    ) -> Undo {
        let keys = zobrist_keys();
        let previous_zobrist = self.zobrist;
        let previous_promotion_deferred = self.promotion_deferred;
        let previous_rights_zobrist = self.rights_zobrist;
        let previous_lion_taken = self.lion_taken_by_non_lion;
        let moving_piece = self.board[mv.from.raw_index()];
        let moving_kind = moving_piece
            .kind()
            .expect("move origin must contain a valid piece");
        let moving_color = self.side_to_move;
        let tracks_promotion_deferred = rules.promotion != PromotionRule::P0 || rules.p5;
        // 「段階6」: 権利を追跡する規則でだけ成りの選択肢を調べる。
        let had_promotion_chance = tracks_promotion_deferred
            && rules.promotion_choice_for(
                moving_color,
                moving_kind,
                moving_piece.is_promoted(),
                mv.from,
                mv.to,
                capture_squares.iter().any(Option::is_some),
                self.promotion_deferred.contains(mv.from),
            ) != PromotionChoice::NoPromotion;
        let was_deferred = self.promotion_deferred.contains(mv.from);
        if tracks_promotion_deferred {
            self.clear_promotion_deferred(mv.from, keys);
            for square in capture_squares.into_iter().flatten() {
                self.clear_promotion_deferred(square, keys);
            }
        }
        if rules.promotion == PromotionRule::P2 {
            // 待機はその側の直後の1手番で満了する(第30条P2)。
            // P5の保留歩兵は恒久状態なので消さない。
            let expiring = self.promotion_deferred & self.pieces_of(moving_color);
            for square in expiring {
                let is_p5_pawn = rules.p5
                    && self.piece_at(square).and_then(PieceCode::kind) == Some(PieceKind::Pawn);
                if !is_p5_pawn {
                    self.clear_promotion_deferred(square, keys);
                }
            }
        }
        let moved_piece_before = self.remove_piece(mv.from, keys);
        debug_assert_eq!(moved_piece_before.color(), Some(self.side_to_move));

        let mut captured = [None; 2];
        for (index, square) in capture_squares.into_iter().enumerate() {
            if let Some(square) = square {
                let piece = self.remove_piece(square, keys);
                debug_assert_eq!(piece.color(), Some(self.side_to_move.opposite()));
                captured[index] = Some(CapturedPiece { square, piece });
            }
        }

        let moved_piece_after = if mv.promote {
            moved_piece_before
                .promote()
                .expect("promoting move must have a promotable piece")
        } else {
            moved_piece_before
        };
        self.put_piece(mv.to, moved_piece_after, keys)
            .expect("generated move must end on an empty square");
        let to_in_zone = in_promotion_zone(moving_color, mv.to);
        let enters_zone = !in_promotion_zone(moving_color, mv.from) && to_in_zone;
        let defer_for_p1 = rules.promotion == PromotionRule::P1
            && had_promotion_chance
            && !mv.promote
            && to_in_zone;
        let defer_for_p2 = rules.promotion == PromotionRule::P2
            && had_promotion_chance
            && !mv.promote
            && enters_zone;
        let defer_for_p5 = rules.p5
            && moving_kind == PieceKind::Pawn
            && !mv.promote
            && (was_deferred || had_promotion_chance);
        if defer_for_p1 || defer_for_p2 || defer_for_p5 {
            debug_assert!(self.promotion_deferred_is_valid(mv.to));
            self.set_promotion_deferred(mv.to, keys);
        }
        self.flip_side_to_move(keys);
        let by_kirin_promotion = moved_piece_before.kind() == Some(PieceKind::Kirin) && mv.promote;
        self.lion_taken_by_non_lion = (moved_piece_before.kind() != Some(PieceKind::Lion))
            .then(|| {
                captured
                    .into_iter()
                    .flatten()
                    .filter(|captured| captured.piece.kind() == Some(PieceKind::Lion))
                    .map(|captured| LionTrigger {
                        square: captured.square,
                        by_kirin_promotion,
                    })
                    .next_back()
            })
            .flatten();
        self.zobrist ^= keys.lion_trigger_state(previous_lion_taken)
            ^ keys.lion_trigger_state(self.lion_taken_by_non_lion);

        Undo {
            mv,
            moved_piece_before,
            captured,
            previous_lion_taken,
            previous_zobrist,
            previous_promotion_deferred,
            previous_rights_zobrist,
        }
    }

    /// `undo`に記録された着手を巻き戻す。
    ///
    /// トークンはこの局面に対する[`Position::make_move_unchecked`]が返した
    /// ものでなければならず、着手は適用と逆の順序で巻き戻す必要がある。
    /// どちらかの前提を破ると、panicするか局面を静かに壊すことがある。
    pub(crate) fn unmake_move(&mut self, undo: Undo) {
        self.side_to_move = self.side_to_move.opposite();
        self.remove_piece_without_hash(undo.mv.to);
        self.put_piece_without_hash(undo.mv.from, undo.moved_piece_before)
            .expect("move origin must be empty while unmaking");
        for captured in undo.captured.into_iter().flatten() {
            self.put_piece_without_hash(captured.square, captured.piece)
                .expect("capture square must be empty while unmaking");
        }
        self.lion_taken_by_non_lion = undo.previous_lion_taken;
        self.zobrist = undo.previous_zobrist;
        debug_assert_eq!(self.zobrist, self.recompute_zobrist());
        debug_assert_eq!(self.rights_zobrist, self.recompute_rights_zobrist());
        self.promotion_deferred = undo.previous_promotion_deferred;
        debug_assert_eq!(
            undo.previous_rights_zobrist,
            self.recompute_rights_zobrist()
        );
        self.rights_zobrist = undo.previous_rights_zobrist;
    }
}

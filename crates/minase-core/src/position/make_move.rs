//! 着手の適用と巻き戻し、およびその記録。

use super::Position;
use super::lion_trigger::LionTrigger;
use super::zobrist::{ZobristKeys, zobrist_keys};
use crate::board::{Bitboard, Square};
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::promotion::{PromotionChoice, in_promotion_zone};
use crate::rules::{MoveRules, PromotionRule};

/// 着手で取られた駒の記録。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CapturedPiece {
    /// 駒が取られた升。
    pub square: Square,
    /// 取られた駒。
    pub piece: PieceCode,
}

/// [`Position::make_move_unchecked`](crate::Position::make_move_unchecked)の巻き戻しに必要な情報。
#[derive(PartialEq, Eq, Debug)]
pub struct Undo {
    /// 適用した着手。
    pub mv: Move,
    /// 移動前(成る前)の駒。
    pub moved_piece_before: PieceCode,
    /// 取られた駒の記録(最大2枚)。
    pub captured: [Option<CapturedPiece>; 2],
    /// 着手前の先獅子トリガー(獅子捕獲升と麒麟成りフラグ)。
    pub previous_lion_taken: Option<LionTrigger>,
    /// 着手前のzobristハッシュ。
    pub(crate) previous_zobrist: u64,
    /// 着手前の成り権保留中の駒の集合。
    pub(crate) previous_promotion_deferred: Bitboard,
    /// 着手前の成り権保留状態（P1・P2・P5）のZobristハッシュ。
    pub(crate) previous_rights_zobrist: u64,
}

/// [`Position::make_null_move`]の巻き戻しに必要な情報。
pub struct NullUndo {
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
    pub fn make_null_move(&mut self) -> NullUndo {
        let previous_lion_taken = self.lion_taken_by_non_lion;
        let keys = zobrist_keys();
        self.flip_side_to_move(keys);
        self.zobrist ^=
            keys.lion_trigger_state(previous_lion_taken) ^ keys.lion_trigger_state(None);
        self.lion_taken_by_non_lion = None;
        #[cfg(feature = "invariants")]
        self.assert_invariants("make null move", None);
        NullUndo {
            previous_lion_taken,
        }
    }

    /// `undo`に記録された手番パスを巻き戻す。
    ///
    /// トークンはこの局面に対する[`Position::make_null_move`]が返した
    /// ものでなければならず、適用と逆の順序で巻き戻す必要がある。
    pub fn unmake_null_move(&mut self, undo: NullUndo) {
        let keys = zobrist_keys();
        self.flip_side_to_move(keys);
        self.zobrist ^= keys.lion_trigger_state(self.lion_taken_by_non_lion)
            ^ keys.lion_trigger_state(undo.previous_lion_taken);
        self.lion_taken_by_non_lion = undo.previous_lion_taken;
        #[cfg(feature = "invariants")]
        self.assert_invariants("unmake null move", None);
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
    /// 適用の手順は[`Self::make_move_with_captures_unchecked`]に従う。
    pub fn make_move_unchecked(&mut self, mv: Move, rules: MoveRules) -> Undo {
        self.make_move_with_captures_unchecked(mv, rules, self.captured_squares(mv))
    }

    /// 生成済みの捕獲升で合法手を適用する。
    ///
    /// 着手の前提条件は[`Self::make_move_unchecked`]と同じである。
    /// `capture_squares`は[`Self::captured_squares`]に`mv`を渡した結果と一致しなければならない。
    /// 駒の除去と成りの判定の両方にこの値を使い、捕獲升の再計算を省く。
    ///
    /// 適用は次の順序で行う。
    ///
    /// 1. P0以外またはP5では、移動元と捕獲升の成り権保留ビットを消す。
    ///    P2では着手側の待機ビットを満了させるが、P5の保留歩兵は残す。
    /// 2. 移動元の駒を除き、経由升、到達升の順に相手駒を除く。
    ///    第2段階は第1段階の捕獲後の盤面で行う（RULES.md第7条第8項）。
    /// 3. `mv.promote`なら成った駒を到達升へ置く。
    ///    成りは着手全体の終了時に適用する（第18条第6項）。
    /// 4. 不成の着手について、P1では成れる着手で到達升が敵陣なら保留する。
    ///    P2では成れる着手で敵陣外から敵陣へ入った場合に待機させる。
    ///    P5では歩兵が既に保留中か、成れる着手だった場合に保留する。
    /// 5. 手番を反転し、先獅子の状態を更新する。
    ///
    /// `rules`は成り権の追跡にだけ使い、先獅子の記録は規則に依存しない。
    /// P0かつP5なしでは成り権を追跡せず、`promotion_choice_for`も呼ばない。
    ///
    /// 成る前の移動駒が獅子以外で、相手の獅子を取ったときに捕獲升を記録する。
    /// 麒麟が同じ着手で成った場合もこの捕獲に含め、`by_kirin_promotion`を立てる（第15条第7項）。
    /// 獅子による獅子の捕獲と、獅子を取らない着手では記録を消す（第15条第4項から第6項）。
    /// 角鷹または飛鷲が1手で獅子を2枚取る場合は、後に取った到達升側の獅子の升を記録する。
    /// 記録と足の判定を分ける理由は
    /// `docs/adr/record-lion-trigger-on-move-and-judge-foot-in-movegen.md`にある。
    pub fn make_move_with_captures_unchecked(
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

        #[cfg(feature = "invariants")]
        self.assert_invariants("make move", Some(mv));

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
    pub fn unmake_move(&mut self, undo: Undo) {
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
        #[cfg(not(feature = "invariants"))]
        debug_assert_eq!(self.zobrist, self.recompute_zobrist());
        #[cfg(not(feature = "invariants"))]
        debug_assert_eq!(self.rights_zobrist, self.recompute_rights_zobrist());
        self.promotion_deferred = undo.previous_promotion_deferred;
        #[cfg(not(feature = "invariants"))]
        debug_assert_eq!(
            undo.previous_rights_zobrist,
            self.recompute_rights_zobrist()
        );
        self.rights_zobrist = undo.previous_rights_zobrist;
        #[cfg(feature = "invariants")]
        self.assert_invariants("unmake move", Some(undo.mv));
    }
}

#[cfg(feature = "invariants")]
impl Position {
    /// `invariants`フィーチャで着手、ヌルムーブ、および各取り消しの後に不変条件を検査する。
    ///
    /// [`Self::validate`]が失敗したら、操作名、着手、手番、両Zobrist値の増分値と再計算値、
    /// および盤面の生の駒コードを含む診断でパニックする。
    /// 拡張SFEN用の局面を構築できる場合は、その表記も加える。
    fn assert_invariants(&self, operation: &str, mv: Option<Move>) {
        use crate::notation::sfen::{SetupPosition, to_extended_sfen};
        use std::fmt::Write;

        let Err(error) = self.validate() else {
            return;
        };
        let mut diagnostic = format!(
            "position invariant failed after {operation}: {error}\nmove: {mv:?}\nside to move: {:?}\nzobrist: incremental={:#018x}, recomputed={:#018x}\nrights zobrist: incremental={:#018x}, recomputed={:#018x}\nboard raw codes (rank 0..11, file 0..11):\n",
            self.side_to_move,
            self.zobrist,
            self.recompute_zobrist(),
            self.rights_zobrist,
            self.recompute_rights_zobrist(),
        );
        // PieceCodeのDebug表現は内部のu8をそのまま表示する。
        // 空升・番兵も失わないようにpiece_atではなく盤面配列を読む。
        for rank in 0..12 {
            writeln!(diagnostic, "{:?}", &self.board[rank * 16..rank * 16 + 12])
                .expect("writing to a String cannot fail");
        }
        if let Ok(setup) = SetupPosition::new(
            self.clone(),
            self.lion_taken_by_non_lion.map(|trigger| trigger.square),
            1,
        ) {
            writeln!(diagnostic, "extended SFEN: {}", to_extended_sfen(&setup))
                .expect("writing to a String cannot fail");
        }
        panic!("{diagnostic}");
    }
}

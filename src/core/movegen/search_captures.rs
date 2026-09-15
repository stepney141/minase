//! 静止探索で必要な捕獲だけを直接出力する生成経路。

use super::*;
use crate::core::piece::PieceCode;

/// 生成時に得た捕獲升を持つ探索専用の候補。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureCandidate {
    pub(crate) mv: Move,
    /// 成り展開前の、動かす駒のコード。
    pub(crate) piece: PieceCode,
    pub(crate) captured: [Option<Square>; 2],
}

/// 対象升へ実際に届く通常駒。利きはノードの初期化時に一度だけ求める。
#[derive(Clone, Copy)]
pub(crate) struct OrdinaryCapturer {
    pub(crate) from: Square,
    pub(crate) kind: PieceKind,
    pub(crate) piece: PieceCode,
    pub(crate) captures: Bitboard,
}

impl MoveGenerator {
    /// 特殊駒の全捕獲を公開生成と同じ相対順序で追加する。
    pub(crate) fn generate_special_captures(
        &self,
        position: &Position,
        output: &mut Vec<CaptureCandidate>,
    ) {
        let color = position.side_to_move();
        for kind in [
            PieceKind::Lion,
            PieceKind::HornedFalcon,
            PieceKind::SoaringEagle,
        ] {
            for from in position.pieces_of_kind(color, kind) {
                let piece = position.piece_at(from).expect("capture origin has a piece");
                self.generate_special_piece_captures(position, from, piece, output);
            }
        }
    }

    /// 特殊駒1枚の捕獲を生成し、置換表の手の検査にも使う。
    fn generate_special_piece_captures(
        &self,
        position: &Position,
        from: Square,
        piece: PieceCode,
        output: &mut Vec<CaptureCandidate>,
    ) {
        let color = position.side_to_move();
        let kind = piece.kind().expect("capture origin has a kind");
        let enemy = position.pieces_of(color.opposite());
        let lions = position.pieces_of_kind(color.opposite(), PieceKind::Lion);
        let special = movement_profile_data(movement_profile(kind)).special;
        let mut emit = |mv: Move| {
            let captured = mv
                .capture_candidates()
                .map(|s| s.filter(|&s| enemy.contains(s)));
            self.push_capture(
                position,
                kind,
                piece,
                lions,
                CaptureCandidate {
                    mv,
                    piece,
                    captured,
                },
                output,
            );
        };
        let steps =
            (piece_control_without_special(self.tables(), position.occupied(), color, kind, from)
                | special_step_destinations(self.tables(), color, from, special))
                & enemy;
        for to in steps {
            emit(Move {
                from,
                mid: None,
                to,
                promote: false,
            });
        }
        match special {
            SpecialMovement::Lion => generate_lion_double_and_jumps::<true>(
                self.tables(),
                position,
                color,
                from,
                &mut emit,
            ),
            SpecialMovement::LionLike(profile) => generate_lion_like_double_and_jumps::<true>(
                position, color, from, profile, &mut emit,
            ),
            SpecialMovement::None => {
                unreachable!("special capture generator requires a special piece")
            }
        }
    }

    /// 遮蔽なしの到達範囲で絞り、通常駒の捕獲可能升を1回だけ計算する。
    pub(crate) fn collect_ordinary_capturers(
        &self,
        position: &Position,
        allowed: Bitboard,
        output: &mut Vec<OrdinaryCapturer>,
    ) {
        if allowed.is_empty() {
            return;
        }
        let color = position.side_to_move();
        for kind in PieceKind::ALL {
            if !matches!(
                movement_profile_data(movement_profile(kind)).special,
                SpecialMovement::None
            ) {
                continue;
            }
            for from in position.pieces_of_kind(color, kind) {
                let piece = position.piece_at(from).expect("capture origin has a piece");
                if let Some(capturer) = self.ordinary_capturer(position, from, piece, allowed) {
                    output.push(capturer);
                }
            }
        }
    }

    /// 通常駒1枚について、対象升へ実際に届く利きを求める。
    fn ordinary_capturer(
        &self,
        position: &Position,
        from: Square,
        piece: PieceCode,
        allowed: Bitboard,
    ) -> Option<OrdinaryCapturer> {
        let color = position.side_to_move();
        let kind = piece.kind().expect("capture origin has a kind");
        if (self.tables().reach(color, movement_profile(kind), from) & allowed).is_empty() {
            return None;
        }
        let captures =
            piece_control_without_special(self.tables(), position.occupied(), color, kind, from)
                & allowed;
        (!captures.is_empty()).then_some(OrdinaryCapturer {
            from,
            kind,
            piece,
            captures,
        })
    }

    /// 保存済みの利きから、対象升への捕獲を公開生成と同じ相対順序で追加する。
    pub(crate) fn emit_ordinary_captures(
        &self,
        position: &Position,
        capturers: &[OrdinaryCapturer],
        targets: Bitboard,
        output: &mut Vec<CaptureCandidate>,
    ) {
        let lions = position.pieces_of_kind(position.side_to_move().opposite(), PieceKind::Lion);
        for capturer in capturers {
            for to in capturer.captures & targets {
                self.push_capture(
                    position,
                    capturer.kind,
                    capturer.piece,
                    lions,
                    CaptureCandidate {
                        mv: Move {
                            from: capturer.from,
                            mid: None,
                            to,
                            promote: false,
                        },
                        piece: capturer.piece,
                        captured: [None, Some(to)],
                    },
                    output,
                );
            }
        }
    }

    /// 再利用バッファへ1駒分の捕獲だけを生成し、置換表の手を検査する。
    pub(crate) fn is_legal_capture(
        &self,
        position: &Position,
        mv: Move,
        output: &mut Vec<CaptureCandidate>,
    ) -> bool {
        output.clear();
        let Some(piece) = position.piece_at(mv.from) else {
            return false;
        };
        if piece.color() != Some(position.side_to_move())
            || position
                .captured_squares(mv)
                .into_iter()
                .all(|s| s.is_none())
        {
            return false;
        }
        let kind = piece.kind().expect("capture origin has a kind");
        if matches!(
            movement_profile_data(movement_profile(kind)).special,
            SpecialMovement::None
        ) {
            if mv.mid.is_some() {
                return false;
            }
            let enemy = position.pieces_of(position.side_to_move().opposite());
            if let Some(capturer) = self.ordinary_capturer(position, mv.from, piece, enemy)
                && capturer.captures.contains(mv.to)
            {
                let mut target = Bitboard::EMPTY;
                target.set(mv.to);
                self.emit_ordinary_captures(position, &[capturer], target, output);
            }
        } else {
            self.generate_special_piece_captures(position, mv.from, piece, output);
        }
        output.iter().any(|candidate| candidate.mv == mv)
    }

    /// 獅子捕獲のときだけ規則を検査し、成りを展開して直接出力する。
    fn push_capture(
        &self,
        position: &Position,
        kind: PieceKind,
        piece: PieceCode,
        lions: Bitboard,
        candidate: CaptureCandidate,
        output: &mut Vec<CaptureCandidate>,
    ) {
        if candidate
            .captured
            .into_iter()
            .flatten()
            .any(|s| lions.contains(s))
            && !self.rules().special_move_is_legal(position, candidate.mv)
        {
            return;
        }
        let choice = if piece.is_promoted() || !kind.can_promote() {
            PromotionChoice::NoPromotion
        } else {
            self.rules().promotion_choice(position, &candidate.mv, kind)
        };
        if !matches!(choice, PromotionChoice::PromotionForced) {
            output.push(candidate);
        }
        if !matches!(choice, PromotionChoice::NoPromotion) {
            output.push(CaptureCandidate {
                mv: promoting_variant(candidate.mv),
                ..candidate
            });
        }
    }
}

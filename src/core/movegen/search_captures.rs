//! 静止探索で必要な捕獲だけを直接出力する生成経路。

use super::*;
use crate::core::piece::PieceCode;

#[cfg(test)]
#[path = "tests/ordinary_capturer.rs"]
mod tests;

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

/// 設計書movegen-speedup-2.md「段階5」に従い、置換表の検証で得た利きを保存する。
#[derive(Default)]
pub(crate) struct CaptureCache {
    pub(crate) ordinary: Option<OrdinaryCapturer>,
    special_from: Option<Square>,
    captures: Vec<CaptureCandidate>,
}

impl MoveGenerator {
    /// 特殊駒の全捕獲を公開生成と同じ相対順序で追加する。
    pub(crate) fn generate_special_captures(
        &self,
        position: &Position,
        cache: &CaptureCache,
        output: &mut impl FnMut(CaptureCandidate),
    ) {
        let color = position.side_to_move();
        for kind in [
            PieceKind::Lion,
            PieceKind::HornedFalcon,
            PieceKind::SoaringEagle,
        ] {
            for from in position.pieces_of_kind(color, kind) {
                let piece = position.piece_at(from).expect("capture origin has a piece");
                if cache.special_from == Some(from) {
                    for &candidate in &cache.captures {
                        output(candidate);
                    }
                } else {
                    self.generate_special_piece_captures(position, from, piece, output);
                }
            }
        }
    }

    /// 特殊駒1枚の捕獲を生成し、置換表の手の検査にも使う。
    fn generate_special_piece_captures(
        &self,
        position: &Position,
        from: Square,
        piece: PieceCode,
        output: &mut impl FnMut(CaptureCandidate),
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
            SpecialMovement::Lion => generate_lion_double_and_jumps::<true, false>(
                self.tables(),
                position,
                color,
                from,
                &mut emit,
            ),
            SpecialMovement::LionLike(profile) => {
                generate_lion_like_double_and_jumps::<true, false>(
                    position, color, from, profile, &mut emit,
                )
            }
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
        cached: Option<OrdinaryCapturer>,
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
                if let Some(mut capturer) = cached.filter(|c| c.from == from) {
                    capturer.captures &= allowed;
                    if !capturer.captures.is_empty() {
                        output.push(capturer);
                    }
                } else if let Some(capturer) =
                    self.ordinary_capturer(position, from, piece, allowed)
                {
                    output.push(capturer);
                }
            }
        }
    }

    /// 通常駒1枚について、対象升へ実際に届く利きを求める。
    /// 設計書movegen-speedup-2.md「段階2」に従い、対象升と交わる利き線だけを調べる。
    fn ordinary_capturer(
        &self,
        position: &Position,
        from: Square,
        piece: PieceCode,
        allowed: Bitboard,
    ) -> Option<OrdinaryCapturer> {
        let color = position.side_to_move();
        let kind = piece.kind().expect("capture origin has a kind");
        let tables = self.tables();
        let profile_id = movement_profile(kind);
        if (tables.reach(color, profile_id, from) & allowed).is_empty() {
            return None;
        }
        let mut captures = tables.fixed(color, profile_id, from) & allowed;
        let profile = movement_profile_data(profile_id);
        for slide in profile.slides {
            let direction = slide.direction.for_color(color);
            if !(tables.ray(from, direction) & allowed).is_empty() {
                captures |= tables.sliding_control(from, direction, position.occupied()) & allowed;
            }
        }
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
        output: &mut impl FnMut(CaptureCandidate),
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

    /// 1駒分の捕獲で置換表の手を検査し、利きを初期化へ引き継ぐ。
    /// 設計書movegen-speedup-2.md「段階5」に従う。
    pub(crate) fn is_legal_capture(
        &self,
        position: &Position,
        mv: Move,
        cache: &mut CaptureCache,
    ) -> bool {
        cache.clear();
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
            cache.ordinary = self.ordinary_capturer(position, mv.from, piece, enemy);
            if let Some(capturer) = cache.ordinary
                && capturer.captures.contains(mv.to)
            {
                let mut legal = false;
                self.emit_ordinary_captures(
                    position,
                    &[capturer],
                    Bitboard::from_squares([mv.to]),
                    &mut |candidate| legal |= candidate.mv == mv,
                );
                return legal;
            }
            false
        } else {
            cache.special_from = Some(mv.from);
            self.generate_special_piece_captures(position, mv.from, piece, &mut |candidate| {
                cache.captures.push(candidate)
            });
            cache.captures.iter().any(|candidate| candidate.mv == mv)
        }
    }

    /// 獅子捕獲のときだけ規則を検査し、成りを展開して直接出力する。
    /// 設計書movegen-speedup-2.md「段階5」に従い、生成済みの駒コードと捕獲升を使う。
    fn push_capture(
        &self,
        position: &Position,
        kind: PieceKind,
        piece: PieceCode,
        lions: Bitboard,
        candidate: CaptureCandidate,
        output: &mut impl FnMut(CaptureCandidate),
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
            self.rules().promotion_choice_for(
                position.side_to_move(),
                kind,
                piece.is_promoted(),
                candidate.mv.from,
                candidate.mv.to,
                candidate.captured.iter().any(Option::is_some),
                position.promotion_deferred().contains(candidate.mv.from),
            )
        };
        if !matches!(choice, PromotionChoice::PromotionForced) {
            output(candidate);
        }
        if !matches!(choice, PromotionChoice::NoPromotion) {
            output(CaptureCandidate {
                mv: promoting_variant(candidate.mv),
                ..candidate
            });
        }
    }
}

impl CaptureCache {
    pub(crate) fn clear(&mut self) {
        self.ordinary = None;
        self.special_from = None;
        self.captures.clear();
    }

    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.captures.capacity()
    }
}

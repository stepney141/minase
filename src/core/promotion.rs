//! 成りの選択肢と敵陣の判定。

use crate::core::board::{BOARD_RANKS, Square};
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::rules::{MoveRules, PromotionRule};

/// 着手に対する成りの選択肢。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PromotionChoice {
    /// この着手では成れない。
    NoPromotion,
    /// 成るか成らないかを選択できる(第18条第5項)。
    PromotionOptional,
    /// 必ず成る。不成の着手は生成しない(第30条P5・P6)。
    PromotionForced,
}

/// 指定升が指定対局者の敵陣(相手側の最奥4段、第3条)にあるかどうかを返す。
#[inline]
pub const fn in_promotion_zone(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() >= BOARD_RANKS - 4,
        Color::White => square.rank() < 4,
    }
}

impl MoveRules {
    /// 着手で成りを選択できるかどうかを判定して返す(第18条・第19条・第30条)。
    pub(crate) fn promotion_choice(
        self,
        position: &Position,
        mv: &Move,
        moving_kind: PieceKind,
    ) -> PromotionChoice {
        let Some(piece) = position.piece_at(mv.from) else {
            return PromotionChoice::NoPromotion;
        };
        let Some(color) = piece.color() else {
            return PromotionChoice::NoPromotion;
        };
        let has_capture = position
            .captured_squares(*mv)
            .into_iter()
            .any(|capture| capture.is_some());
        let deferred = position.promotion_deferred().contains(mv.from);

        self.promotion_choice_for(
            color,
            moving_kind,
            piece.is_promoted(),
            mv.from,
            mv.to,
            has_capture,
            deferred,
        )
    }

    /// 指定した駒状態と捕獲条件について、成りの選択肢を返す。
    ///
    /// 実局面に存在しない交換列からも使える規則判定であり、第18条、第19条、
    /// および第30条P1からP6までを適用する。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn promotion_choice_for(
        self,
        color: Color,
        moving_kind: PieceKind,
        is_promoted: bool,
        from: Square,
        to: Square,
        has_capture: bool,
        deferred: bool,
    ) -> PromotionChoice {
        if is_promoted || !moving_kind.can_promote() {
            return PromotionChoice::NoPromotion;
        }

        let from_in_zone = in_promotion_zone(color, from);
        let to_in_zone = in_promotion_zone(color, to);
        let enters_zone = !from_in_zone && to_in_zone;
        let capture_in_or_from_zone = has_capture && (from_in_zone || to_in_zone);
        let reaches_last_rank = match color {
            Color::Black => to.rank() == BOARD_RANKS - 1,
            Color::White => to.rank() == 0,
        };

        if self.p5 && moving_kind == PieceKind::Pawn && deferred {
            // 保留歩兵は、採用中の成り規則で成れる着手のうち到達升が最奥段である
            // 着手でのみ、必ず成る(第30条P5)。標準規則では敵陣内の捕獲着手に
            // 限られ、P2では非捕獲着手も該当する。
            let promotable = capture_in_or_from_zone
                || (self.promotion == PromotionRule::P2
                    && !has_capture
                    && (from_in_zone || to_in_zone));
            return if promotable && reaches_last_rank {
                PromotionChoice::PromotionForced
            } else {
                PromotionChoice::NoPromotion
            };
        }

        let piece_reaches_last_rank_without_capture = !has_capture
            && reaches_last_rank
            && match moving_kind {
                PieceKind::Pawn => !self.p5,
                PieceKind::Lance => self.p3,
                PieceKind::GoBetween => self.p4,
                _ => false,
            };
        let p2_waiting_promotion = self.promotion == PromotionRule::P2
            && !has_capture
            && (from_in_zone || to_in_zone)
            && !deferred;
        let p1_recovered_promotion = self.promotion == PromotionRule::P1
            && from_in_zone
            && to_in_zone
            && !has_capture
            && !deferred;

        if enters_zone
            || capture_in_or_from_zone
            || piece_reaches_last_rank_without_capture
            || p2_waiting_promotion
            || p1_recovered_promotion
        {
            if self.p6
                && matches!(moving_kind, PieceKind::Pawn | PieceKind::Lance)
                && reaches_last_rank
            {
                PromotionChoice::PromotionForced
            } else {
                PromotionChoice::PromotionOptional
            }
        } else {
            PromotionChoice::NoPromotion
        }
    }
}

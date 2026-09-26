//! 候補手の登録と成りの展開。

use super::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::PieceKind;
use crate::core::position::Position;
use crate::core::promotion::PromotionChoice;

/// 同じ着手の成りを選んだ変種を返す。
pub(super) fn promoting_variant(mv: Move) -> Move {
    Move {
        promote: true,
        ..mv
    }
}

/// 獅子の捕獲制限を検査し、成りの選択肢(第18条)を展開して着手を追加する。
pub(super) fn push_with_promotion(
    generator: &MoveGenerator,
    position: &Position,
    moving_kind: PieceKind,
    base: Move,
    output: &mut Vec<Move>,
) {
    if !generator.rules().special_move_is_legal(position, base) {
        return;
    }

    match generator
        .rules()
        .promotion_choice(position, &base, moving_kind)
    {
        PromotionChoice::NoPromotion => output.push(base),
        PromotionChoice::PromotionOptional => {
            output.push(base);
            output.push(promoting_variant(base));
        }
        PromotionChoice::PromotionForced => output.push(promoting_variant(base)),
    }
}

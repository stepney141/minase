//! 候補盤面の成り権保留と先獅子の記録升を具体的な値として列挙する。

use super::lion_membership;
use crate::core::bitboard::Bitboard;
use crate::core::mv::Move;
use crate::core::piece::{PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::{MoveRules, PromotionRule, in_promotion_zone};
use crate::core::square::Square;

/// 盤面を復元し、捕獲駒の保留と着手側の保留・待機を列挙する。
pub(super) fn candidates(
    rules: MoveRules,
    target: &Position,
    mv: Move,
    before: PieceCode,
    captured: &[(Square, PieceCode)],
) -> Vec<Position> {
    let mover = target.side_to_move().opposite();
    let mut affected = Bitboard::EMPTY;
    affected.set(mv.from);
    affected.set(mv.to);
    if let Some(mid) = mv.mid {
        affected.set(mid);
    }
    for &(square, _) in captured {
        affected.set(square);
    }
    let mut pieces: Vec<_> = (target.occupied() & !affected)
        .iter()
        .map(|square| (square, target.piece_at(square).expect("occupied piece")))
        .collect();
    pieces.push((mv.from, before));
    pieces.extend_from_slice(captured);
    let mut inherited = target.promotion_deferred() & !affected;
    let mut optional = Vec::new();
    let mut waiting = Vec::new();
    for &(square, piece) in &pieces {
        let expiring = rules.promotion == PromotionRule::P2
            && piece.color() == Some(mover)
            && !(rules.p5 && piece.kind() == Some(PieceKind::Pawn));
        if expiring {
            inherited.clear(square);
        }
        if can_defer(rules, square, piece) {
            if expiring {
                waiting.push(square);
            } else if affected.contains(square) {
                optional.push(square);
            }
        }
    }
    let mut result = Vec::new();
    for bits in 0..(1 << optional.len()) {
        let mut deferred = inherited;
        for (index, &square) in optional.iter().enumerate() {
            if bits & (1 << index) != 0 {
                deferred.set(square);
            }
        }
        // P2の着手側は待機なし、または適格な1駒だけのk+1通りに限る。
        for wait in std::iter::once(None).chain(waiting.iter().copied().map(Some)) {
            let mut deferred = deferred;
            if let Some(square) = wait {
                deferred.set(square);
            }
            let mut builder = PositionBuilder::new(mover);
            for &(square, piece) in &pieces {
                builder
                    .put(square, piece)
                    .expect("restored squares are distinct");
            }
            for square in deferred.iter() {
                builder
                    .mark_promotion_deferred(square)
                    .expect("deferred pieces are eligible");
            }
            result.push(builder.finish().expect("reconstructed board is consistent"));
        }
    }
    result
}

/// 検査済み基底のクローンへ先獅子の記録升を設定し、その局所条件だけを再検査する。
pub(super) fn lion_records(base: &Position, missing_lion: bool, mut visit: impl FnMut(Position)) {
    visit(base.clone());
    if !missing_lion {
        return;
    }
    for square in Square::all() {
        let mut candidate = base.clone();
        if candidate.set_lion_capture(Some(square)).is_ok()
            && lion_membership(&candidate, missing_lion).is_ok()
        {
            visit(candidate);
        }
    }
}

/// 採用規則の下で成り権保留状態を持てる配置かを返す(第30条)。
fn can_defer(rules: MoveRules, square: Square, piece: PieceCode) -> bool {
    let kind = piece.kind().expect("board piece has a kind");
    !piece.is_promoted()
        && kind.can_promote()
        && in_promotion_zone(piece.color().expect("board piece has an owner"), square)
        && (rules.promotion != PromotionRule::P0 || (rules.p5 && kind == PieceKind::Pawn))
}

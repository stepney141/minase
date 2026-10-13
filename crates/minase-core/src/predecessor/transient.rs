//! 候補盤面の成り権保留と先獅子の記録升を具体的な値として列挙する。

use super::lion_record_squares;
use crate::board::Bitboard;
use crate::board::Square;
use crate::mv::Move;
use crate::piece::{PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::promotion::in_promotion_zone;
use crate::rules::{MoveRules, PromotionRule};

/// 盤面を復元し、捕獲駒の保留と着手側の保留・待機を列挙する。
///
/// 到達升のビットは直前の着手で新設された可能性があるので、対象局面から引き継がない。
/// P1とP5では、移動元と捕獲升のうち保留可能な駒に、保留ありとなしを独立に割り当てる。
/// P2の着手側では待機なしの1通りと、敵陣内の成れる不成駒のうち
/// P5の歩兵を除く`k`個から1駒だけを待機させる`k`通りを列挙する。
/// 計`k + 1`通りとし、2駒以上の待機は[`predecessor`](crate::predecessor)の集合Aに反するので作らない。
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

/// 検査済み基底のクローンへ、局所条件を満たす先獅子の記録升を設定して渡す。
///
/// 記録升の候補は[`lion_record_squares`]で先に絞る。
/// 全144升へ設定してから検査する方式を採らない理由は
/// `docs/adr/verify-predecessor-candidates-with-forward-movegen.md`にある。
/// 戻り値は構成した候補数(基底を含む)。
pub(super) fn lion_records(
    base: &Position,
    missing_lion: bool,
    mut visit: impl FnMut(Position),
) -> usize {
    visit(base.clone());
    if !missing_lion {
        return 1;
    }
    let squares = lion_record_squares(base);
    for square in squares.iter() {
        let mut candidate = base.clone();
        candidate
            .set_lion_capture(Some(square))
            .expect("record squares never hold a piece of the side to move");
        visit(candidate);
    }
    1 + squares.popcount() as usize
}

/// 採用規則の下で成り権保留状態を持てる配置かを返す(第30条)。
fn can_defer(rules: MoveRules, square: Square, piece: PieceCode) -> bool {
    let kind = piece.kind().expect("board piece has a kind");
    !piece.is_promoted()
        && kind.can_promote()
        && in_promotion_zone(piece.color().expect("board piece has an owner"), square)
        && (rules.promotion != PromotionRule::P0 || (rules.p5 && kind == PieceKind::Pawn))
}

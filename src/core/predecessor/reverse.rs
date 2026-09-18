//! 移動の幾何と不足在庫から候補を構成し、順方向再適用で絞り込む。

use std::collections::HashSet;

use super::{base_membership, lion_missing, material, transient};
use crate::core::attacks::{SpecialMovement, movement_profile, movement_profile_data};
use crate::core::bitboard::Bitboard;
use crate::core::direction::step_square;
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::PieceCode;
use crate::core::position::Position;
use crate::core::square::Square;

/// 集合Aに属する対象局面へ到達する直前局面を返す。
pub(super) fn generate(forward: &MoveGenerator, target: &Position) -> Vec<Position> {
    verify_candidates(forward, target, |visit| {
        enumerate_candidates(forward, target, visit);
    })
}

/// 集合Aの検査を通った着手と候補局面の組を列挙する。
/// 基底での検査と記録升の検査の順序を保ち、組をコールバックへ逐次渡す。
/// 全候補を保持せず、計測時も従来と同じ作業領域で再適用する。
/// 戻り値は集合Aの検査で棄却した局面も含む候補構成数。
pub(super) fn enumerate_candidates(
    forward: &MoveGenerator,
    target: &Position,
    mut visit: impl FnMut(Move, Position),
) -> usize {
    let mut constructed = 0;
    let mover = target.side_to_move().opposite();
    let mut missing = material::missing(target);
    // 着手側の由来別在庫は移動・成り・相手駒の復元では変わらない。
    let missing_lion = lion_missing(&material::count(target), mover);
    for to in target.pieces_of(mover).iter() {
        let arrived = target.piece_at(to).expect("mover square has a piece");
        let mut before_choices = vec![(arrived, false)];
        if arrived.is_promoted() {
            let origin = arrived
                .kind()
                .expect("board piece has a kind")
                .unpromoted()
                .expect("promoted piece has an origin");
            before_choices.push((
                PieceCode::new(mover, origin).expect("origin is unpromoted"),
                true,
            ));
        }
        for (before, promote) in before_choices {
            for mv in moves(forward, target, to, before, promote) {
                restorations(&mut missing, target, mv, |captured| {
                    for candidate in
                        transient::candidates(forward.rules(), target, mv, before, captured)
                    {
                        // validate・在庫・保留集合は記録升に依存しないので基底で1回検査する。
                        if base_membership(forward.rules(), &candidate).is_err() {
                            constructed += 1;
                            continue;
                        }
                        constructed +=
                            transient::lion_records(&candidate, missing_lion, |candidate| {
                                visit(mv, candidate);
                            });
                    }
                });
            }
        }
    }
    constructed
}

/// 列挙済み候補を順方向に再適用し、一致する局面を重複なく返す。
pub(super) fn verify_candidates(
    forward: &MoveGenerator,
    target: &Position,
    enumerate: impl FnOnce(&mut dyn FnMut(Move, Position)),
) -> Vec<Position> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    enumerate(&mut |mv, candidate| {
        let mut replayed = candidate.clone();
        if replayed.try_make_move(mv, forward).is_ok()
            && replayed == *target
            && seen.insert(candidate.clone())
        {
            result.push(candidate);
        }
    });
    result
}

/// 通常移動と獅子力の幾何を反転し、捕獲復元前に候補着手を重複排除する。
fn moves(
    forward: &MoveGenerator,
    target: &Position,
    to: Square,
    before: PieceCode,
    promote: bool,
) -> HashSet<Move> {
    let mover = before.color().expect("moving piece has an owner");
    let profile = movement_profile_data(movement_profile(
        before.kind().expect("moving piece has a kind"),
    ));
    let mut result = HashSet::new();
    let mut add = |from, mid: Option<Square>| {
        if (from == to || target.piece_at(from).is_none())
            && mid.is_none_or(|square| target.piece_at(square).is_none())
        {
            result.insert(Move {
                from,
                mid,
                to,
                promote,
            });
        }
    };
    for delta in profile.fixed_deltas {
        let (df, dr) = delta.for_color(mover);
        if let Some(from) = to.offset(-df, -dr) {
            add(from, None);
        }
    }
    for slide in profile.slides {
        // 捕獲駒を復元した盤面での遮蔽判定を順方向生成器へ委ねる。
        for from in forward
            .tables()
            .sliding_control(
                to,
                slide.direction.for_color(mover).opposite(),
                Bitboard::EMPTY,
            )
            .iter()
        {
            add(from, None);
        }
    }
    match profile.special {
        SpecialMovement::None => {}
        SpecialMovement::Lion => {
            for from in (forward.tables().king_steps(to) | forward.tables().lion_jumps(to)).iter() {
                add(from, None);
            }
            add(to, None);
            for mid in forward.tables().king_steps(to).iter() {
                for from in forward.tables().king_steps(mid).iter() {
                    add(from, Some(mid));
                }
            }
        }
        SpecialMovement::LionLike(profile) => {
            add(to, None);
            for direction in profile.directions {
                let dir = direction.for_color(mover);
                if let Some(first) = step_square(to, dir.opposite()) {
                    add(first, None);
                    if let Some(from) = step_square(first, dir.opposite()) {
                        add(from, None);
                        add(from, Some(first));
                    }
                }
                if let Some(mid) = step_square(to, dir) {
                    add(to, Some(mid));
                }
            }
        }
    }
    result
}

/// 経由升では必ず捕獲駒を復元し、到達升では非捕獲と捕獲を列挙する。
fn restorations(
    missing: &mut material::Inventory,
    target: &Position,
    mv: Move,
    mut visit: impl FnMut(&[(Square, PieceCode)]),
) {
    let mut destination = |captured: &[(Square, PieceCode)],
                           remaining: &mut material::Inventory| {
        visit(captured);
        if mv.to != mv.from {
            material::for_each_restoration(remaining, target.side_to_move(), |piece, _| {
                let mut both = captured.to_vec();
                both.push((mv.to, piece));
                visit(&both);
            });
        }
    };
    if let Some(mid) = mv.mid {
        material::for_each_restoration(missing, target.side_to_move(), |piece, remaining| {
            destination(&[(mid, piece)], remaining);
        });
    } else {
        destination(&[], missing);
    }
}

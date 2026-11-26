//! 探索の深さごとに、ネットと対応する累算器を保持する。

use super::{LayerStackNetwork, Network, SimpleNetwork, layer_stack::LayerStackAccumulator};
use crate::eval::nnue::{
    input::HalfKpVm,
    stats,
    transformer::{FinnyTable, FtAccumulator},
};
use crate::search::MAX_PLY;
use minase_core::{Color, Position, Square, position::Undo};

pub(crate) enum SearchEvaluator<'a> {
    LayerStack {
        net: &'a LayerStackNetwork,
        accumulators: Vec<LayerStackAccumulator>,
    },
    Simple {
        net: &'a SimpleNetwork,
        accumulators: SimpleAccumulators,
    },
}

/// Simpleの深さごとの累算器と、それらが文脈の変更時に使う探索ごとの表。
#[cfg_attr(test, derive(Clone))]
pub(crate) struct SimpleAccumulators {
    plies: Vec<FtAccumulator<HalfKpVm>>,
    finny: FinnyTable,
}

impl std::ops::Index<usize> for SimpleAccumulators {
    type Output = FtAccumulator<HalfKpVm>;

    /// 指定の深さの累算器を返す。
    fn index(&self, ply: usize) -> &Self::Output {
        &self.plies[ply]
    }
}

#[cfg(test)]
impl std::ops::IndexMut<usize> for SimpleAccumulators {
    /// 指定の深さの累算器を書き換える。探索の表は変更しない。
    fn index_mut(&mut self, ply: usize) -> &mut Self::Output {
        &mut self.plies[ply]
    }
}

impl<'a> SearchEvaluator<'a> {
    pub(crate) fn new(network: &'a Network, root: &Position) -> Self {
        stats::attach(network.accumulator_stats());
        stats::record(true);
        match network {
            Network::LayerStack(net) => Self::LayerStack {
                net,
                accumulators: std::iter::once(net.refresh(root))
                    .chain((0..MAX_PLY).map(|_| net.new_accumulator()))
                    .collect(),
            },
            Network::Simple(net) => {
                let accumulator = net.refresh(root);
                let mut finny = FinnyTable::new(net.width());
                finny.save_root(root, &accumulator);
                Self::Simple {
                    net,
                    accumulators: SimpleAccumulators {
                        plies: std::iter::once(accumulator)
                            .chain((0..MAX_PLY).map(|_| net.new_accumulator()))
                            .collect(),
                        finny,
                    },
                }
            }
        }
    }
    pub(crate) fn update_after_move(&mut self, ply: u32, position: &Position, undo: &Undo) {
        let index = ply as usize;
        let refreshed = match self {
            Self::LayerStack { net, accumulators } => {
                let (parents, children) = accumulators.split_at_mut(index);
                net.update_after_move(&parents[index - 1], &mut children[0], position, undo)
            }
            Self::Simple { net, accumulators } => {
                let (parents, children) = accumulators.plies.split_at_mut(index);
                net.update_after_move_cached(
                    &parents[index - 1],
                    &mut children[0],
                    position,
                    undo,
                    &mut accumulators.finny,
                )
            }
        };
        stats::record(refreshed);
    }
    pub(crate) fn update_after_null(&mut self, ply: u32, lion_before: Option<Square>) {
        let index = ply as usize;
        match self {
            Self::LayerStack { net, accumulators } => {
                let (parents, children) = accumulators.split_at_mut(index);
                net.update_after_null(&parents[index - 1], &mut children[0], lion_before);
            }
            Self::Simple { net, accumulators } => {
                let (parents, children) = accumulators.plies.split_at_mut(index);
                net.update_after_null(&parents[index - 1], &mut children[0], lion_before);
            }
        }
        stats::record(false);
    }
    pub(crate) fn evaluate(&self, ply: u32, side: Color) -> i32 {
        match self {
            Self::LayerStack { net, accumulators } => {
                net.evaluate_accumulator(&accumulators[ply as usize], side)
            }
            Self::Simple { net, accumulators } => {
                net.evaluate_accumulator(&accumulators[ply as usize], side)
            }
        }
    }
    #[cfg(feature = "invariants")]
    pub(crate) fn assert_accumulator(&self, position: &Position, ply: u32) {
        fn check<T: PartialEq + std::fmt::Debug>(
            incremental: &T,
            recomputed: T,
            position: &Position,
            ply: u32,
        ) {
            use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
            use std::fmt::Write;
            if *incremental == recomputed {
                return;
            }
            let mut diagnostic = format!(
                "NNUE accumulator mismatch: zobrist={:#018x}, ply={ply}\nincremental: {incremental:?}\nrecomputed: {recomputed:?}",
                position.zobrist()
            );
            if let Ok(setup) = SetupPosition::new(
                position.clone(),
                position.lion_taken_by_non_lion().map(|t| t.square),
                1,
            ) {
                write!(diagnostic, "\nextended SFEN: {}", to_extended_sfen(&setup))
                    .expect("writing to a String cannot fail");
            }
            panic!("{diagnostic}");
        }
        match self {
            Self::LayerStack { net, accumulators } => check(
                &accumulators[ply as usize],
                net.refresh(position),
                position,
                ply,
            ),
            Self::Simple { net, accumulators } => check(
                &accumulators[ply as usize],
                net.refresh(position),
                position,
                ply,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::nnue::network::test_util::{fixed_layer_stack, fixed_simple};
    use minase_core::{
        Move, MoveGenerator, MoveRules, PieceCode, PieceKind,
        rng::XorShift64,
        test_util::{position_from_codes, sq},
    };
    use std::num::NonZeroU64;

    #[test]
    fn random_games_and_null_moves_match_full_evaluation() {
        let code = |color, kind| PieceCode::new(color, kind).unwrap();
        let sparse_root = position_from_codes(
            Color::Black,
            &[
                (sq(0, 0), code(Color::Black, PieceKind::King)),
                (sq(11, 11), code(Color::White, PieceKind::King)),
                (sq(4, 1), code(Color::Black, PieceKind::Pawn)),
                (sq(7, 10), code(Color::White, PieceKind::Pawn)),
            ],
        );
        for network in [
            Network::LayerStack(fixed_layer_stack()),
            Network::Simple(fixed_simple()),
        ] {
            for (seed, mut position, favor_kings) in [
                (0x1234, Position::initial(), false),
                (0x5678, Position::initial(), false),
                (0x9abc, Position::initial(), false),
                (0xdef0, Position::initial(), false),
                (0xabcd, sparse_root.clone(), true),
            ] {
                let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
                network.check_search_root(&position).unwrap();
                let mut evaluator = SearchEvaluator::new(&network, &position);
                let mut king_moves = 0;
                for ply in 1..=128.min(MAX_PLY) {
                    assert_eq!(
                        evaluator.evaluate(ply - 1, position.side_to_move()),
                        network.evaluate(&position).unwrap()
                    );
                    if ply % 7 == 0 {
                        let lion = position.lion_taken_by_non_lion().map(|t| t.square);
                        position.make_null_move();
                        evaluator.update_after_null(ply, lion);
                    } else {
                        let mut moves = Vec::new();
                        MoveGenerator::standard().generate_moves(&position, &mut moves);
                        if moves.is_empty() {
                            break;
                        }
                        if favor_kings && ply % 3 != 0 {
                            let kings =
                                position.pieces_of_kind(position.side_to_move(), PieceKind::King);
                            if moves.iter().any(|mv| kings.contains(mv.from)) {
                                moves.retain(|mv| kings.contains(mv.from));
                                king_moves += 1;
                            }
                        }
                        let undo = position.make_move_unchecked(
                            moves[rng.next() as usize % moves.len()],
                            MoveRules::standard(),
                        );
                        evaluator.update_after_move(ply, &position, &undo);
                    }
                    // 最後の王駒の捕獲は探索が詰みとして扱い、静的評価を呼ばない。
                    if Color::ALL
                        .into_iter()
                        .any(|side| position.royal_pieces(side).is_empty())
                    {
                        break;
                    }
                    assert_eq!(
                        evaluator.evaluate(ply, position.side_to_move()),
                        network.evaluate(&position).unwrap(),
                        "seed={seed} ply={ply}"
                    );
                    #[cfg(feature = "invariants")]
                    evaluator.assert_accumulator(&position, ply);
                }
                if favor_kings {
                    assert!(king_moves >= 16, "seed={seed}, king_moves={king_moves}");
                }
            }
        }
    }
    #[test]
    fn counts_distinguish_anchor_refresh_moves_and_null_updates() {
        for network in [
            Network::LayerStack(fixed_layer_stack()),
            Network::Simple(fixed_simple()),
        ] {
            let mut position = position_from_codes(
                Color::Black,
                &[
                    (
                        sq(0, 0),
                        PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                    ),
                    (
                        sq(11, 11),
                        PieceCode::new(Color::White, PieceKind::King).unwrap(),
                    ),
                    (
                        sq(5, 5),
                        PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
                    ),
                ],
            );
            let mut evaluator = SearchEvaluator::new(&network, &position);
            assert_eq!(
                network.take_accumulator_stats(),
                crate::eval::nnue::AccumulatorStats {
                    refreshes: 1,
                    incremental_updates: 0
                }
            );
            for (ply, from, to) in [
                (1, sq(5, 5), sq(5, 6)),
                (2, sq(11, 11), sq(10, 11)),
                (3, sq(5, 6), sq(5, 7)),
                (4, sq(10, 11), sq(11, 11)),
            ] {
                let mv = Move {
                    from,
                    to,
                    mid: None,
                    promote: false,
                };
                let mut moves = Vec::new();
                MoveGenerator::standard().generate_moves(&position, &mut moves);
                assert!(moves.contains(&mv));
                if matches!(network, Network::Simple(_)) {
                    assert_eq!(network.quiet_move_delta(&position, mv), 0);
                }
                let undo = position.make_move_unchecked(mv, MoveRules::standard());
                evaluator.update_after_move(ply, &position, &undo);
                let refreshed = ply % 2 == 0 && matches!(network, Network::Simple(_));
                assert_eq!(
                    network.take_accumulator_stats(),
                    crate::eval::nnue::AccumulatorStats {
                        refreshes: u64::from(refreshed),
                        incremental_updates: u64::from(!refreshed)
                    }
                );
            }
            position.make_null_move();
            evaluator.update_after_null(5, None);
            assert_eq!(
                network.take_accumulator_stats(),
                crate::eval::nnue::AccumulatorStats {
                    refreshes: 0,
                    incremental_updates: 1
                }
            );
            assert_eq!(network.take_accumulator_stats().updates(), 0);
            assert_eq!(
                evaluator.evaluate(5, position.side_to_move()),
                network.evaluate(&position).unwrap()
            );
        }
    }
}

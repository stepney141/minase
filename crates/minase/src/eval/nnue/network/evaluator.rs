//! 探索の深さごとに、ネットと対応する累算器を保持する。

use super::{LayerStackNetwork, Network, SimpleNetwork, layer_stack::LayerStackAccumulator};
use crate::eval::nnue::{input::HalfKpVm, stats, transformer::FtAccumulator};
use crate::search::MAX_PLY;
use minase_core::{Color, Position, Square, position::Undo};

pub(crate) enum SearchEvaluator<'a> {
    LayerStack {
        net: &'a LayerStackNetwork,
        accumulators: Vec<LayerStackAccumulator>,
    },
    Simple {
        net: &'a SimpleNetwork,
        accumulators: Vec<FtAccumulator<HalfKpVm>>,
    },
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
            Network::Simple(net) => Self::Simple {
                net,
                accumulators: std::iter::once(net.refresh(root))
                    .chain((0..MAX_PLY).map(|_| net.new_accumulator()))
                    .collect(),
            },
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
                let (parents, children) = accumulators.split_at_mut(index);
                net.update_after_move(&parents[index - 1], &mut children[0], position, undo)
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
                let (parents, children) = accumulators.split_at_mut(index);
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
    use minase_core::{MoveGenerator, MoveRules, rng::XorShift64};
    use std::num::NonZeroU64;

    #[test]
    fn random_games_and_null_moves_match_full_evaluation() {
        for network in [
            Network::LayerStack(fixed_layer_stack()),
            Network::Simple(fixed_simple()),
        ] {
            for seed in [0x1234, 0x5678, 0x9abc, 0xdef0] {
                let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
                let mut position = Position::initial();
                network.check_search_root(&position).unwrap();
                let mut evaluator = SearchEvaluator::new(&network, &position);
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
            }
        }
    }
    #[test]
    fn counts_distinguish_anchor_refresh_moves_and_null_updates() {
        use minase_core::{
            Move, PieceCode, PieceKind,
            test_util::{position_from_codes, sq},
        };
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
            for (ply, from, to) in [(1, sq(5, 5), sq(5, 6)), (2, sq(11, 11), sq(10, 11))] {
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
                let refreshed = ply == 2 && matches!(network, Network::Simple(_));
                assert_eq!(
                    network.take_accumulator_stats(),
                    crate::eval::nnue::AccumulatorStats {
                        refreshes: u64::from(refreshed),
                        incremental_updates: u64::from(!refreshed)
                    }
                );
            }
            position.make_null_move();
            evaluator.update_after_null(3, None);
            assert_eq!(
                network.take_accumulator_stats(),
                crate::eval::nnue::AccumulatorStats {
                    refreshes: 0,
                    incremental_updates: 1
                }
            );
            assert_eq!(network.take_accumulator_stats().updates(), 0);
            assert_eq!(
                evaluator.evaluate(3, position.side_to_move()),
                network.evaluate(&position).unwrap()
            );
        }
    }
}

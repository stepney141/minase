//! 学習PSTの累算値を着手に応じて差分更新する。

use super::features::{active_features_for, feature_index, lion_feature_index};
use super::{Pst, interpolate};
use crate::core::position::Undo;
use crate::{Color, Position, Square};

/// 先手視点と後手視点で集計したPSTの生重み和。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct PstAccumulator {
    /// 視点、端点の順に保持する生重み和。
    pub(super) sums: [[i32; 2]; 2],
    /// 係数へ変換する前の盤上総駒数。
    pub(super) piece_count: u32,
}

impl Pst {
    /// 同「段階6」の隣接した端点重みを読み、両方の累算値へ加える。
    pub(super) fn add_feature(&self, sums: &mut [i32; 2], feature: usize, sign: i32) {
        let pair = self.weights[feature];
        sums[0] += sign * i32::from(pair[0]);
        sums[1] += sign * i32::from(pair[1]);
    }

    /// 局面のPST累算値を両視点から完全再計算する。
    pub(crate) fn refresh_accumulator(&self, position: &Position) -> PstAccumulator {
        let mut accumulator = PstAccumulator {
            sums: [[0; 2]; 2],
            piece_count: position.occupied().popcount(),
        };
        for perspective in Color::ALL {
            active_features_for(perspective, position, |feature| {
                self.add_feature(&mut accumulator.sums[perspective.index()], feature, 1);
            });
        }
        accumulator
    }

    /// 通常着手後の局面について、PST累算値を差分更新する。
    pub(crate) fn update_accumulator_after_move(
        &self,
        before: PstAccumulator,
        position_after: &Position,
        undo: &Undo,
    ) -> PstAccumulator {
        let mut after = before;
        let moved_piece_after = position_after
            .piece_at(undo.mv.to)
            .expect("move destination must contain the moved piece");

        after.piece_count -= undo.captured.iter().flatten().count() as u32;
        for perspective in Color::ALL {
            let sums = &mut after.sums[perspective.index()];
            self.add_feature(
                sums,
                feature_index(perspective, undo.moved_piece_before, undo.mv.from),
                -1,
            );
            for captured in undo.captured.into_iter().flatten() {
                self.add_feature(
                    sums,
                    feature_index(perspective, captured.piece, captured.square),
                    -1,
                );
            }
            if let Some(trigger) = undo.previous_lion_taken {
                self.add_feature(sums, lion_feature_index(perspective, trigger.square), -1);
            }
            self.add_feature(
                sums,
                feature_index(perspective, moved_piece_after, undo.mv.to),
                1,
            );
            if let Some(trigger) = position_after.lion_taken_by_non_lion() {
                self.add_feature(sums, lion_feature_index(perspective, trigger.square), 1);
            }
        }
        after
    }

    /// null move後の局面について、PST累算値から直前の先獅子特徴を除く。
    pub(crate) fn update_accumulator_after_null(
        &self,
        before: PstAccumulator,
        lion_before: Option<Square>,
    ) -> PstAccumulator {
        let mut after = before;
        if let Some(square) = lion_before {
            for perspective in Color::ALL {
                self.add_feature(
                    &mut after.sums[perspective.index()],
                    lion_feature_index(perspective, square),
                    -1,
                );
            }
        }
        after
    }

    /// 指定手番の視点からPST累算値をセンチポーン評価へ変換する。
    pub(crate) fn evaluate_accumulator(
        &self,
        accumulator: PstAccumulator,
        side_to_move: Color,
    ) -> i32 {
        interpolate(
            accumulator.sums[side_to_move.index()],
            accumulator.piece_count,
        )
    }
}

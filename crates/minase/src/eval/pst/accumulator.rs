//! 学習PSTの累算値を着手に応じて差分更新する。

use super::features::{active_features_for, feature_index, lion_feature_index};
use super::fm::FmAccumulator;
use super::{Pst, add_correction, interpolate};
use minase_core::position::Undo;
use minase_core::{Color, Position, Square};

/// 先手視点と後手視点で集計したPSTの生重み和。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct PstAccumulator {
    /// 視点、端点の順に保持する生重み和。
    pub(super) sums: [[i32; 2]; 2],
    /// 係数へ変換する前の盤上総駒数。
    pub(super) piece_count: u32,
    /// 両視点のFM累算値。
    fm: [FmAccumulator; 2],
}

impl Pst {
    /// 同「段階6」の隣接した端点重みを読み、両方の累算値へ加える。
    pub(super) fn add_feature(&self, sums: &mut [i32; 2], feature: usize, sign: i32) {
        let pair = self.weights[feature];
        sums[0] += sign * i32::from(pair[0]);
        sums[1] += sign * i32::from(pair[1]);
    }

    /// PSTとFMに同じ特徴を加える。
    fn add_accumulator_feature(
        &self,
        accumulator: &mut PstAccumulator,
        perspective: Color,
        feature: usize,
    ) {
        self.add_feature(&mut accumulator.sums[perspective.index()], feature, 1);
        if let Some(fm) = &self.fm {
            fm.add(&mut accumulator.fm[perspective.index()], feature);
        }
    }

    /// PSTとFMから同じ特徴を除く。
    fn remove_feature(&self, accumulator: &mut PstAccumulator, perspective: Color, feature: usize) {
        self.add_feature(&mut accumulator.sums[perspective.index()], feature, -1);
        if let Some(fm) = &self.fm {
            fm.remove(&mut accumulator.fm[perspective.index()], feature);
        }
    }

    /// 局面のPSTとFMの累算値を両視点から完全再計算する。
    pub(crate) fn refresh_accumulator(&self, position: &Position) -> PstAccumulator {
        let mut accumulator = PstAccumulator {
            piece_count: position.occupied().popcount(),
            ..PstAccumulator::default()
        };
        for perspective in Color::ALL {
            active_features_for(perspective, position, |feature| {
                self.add_accumulator_feature(&mut accumulator, perspective, feature);
            });
        }
        accumulator
    }

    /// 親の累算値を子の格納先へ複写し、通常着手の差分を反映する。
    pub(crate) fn update_accumulator_after_move(
        &self,
        before: &PstAccumulator,
        after: &mut PstAccumulator,
        position_after: &Position,
        undo: &Undo,
    ) {
        let moved_piece_after = position_after
            .piece_at(undo.mv.to)
            .expect("move destination must contain the moved piece");

        after.piece_count = before.piece_count - undo.captured.iter().flatten().count() as u32;
        for perspective in Color::ALL {
            let removed = feature_index(perspective, undo.moved_piece_before, undo.mv.from);
            let added = feature_index(perspective, moved_piece_after, undo.mv.to);
            let index = perspective.index();
            for endpoint in 0..2 {
                after.sums[index][endpoint] = before.sums[index][endpoint]
                    - i32::from(self.weights[removed][endpoint])
                    + i32::from(self.weights[added][endpoint]);
            }
            if let Some(fm) = &self.fm {
                fm.replace(&before.fm[index], &mut after.fm[index], removed, added);
            }
            for captured in undo.captured.into_iter().flatten() {
                self.remove_feature(
                    after,
                    perspective,
                    feature_index(perspective, captured.piece, captured.square),
                );
            }
            if let Some(trigger) = undo.previous_lion_taken {
                self.remove_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, trigger.square),
                );
            }
            if let Some(trigger) = position_after.lion_taken_by_non_lion() {
                self.add_accumulator_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, trigger.square),
                );
            }
        }
    }

    /// 親の累算値を子の格納先へ複写し、null moveで消える先獅子特徴を除く。
    pub(crate) fn update_accumulator_after_null(
        &self,
        before: &PstAccumulator,
        after: &mut PstAccumulator,
        lion_before: Option<Square>,
    ) {
        *after = *before;
        if let Some(square) = lion_before {
            for perspective in Color::ALL {
                self.remove_feature(after, perspective, lion_feature_index(perspective, square));
            }
        }
    }

    /// 指定手番の視点から累算値をFM込みのセンチポーン評価へ変換する。
    pub(crate) fn evaluate_accumulator(
        &self,
        accumulator: &PstAccumulator,
        side_to_move: Color,
    ) -> i32 {
        let baseline = interpolate(
            accumulator.sums[side_to_move.index()],
            accumulator.piece_count,
        );
        match &self.fm {
            Some(fm) => add_correction(
                baseline,
                fm.correction(&accumulator.fm[side_to_move.index()]),
            ),
            None => baseline,
        }
    }
}

#[cfg(feature = "invariants")]
impl Pst {
    /// 探索の差分累算値を両視点とも全再計算と照合する。
    pub(crate) fn assert_accumulator(
        &self,
        position: &Position,
        incremental: &PstAccumulator,
        ply: u32,
    ) {
        use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
        use std::fmt::Write;

        let recomputed = self.refresh_accumulator(position);
        if *incremental == recomputed {
            return;
        }
        let mut diagnostic = format!(
            "PST accumulator mismatch: zobrist={:#018x}, ply={ply}\nincremental: {incremental:?}\nrecomputed: {recomputed:?}",
            position.zobrist(),
        );
        if let Ok(setup) = SetupPosition::new(
            position.clone(),
            position
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square),
            1,
        ) {
            write!(diagnostic, "\nextended SFEN: {}", to_extended_sfen(&setup))
                .expect("writing to a String cannot fail");
        }
        panic!("{diagnostic}");
    }
}

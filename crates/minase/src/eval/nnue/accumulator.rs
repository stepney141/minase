//! 両視点の線形・非線形累算値を着手に応じて差分更新する。

use super::{EVALUATION_LIMIT, Nnue};
use crate::eval::features::{active_features_for, feature_index, lion_feature_index};
use minase_core::position::Undo;
use minase_core::{Color, Position, Square};

/// 両視点の線形2成分、非線形H成分および盤上総駒数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct NnueAccumulator {
    pub(super) sums: [[i32; 2]; 2],
    hidden: [Box<[i32]>; 2],
    pub(super) piece_count: u32,
}

impl NnueAccumulator {
    /// 指定幅の格納領域を確保する。探索では各plyにつき生成時だけ呼ぶ。
    pub(crate) fn new(width: usize) -> Self {
        Self {
            sums: [[0; 2]; 2],
            hidden: std::array::from_fn(|_| vec![0; width].into_boxed_slice()),
            piece_count: 0,
        }
    }

    /// 確保済みの領域へ親の値を複写する。
    fn copy_from(&mut self, before: &Self) {
        self.sums = before.sums;
        self.piece_count = before.piece_count;
        for perspective in 0..2 {
            self.hidden[perspective].copy_from_slice(&before.hidden[perspective]);
        }
    }
}

impl Nnue {
    fn add_feature(
        &self,
        accumulator: &mut NnueAccumulator,
        perspective: Color,
        feature: usize,
        sign: i32,
    ) {
        let index = perspective.index();
        for endpoint in 0..2 {
            accumulator.sums[index][endpoint] += sign * i32::from(self.linear[feature][endpoint]);
        }
        for (value, &weight) in accumulator.hidden[index]
            .iter_mut()
            .zip(&self.weights[feature * self.width..(feature + 1) * self.width])
        {
            *value += sign * i32::from(weight);
        }
    }

    /// 両視点の累算値を完全再計算する。
    pub(crate) fn refresh_accumulator(&self, position: &Position) -> NnueAccumulator {
        let mut accumulator = NnueAccumulator::new(self.width);
        accumulator.piece_count = position.occupied().popcount();
        for perspective in Color::ALL {
            accumulator.hidden[perspective.index()].copy_from_slice(&self.biases);
            active_features_for(perspective, position, |feature| {
                self.add_feature(&mut accumulator, perspective, feature, 1)
            });
        }
        accumulator
    }

    /// 親の累算値を子へ複写し、着手、捕獲、成りおよび先獅子の差分を反映する。
    pub(crate) fn update_accumulator_after_move(
        &self,
        before: &NnueAccumulator,
        after: &mut NnueAccumulator,
        position_after: &Position,
        undo: &Undo,
    ) {
        let moved = position_after
            .piece_at(undo.mv.to)
            .expect("move destination contains moved piece");
        after.copy_from(before);
        after.piece_count -= undo.captured.iter().flatten().count() as u32;
        for perspective in Color::ALL {
            self.add_feature(
                after,
                perspective,
                feature_index(perspective, undo.moved_piece_before, undo.mv.from),
                -1,
            );
            self.add_feature(
                after,
                perspective,
                feature_index(perspective, moved, undo.mv.to),
                1,
            );
            for captured in undo.captured.into_iter().flatten() {
                self.add_feature(
                    after,
                    perspective,
                    feature_index(perspective, captured.piece, captured.square),
                    -1,
                );
            }
            if let Some(trigger) = undo.previous_lion_taken {
                self.add_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, trigger.square),
                    -1,
                );
            }
            if let Some(trigger) = position_after.lion_taken_by_non_lion() {
                self.add_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, trigger.square),
                    1,
                );
            }
        }
    }

    /// 親の累算値を子へ複写し、null moveで消える先獅子特徴を除く。
    pub(crate) fn update_accumulator_after_null(
        &self,
        before: &NnueAccumulator,
        after: &mut NnueAccumulator,
        lion_before: Option<Square>,
    ) {
        after.copy_from(before);
        if let Some(square) = lion_before {
            for perspective in Color::ALL {
                self.add_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, square),
                    -1,
                );
            }
        }
    }

    /// SCReLUと両視点の直接内積で非線形成分をセンチポーンへ換算する。
    pub(super) fn nonlinear_cp(&self, accumulator: &NnueAccumulator, side: Color) -> i64 {
        let mut out = self.output_bias;
        for (p, perspective) in [side, side.opposite()].into_iter().enumerate() {
            for (&value, &weight) in accumulator.hidden[perspective.index()]
                .iter()
                .zip(&self.output_weights[p * self.width..(p + 1) * self.width])
            {
                let c = value.clamp(0, 127);
                let a = (c * c) >> 7;
                out += i32::from(weight) * a;
            }
        }
        (i64::from(out) * self.scale + 32_768) >> 16
    }

    /// 線形補間と非線形出力を合算し、最後に1回だけ静的評価の範囲へ制限する。
    pub(crate) fn evaluate_accumulator(
        &self,
        accumulator: &NnueAccumulator,
        side_to_move: Color,
    ) -> i32 {
        let q = i64::from(accumulator.piece_count.saturating_sub(2)).min(90);
        let [mg, eg] = accumulator.sums[side_to_move.index()].map(i64::from);
        let linear = (q * mg + (90 - q) * eg) / 720;
        (linear + self.nonlinear_cp(accumulator, side_to_move))
            .clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
    }
}

#[cfg(feature = "invariants")]
impl Nnue {
    /// 探索の差分累算値を両視点とも全再計算と照合する。
    pub(crate) fn assert_accumulator(
        &self,
        position: &Position,
        incremental: &NnueAccumulator,
        ply: u32,
    ) {
        use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
        use std::fmt::Write;

        let recomputed = self.refresh_accumulator(position);
        if *incremental == recomputed {
            return;
        }
        let mut diagnostic = format!(
            "NNUE accumulator mismatch: zobrist={:#018x}, ply={ply}\nincremental: {incremental:?}\nrecomputed: {recomputed:?}",
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

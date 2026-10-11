//! 両視点の線形・非線形累算値を着手に応じて差分更新する。

use super::feature_set::FeatureSet;
use super::head::Head;
use super::{ActiveFeatureSet, EVALUATION_LIMIT, Network};
use crate::eval::features::{active_features_for, feature_index, lion_feature_index};
use minase_core::position::Undo;
use minase_core::{Color, Position, Square};

/// 両視点の線形2成分、非線形H成分および盤上総駒数。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Accumulator<F: FeatureSet> {
    pub(super) contexts: [F::Context; 2],
    pub(super) sums: [[i32; 2]; 2],
    hidden: [Box<[i32]>; 2],
    pub(super) piece_count: u32,
}

/// 探索が使う入力特徴集合の累算器。
pub(crate) type NnueAccumulator = Accumulator<ActiveFeatureSet>;

impl<F: FeatureSet> Accumulator<F> {
    /// 指定幅の格納領域を確保する。探索では各plyにつき生成時だけ呼ぶ。
    pub(crate) fn new(width: usize) -> Self {
        Self {
            sums: [[0; 2]; 2],
            contexts: [F::Context::default(); 2],
            hidden: std::array::from_fn(|_| vec![0; width].into_boxed_slice()),
            piece_count: 0,
        }
    }

    /// 確保済みの領域へ親の値を複写する。
    fn copy_from(&mut self, before: &Self) {
        self.contexts = before.contexts;
        self.sums = before.sums;
        self.piece_count = before.piece_count;
        for perspective in 0..2 {
            self.hidden[perspective].copy_from_slice(&before.hidden[perspective]);
        }
    }
}

impl<F: FeatureSet, H: Head> Network<F, H> {
    fn add_feature(
        &self,
        accumulator: &mut Accumulator<F>,
        perspective: Color,
        feature: usize,
        sign: i32,
        update_hidden: bool,
    ) {
        let index = perspective.index();
        for endpoint in 0..2 {
            accumulator.sums[index][endpoint] += sign * i32::from(self.linear[feature][endpoint]);
        }
        if update_hidden {
            self.add_hidden(accumulator, perspective, feature, sign);
        }
    }

    fn add_hidden(
        &self,
        accumulator: &mut Accumulator<F>,
        perspective: Color,
        base: usize,
        sign: i32,
    ) {
        let index = perspective.index();
        let feature = F::index(accumulator.contexts[index], base);
        for (value, &weight) in accumulator.hidden[index]
            .iter_mut()
            .zip(&self.weights[feature * self.width..(feature + 1) * self.width])
        {
            *value += sign * i32::from(weight);
        }
    }

    /// 両視点の累算値を完全再計算する。
    pub(crate) fn refresh_accumulator(&self, position: &Position) -> Accumulator<F> {
        let mut accumulator = Accumulator::new(self.width);
        accumulator.piece_count = position.occupied().popcount();
        for perspective in Color::ALL {
            accumulator.contexts[perspective.index()] = F::context(position, perspective);
            accumulator.hidden[perspective.index()].copy_from_slice(&self.biases);
            active_features_for(perspective, position, |feature| {
                self.add_feature(&mut accumulator, perspective, feature, 1, true)
            });
        }
        accumulator
    }

    /// 親の累算値を子へ複写し、着手、捕獲、成りおよび先獅子の差分を反映する。
    pub(crate) fn update_accumulator_after_move(
        &self,
        before: &Accumulator<F>,
        after: &mut Accumulator<F>,
        position_after: &Position,
        undo: &Undo,
    ) {
        let moved = position_after
            .piece_at(undo.mv.to)
            .expect("move destination contains moved piece");
        after.copy_from(before);
        after.piece_count -= undo.captured.iter().flatten().count() as u32;
        for perspective in Color::ALL {
            let index = perspective.index();
            after.contexts[index] =
                F::context_after_move(before.contexts[index], position_after, perspective, undo);
            let same_context = after.contexts[index] == before.contexts[index];
            self.add_feature(
                after,
                perspective,
                feature_index(perspective, undo.moved_piece_before, undo.mv.from),
                -1,
                same_context,
            );
            self.add_feature(
                after,
                perspective,
                feature_index(perspective, moved, undo.mv.to),
                1,
                same_context,
            );
            for captured in undo.captured.into_iter().flatten() {
                self.add_feature(
                    after,
                    perspective,
                    feature_index(perspective, captured.piece, captured.square),
                    -1,
                    same_context,
                );
            }
            if let Some(trigger) = undo.previous_lion_taken {
                self.add_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, trigger.square),
                    -1,
                    same_context,
                );
            }
            if let Some(trigger) = position_after.lion_taken_by_non_lion() {
                self.add_feature(
                    after,
                    perspective,
                    lion_feature_index(perspective, trigger.square),
                    1,
                    same_context,
                );
            }
            if !same_context {
                after.hidden[index].copy_from_slice(&self.biases);
                active_features_for(perspective, position_after, |feature| {
                    self.add_hidden(after, perspective, feature, 1);
                });
            }
        }
    }

    /// 親の累算値を子へ複写し、null moveで消える先獅子特徴を除く。
    pub(crate) fn update_accumulator_after_null(
        &self,
        before: &Accumulator<F>,
        after: &mut Accumulator<F>,
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
                    true,
                );
            }
        }
    }

    /// SCReLUと両視点の直接内積で非線形成分をセンチポーンへ換算する。
    pub(super) fn nonlinear_cp(&self, accumulator: &Accumulator<F>, side: Color) -> i64 {
        let out = self.head.output(
            &accumulator.hidden[side.index()],
            &accumulator.hidden[side.opposite().index()],
        );
        (i64::from(out) * self.scale + 32_768) >> 16
    }

    /// 線形補間と非線形出力を合算し、最後に1回だけ静的評価の範囲へ制限する。
    pub(crate) fn evaluate_accumulator(
        &self,
        accumulator: &Accumulator<F>,
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
impl<F: FeatureSet, H: Head> Network<F, H> {
    /// 探索の差分累算値を両視点とも全再計算と照合する。
    pub(crate) fn assert_accumulator(
        &self,
        position: &Position,
        incremental: &Accumulator<F>,
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

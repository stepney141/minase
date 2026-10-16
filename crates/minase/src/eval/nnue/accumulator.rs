//! 両視点の線形・非線形累算値を着手に応じて差分更新する。

use super::feature_set::FeatureSet;
use super::head::Head;
use super::{ActiveFeatureSet, EVALUATION_LIMIT, Network, stats};
use crate::eval::features::{active_features_for, feature_index, lion_feature_index};
use minase_core::position::Undo;
use minase_core::{Color, Position, Square};

/// 両視点の線形2成分、非線形H成分、および王駒を含む盤上総駒数。
/// 非線形成分はバイアスと発火した行の和であり、絶対値は`146×1,016 = 148,336`以下である。
/// 線形成分は`145×32,768 = 4,751,360`以下であり、どちらもi32に収まる。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Accumulator<F: FeatureSet> {
    /// 第1層の特徴番号を定める、視点ごとの文脈。
    pub(super) contexts: [F::Context; 2],
    pub(super) sums: [[i32; 2]; 2],
    hidden: [Box<[i32]>; 2],
    /// 王駒を含む盤上総駒数。先獅子の特徴は含めない。
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
        stats::attach(&self.accumulator_stats);
        stats::record(true);
        self.recompute_accumulator(position)
    }

    /// 整合性検査からも使う、計数を伴わない再計算。
    fn recompute_accumulator(&self, position: &Position) -> Accumulator<F> {
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

    /// 親の累算値を複写し、移動、1枚または2枚の捕獲、居喰い、じっと、成り、先獅子の発生と消滅を反映する。
    /// 線形の経路は基底特徴上にあるため常に差分更新する。
    /// 非線形の経路は文脈が変わった視点だけバイアスから全再計算する。
    /// 少なくとも1視点を全再計算した着手は[`super::AccumulatorStats::refreshes`]に1回と数える。
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
        let mut refreshed = false;
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
                refreshed = true;
                after.hidden[index].copy_from_slice(&self.biases);
                active_features_for(perspective, position_after, |feature| {
                    self.add_hidden(after, perspective, feature, 1);
                });
            }
        }
        stats::record(refreshed);
    }

    /// 親の累算値を子へ複写し、null moveで消える先獅子特徴を除く。
    pub(crate) fn update_accumulator_after_null(
        &self,
        before: &Accumulator<F>,
        after: &mut Accumulator<F>,
        lion_before: Option<Square>,
    ) {
        stats::record(false);
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

    /// 尺度127×4,096の非線形出力を`(out×scale + 32,768) >> 16`でセンチポーンへ換算する。
    /// i64で計算し、最近接へ丸め、中間値は正の方向へ丸める。
    /// 負数の右シフトは床方向に働く。
    pub(super) fn nonlinear_cp(&self, accumulator: &Accumulator<F>, side: Color, q: i32) -> i64 {
        let out = self.head.output(
            &accumulator.hidden[side.index()],
            &accumulator.hidden[side.opposite().index()],
            q,
        );
        (i64::from(out) * self.scale + 32_768) >> 16
    }

    /// 線形の補間と[`Self::nonlinear_cp`]の出力を合算する。
    /// 線形は`(q×mg + (90−q)×eg)/720`をi64で計算し、0方向へ切り捨てる。
    /// 両者の和を最後に1回だけ±28,999センチポーンへ制限する。
    pub(crate) fn evaluate_accumulator(
        &self,
        accumulator: &Accumulator<F>,
        side_to_move: Color,
    ) -> i32 {
        let q = i64::from(accumulator.piece_count.saturating_sub(2)).min(90);
        let [mg, eg] = accumulator.sums[side_to_move.index()].map(i64::from);
        let linear = (q * mg + (90 - q) * eg) / 720;
        (linear + self.nonlinear_cp(accumulator, side_to_move, q as i32))
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

        let recomputed = self.recompute_accumulator(position);
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

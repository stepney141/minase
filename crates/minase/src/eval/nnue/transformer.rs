//! 入力特徴の行を加算する第1層と、両視点の累算器。

use super::input::{Indexer, for_each_active_row};
use crate::eval::features::{for_each_move_change, for_each_null_change};
use minase_core::position::Undo;
use minase_core::{Color, Position, Square};
use std::{fmt, marker::PhantomData};

/// 行優先の重みとバイアスからなる第1層。
/// 加減算にはi16の剰余演算を使い、途中で範囲を超えても同じ最終値を得る。
/// 最終値がi16に収まることは、後のフェーズの復号が重みの値域で保証する。
pub(crate) struct FeatureTransformer<I: Indexer> {
    width: usize,
    biases: Box<[i16]>,
    weights: Box<[i16]>,
    indexer: PhantomData<I>,
}

/// 第1層を構築できない原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransformerError {
    /// 幅が0または奇数である。
    Width { width: usize },
    /// バイアスの長さが幅と異なる。
    BiasLength { expected: usize, actual: usize },
    /// 重みの長さが行数×幅と異なる。積がusizeに収まらない場合は`expected`が`None`。
    WeightLength {
        expected: Option<usize>,
        actual: usize,
    },
}

impl fmt::Display for TransformerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Width { width } => {
                write!(f, "transformer width must be positive and even: {width}")
            }
            Self::BiasLength { expected, actual } => {
                write!(f, "expected {expected} biases, got {actual}")
            }
            Self::WeightLength {
                expected: Some(expected),
                actual,
            } => write!(f, "expected {expected} weights, got {actual}"),
            Self::WeightLength {
                expected: None,
                actual,
            } => write!(f, "weight count exceeds usize, got {actual}"),
        }
    }
}
impl std::error::Error for TransformerError {}

/// 視点ごとの文脈と第1層の累算値。文脈がない視点の成分は0である。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct FtAccumulator<I: Indexer> {
    contexts: [Option<I::Context>; 2],
    rows: [Box<[i16]>; 2],
}

impl<I: Indexer> FtAccumulator<I> {
    /// 全成分0、文脈なしの領域を確保する。
    pub(crate) fn new(width: usize) -> Self {
        Self {
            contexts: [None; 2],
            rows: std::array::from_fn(|_| vec![0; width].into_boxed_slice()),
        }
    }

    /// 指定視点の累算値を返す。文脈がなければ`None`を返す。
    pub(crate) fn row(&self, perspective: Color) -> Option<&[i16]> {
        self.contexts[perspective.index()].map(|_| self.rows[perspective.index()].as_ref())
    }

    /// 指定視点の文脈を返す。
    pub(crate) fn context(&self, perspective: Color) -> Option<I::Context> {
        self.contexts[perspective.index()]
    }
}

impl<I: Indexer> FeatureTransformer<I> {
    /// 第1層の幅を返す。
    pub(crate) fn width(&self) -> usize {
        self.width
    }

    /// 書き出し用のバイアスを借用する。
    pub(crate) fn biases(&self) -> &[i16] {
        &self.biases
    }

    /// 書き出し用の行優先の重みを借用する。
    pub(crate) fn weights(&self) -> &[i16] {
        &self.weights
    }

    /// 正の偶数の幅、幅と同じバイアス数、行数×幅と同じ重み数を検査する。
    /// 行数×幅は桁あふれを検査し、表現できなければ重みの長さのエラーとする。
    pub(crate) fn new(
        width: usize,
        biases: Box<[i16]>,
        weights: Box<[i16]>,
    ) -> Result<Self, TransformerError> {
        if width == 0 || !width.is_multiple_of(2) {
            return Err(TransformerError::Width { width });
        }
        let expected = I::FEATURE_SET.spec().dimensions().checked_mul(width);
        if expected.is_none() {
            return Err(TransformerError::WeightLength {
                expected,
                actual: weights.len(),
            });
        }
        if biases.len() != width {
            return Err(TransformerError::BiasLength {
                expected: width,
                actual: biases.len(),
            });
        }
        if Some(weights.len()) != expected {
            return Err(TransformerError::WeightLength {
                expected,
                actual: weights.len(),
            });
        }
        Ok(Self {
            width,
            biases,
            weights,
            indexer: PhantomData,
        })
    }

    fn add_row(&self, values: &mut [i16], row: usize, sign: i32) {
        let weights = &self.weights[row * self.width..(row + 1) * self.width];
        for (value, &weight) in values.iter_mut().zip(weights) {
            *value = if sign > 0 {
                value.wrapping_add(weight)
            } else {
                value.wrapping_sub(weight)
            };
        }
    }

    fn refresh_row(
        &self,
        values: &mut [i16],
        context: I::Context,
        perspective: Color,
        position: &Position,
    ) {
        values.copy_from_slice(&self.biases);
        for_each_active_row::<I>(context, perspective, position, |row| {
            self.add_row(values, row, 1)
        });
    }

    /// 両視点を全再計算する。基準の王駒が不正な視点は文脈なし、全成分0にする。
    pub(crate) fn refresh(&self, position: &Position) -> FtAccumulator<I> {
        let mut accumulator = FtAccumulator::new(self.width);
        for perspective in Color::ALL {
            let index = perspective.index();
            let context = I::context(position, perspective).ok();
            accumulator.contexts[index] = context;
            if let Some(context) = context {
                self.refresh_row(&mut accumulator.rows[index], context, perspective, position);
            }
        }
        accumulator
    }

    /// 確保済みの領域へ着手後の値を書く。文脈が変わった視点だけ全再計算する。
    /// 新しい文脈がなければ全成分を0にする。1視点でも全再計算した場合は真を返す。
    pub(crate) fn update_after_move(
        &self,
        before: &FtAccumulator<I>,
        after: &mut FtAccumulator<I>,
        position_after: &Position,
        undo: &Undo,
    ) -> bool {
        let mut refreshed = false;
        for perspective in Color::ALL {
            let index = perspective.index();
            let context = I::context(position_after, perspective).ok();
            after.contexts[index] = context;
            let values = &mut after.rows[index];
            match context {
                Some(context) if before.contexts[index] == Some(context) => {
                    values.copy_from_slice(&before.rows[index]);
                    for_each_move_change(perspective, position_after, undo, |base, sign| {
                        if let Some(row) = I::row(context, base) {
                            self.add_row(values, row, sign);
                        }
                    });
                }
                Some(context) => {
                    self.refresh_row(values, context, perspective, position_after);
                    refreshed = true;
                }
                None => values.fill(0),
            }
        }
        refreshed
    }

    /// 確保済みの領域へ親の値と文脈を写し、null moveで消える先獅子の特徴を引く。
    pub(crate) fn update_after_null(
        &self,
        before: &FtAccumulator<I>,
        after: &mut FtAccumulator<I>,
        lion_before: Option<Square>,
    ) {
        after.contexts = before.contexts;
        for perspective in Color::ALL {
            let index = perspective.index();
            let values = &mut after.rows[index];
            values.copy_from_slice(&before.rows[index]);
            if let Some(context) = before.contexts[index] {
                for_each_null_change(perspective, lion_before, |base, sign| {
                    if let Some(row) = I::row(context, base) {
                        self.add_row(values, row, sign);
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;

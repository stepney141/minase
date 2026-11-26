//! 入力特徴の行を加算する第1層と、両視点の累算器。

use super::input::{HalfKpVm, HalfKpVmContext, Indexer, for_each_active_row};
use crate::eval::features::{
    feature_index, for_each_move_change, for_each_null_change, lion_feature_index,
};
use minase_core::position::Undo;
use minase_core::{BOARD_SQUARE_COUNT, Color, PieceCode, Position, Square};
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
        self.context(perspective)
            .map(|_| self.rows[perspective.index()].as_ref())
    }

    /// 指定視点の文脈を返す。
    pub(crate) fn context(&self, perspective: Color) -> Option<I::Context> {
        self.contexts[perspective.index()]
    }
}

const FINNY_ENTRIES: usize = Color::ALL.len() * BOARD_SQUARE_COUNT;

/// 探索ごとに作るHalfKP_vmの累算値の表。
/// キーは視点と文脈の組で、1視点につき144項目を`2×区分 + 鏡映の有無`で引く。
/// 各項目には幅ぶんのi16の累算値、144升の駒コード、先獅子の対象升、有効印を保存する。
/// 有効印が偽の項目は未使用であり、保存領域の値は読まない。
/// 行の加減算は2^16を法とする剰余演算なので、順序が変わっても全再計算とビット単位で一致する。
#[cfg_attr(test, derive(Clone))]
pub(crate) struct FinnyTable {
    width: usize,
    rows: Box<[i16]>,
    entries: Box<[FinnyEntry]>,
    #[cfg(test)]
    last_refresh: [Option<FinnyRefresh>; 2],
}

/// 1項目の累算値と対応する盤面および先獅子の状態。
#[derive(Clone, Copy)]
struct FinnyEntry {
    board: [PieceCode; BOARD_SQUARE_COUNT],
    lion: Option<Square>,
    valid: bool,
}

impl FinnyEntry {
    const UNUSED: Self = Self {
        board: [PieceCode::EMPTY; BOARD_SQUARE_COUNT],
        lion: None,
        valid: false,
    };

    /// 現在の駒配置と先獅子を保存し、計算済みの累算値に対応付ける。
    fn save_position(&mut self, position: &Position) {
        for (piece, square) in self.board.iter_mut().zip(Square::all()) {
            *piece = position.piece_at(square).unwrap_or(PieceCode::EMPTY);
        }
        self.lion = position.lion_capture_square();
        self.valid = true;
    }
}

/// テストで観測する文脈変更時の表の使用結果。
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FinnyRefresh {
    /// 未使用の項目を全再計算した。
    Miss,
    /// 有効な項目を使い、指定の行数を足し引きした。
    Hit(usize),
}

impl FinnyTable {
    /// 両視点の領域を確保し、全項目を未使用にする。
    pub(crate) fn new(width: usize) -> Self {
        Self {
            width,
            rows: vec![0; FINNY_ENTRIES * width].into_boxed_slice(),
            entries: vec![FinnyEntry::UNUSED; FINNY_ENTRIES].into_boxed_slice(),
            #[cfg(test)]
            last_refresh: [None; 2],
        }
    }

    /// 根の全再計算の結果を、文脈がある視点の項目へ保存する。
    pub(crate) fn save_root(&mut self, position: &Position, accumulator: &FtAccumulator<HalfKpVm>) {
        let mut snapshot = FinnyEntry::UNUSED;
        for perspective in Color::ALL {
            if let Some(context) = accumulator.context(perspective) {
                if !snapshot.valid {
                    snapshot.save_position(position);
                }
                let (entry, values) = self.entry_mut(perspective, context);
                values.copy_from_slice(&accumulator.rows[perspective.index()]);
                *entry = snapshot;
            }
        }
    }

    /// 指定の視点と文脈に対応する盤面と累算値の領域を借用する。
    fn entry_mut(
        &mut self,
        perspective: Color,
        context: HalfKpVmContext,
    ) -> (&mut FinnyEntry, &mut [i16]) {
        let index = perspective.index() * BOARD_SQUARE_COUNT
            + 2 * context.bucket()
            + usize::from(context.mirrored());
        let start = index * self.width;
        (
            &mut self.entries[index],
            &mut self.rows[start..start + self.width],
        )
    }

    /// 未使用なら全再計算し、有効なら保存局面との差だけを足し引きする。
    /// 項目に結果と現在の局面を残し、結果を累算器へ写す。
    fn refresh_row(
        &mut self,
        transformer: &FeatureTransformer<HalfKpVm>,
        dest: &mut [i16],
        context: HalfKpVmContext,
        perspective: Color,
        position: &Position,
    ) {
        let (entry, values) = self.entry_mut(perspective, context);
        #[cfg(test)]
        let refresh;
        if entry.valid {
            #[cfg(test)]
            let mut row_count = 0;
            let mut apply = |base, sign| {
                if let Some(row) = HalfKpVm::row(context, base) {
                    transformer.add_row(values, row, sign);
                    #[cfg(test)]
                    {
                        row_count += 1;
                    }
                }
            };
            for (previous, square) in entry.board.iter_mut().zip(Square::all()) {
                let current = position.piece_at(square).unwrap_or(PieceCode::EMPTY);
                if *previous != current {
                    if !previous.is_empty() {
                        apply(feature_index(perspective, *previous, square), -1);
                    }
                    if !current.is_empty() {
                        apply(feature_index(perspective, current, square), 1);
                    }
                    *previous = current;
                }
            }
            let lion = position.lion_capture_square();
            if entry.lion != lion {
                if let Some(square) = entry.lion {
                    apply(lion_feature_index(perspective, square), -1);
                }
                if let Some(square) = lion {
                    apply(lion_feature_index(perspective, square), 1);
                }
                entry.lion = lion;
            }
            #[cfg(test)]
            {
                refresh = FinnyRefresh::Hit(row_count);
            }
        } else {
            transformer.refresh_row(values, context, perspective, position);
            entry.save_position(position);
            #[cfg(test)]
            {
                refresh = FinnyRefresh::Miss;
            }
        }
        dest.copy_from_slice(values);
        #[cfg(test)]
        {
            self.last_refresh[perspective.index()] = Some(refresh);
        }
    }
}

impl FeatureTransformer<HalfKpVm> {
    /// 着手の差分を更新し、文脈が変わった視点では探索の表を使う。
    /// 表に当たった場合も文脈が変わった更新は全再計算として数える。
    pub(crate) fn update_after_move_cached(
        &self,
        before: &FtAccumulator<HalfKpVm>,
        after: &mut FtAccumulator<HalfKpVm>,
        position_after: &Position,
        undo: &Undo,
        table: &mut FinnyTable,
    ) -> bool {
        #[cfg(test)]
        {
            table.last_refresh = [None; 2];
        }
        self.update_after_move_with_refresh(
            before,
            after,
            position_after,
            undo,
            |dest, context, perspective| {
                table.refresh_row(self, dest, context, perspective, position_after);
            },
        )
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
        self.update_after_move_with_refresh(
            before,
            after,
            position_after,
            undo,
            |values, context, perspective| {
                self.refresh_row(values, context, perspective, position_after);
            },
        )
    }

    /// 新しい文脈があり、親と異なる視点にだけ、指定の再計算処理を使う。
    fn update_after_move_with_refresh(
        &self,
        before: &FtAccumulator<I>,
        after: &mut FtAccumulator<I>,
        position_after: &Position,
        undo: &Undo,
        mut refresh: impl FnMut(&mut [i16], I::Context, Color),
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
                    refresh(values, context, perspective);
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

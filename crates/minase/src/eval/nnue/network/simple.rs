//! HalfKP_vmの第1層と、32→32→1の後段。

use super::*;
use crate::eval::nnue::{
    input::HalfKpVm,
    transformer::{FinnyTable, FtAccumulator},
};
use minase_core::{Square, position::Undo};

/// Simpleの量子化済みパラメータ。配列は詰め物のない行優先である。
/// 全ての不変条件を`new`で検査してからネットに移す。
pub struct SimpleParameters {
    /// 第1層の幅。2〜1,024の偶数。
    pub width: usize,
    /// 評価値へ換算する正の除数。
    pub fv_scale: i32,
    /// 第1層のバイアスH個。
    pub ft_biases: Vec<i16>,
    /// 第1層の重み964,224×H個。
    pub ft_weights: Vec<i16>,
    /// L1のバイアス32個。
    pub l1_biases: Vec<i32>,
    /// L1の重み32×2H個。
    pub l1_weights: Vec<i8>,
    /// L2のバイアス32個。
    pub l2_biases: Vec<i32>,
    /// L2の重み32×32個。
    pub l2_weights: Vec<i8>,
    /// 出力のバイアス。
    pub output_bias: i32,
    /// 出力の重み32個。
    pub output_weights: Vec<i8>,
}

/// 値域を検査済みのHalfKP_vmのSimple。
pub struct SimpleNetwork {
    ft: FeatureTransformer<HalfKpVm>,
    fv_scale: i32,
    dense: Dense,
    pub(super) accumulator_stats: super::super::stats::SharedStats,
}

/// 手番側の評価値と、除算前の整数の出力。
pub(crate) struct SimpleBreakdown {
    /// センチポーンでの評価値。
    pub(crate) value: i32,
    /// 出力の除数。
    pub(crate) fv_scale: i32,
    /// 除算前の整数の出力。
    pub(crate) out: i32,
}

impl SimpleNetwork {
    /// 寸法、区分ごとの累算値の上限、および後段の値域を検査する。
    /// 第1層の重みはVecからBoxへ所有権を移し、複製を作らない。
    pub fn new(p: SimpleParameters) -> Result<Self, FormatError> {
        check_scale(p.fv_scale)?;
        validate_ft(FeatureSet::HalfKpVm, p.width, &p.ft_biases, &p.ft_weights)?;
        let dense = Dense {
            l1_biases: p.l1_biases,
            l1_weights: p.l1_weights,
            l2_biases: p.l2_biases,
            l2_weights: p.l2_weights,
            output_bias: p.output_bias,
            output_weights: p.output_weights,
        };
        dense.validate(2 * p.width, 32, 32)?;
        Ok(Self {
            ft: transformer(p.width, p.ft_biases, p.ft_weights),
            fv_scale: p.fv_scale,
            dense,
            accumulator_stats: Default::default(),
        })
    }
    /// 第1層の幅を返す。
    pub fn width(&self) -> usize {
        self.ft.width()
    }
    /// 評価値へ換算する正の除数を返す。
    pub fn fv_scale(&self) -> i32 {
        self.fv_scale
    }
    /// 第1層を非圧縮のi16で持つtataraのSimple形式で書き出す。
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = header(FeatureSet::HalfKpVm, self.width(), self.fv_scale);
        bytes.extend(ft_hash(FeatureSet::HalfKpVm, self.width()).to_le_bytes());
        for &value in self.ft.biases().iter().chain(self.ft.weights()) {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(fc_hash(FeatureSet::HalfKpVm, self.width()).to_le_bytes());
        self.dense.encode(&mut bytes, 2 * self.width(), 32);
        bytes
    }
    /// tataraのSimple形式を読み、`new`と同じ値域条件を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(bytes);
        let (width, fv_scale) = read_header(&mut r, FeatureSet::HalfKpVm)?;
        check_hash(
            "ft_hash",
            ft_hash(FeatureSet::HalfKpVm, width),
            r.u32("ft_hash")?,
        )?;
        let ft_biases = r.i16s(width, "FT biases")?;
        let ft_weights = r.i16s(
            FeatureSet::HalfKpVm.spec().dimensions() * width,
            "FT weights",
        )?;
        check_hash(
            "fc_hash",
            fc_hash(FeatureSet::HalfKpVm, width),
            r.u32("fc_hash")?,
        )?;
        let dense = Dense::decode(&mut r, 2 * width, 32, 32)?;
        r.finish()?;
        Self::new(SimpleParameters {
            width,
            fv_scale,
            ft_biases,
            ft_weights,
            l1_biases: dense.l1_biases,
            l1_weights: dense.l1_weights,
            l2_biases: dense.l2_biases,
            l2_weights: dense.l2_weights,
            output_bias: dense.output_bias,
            output_weights: dense.output_weights,
        })
    }
    /// 手番側の視点で全再計算する。両視点の王駒の文脈が一意でなければ拒否する。
    pub fn evaluate(&self, position: &Position) -> Result<i32, AnchorError> {
        self.evaluate_for(position, position.side_to_move())
    }
    /// 指定陣営を手番側として全再計算し、両視点の王駒の文脈を検査する。
    pub fn evaluate_for(&self, position: &Position, side: Color) -> Result<i32, AnchorError> {
        for perspective in Color::ALL {
            HalfKpVm::context(position, perspective)?;
        }
        Ok(self.evaluate_accumulator(&self.refresh(position), side))
    }
    /// 探索の差分更新先として、文脈のない空の累算器を確保する。
    pub(crate) fn new_accumulator(&self) -> FtAccumulator<HalfKpVm> {
        FtAccumulator::new(self.width())
    }
    /// 両視点を全再計算する。王駒の文脈が不正な視点は文脈なしとする。
    pub(crate) fn refresh(&self, position: &Position) -> FtAccumulator<HalfKpVm> {
        self.ft.refresh(position)
    }
    /// 表を通さず着手差分を更新し、文脈が変わった視点だけ全再計算する。
    #[cfg(test)]
    pub(crate) fn update_after_move(
        &self,
        before: &FtAccumulator<HalfKpVm>,
        after: &mut FtAccumulator<HalfKpVm>,
        position_after: &Position,
        undo: &Undo,
    ) -> bool {
        self.ft
            .update_after_move(before, after, position_after, undo)
    }
    /// 着手の差分を更新し、王駒の文脈が変わった視点では探索の表を使う。
    pub(crate) fn update_after_move_cached(
        &self,
        before: &FtAccumulator<HalfKpVm>,
        after: &mut FtAccumulator<HalfKpVm>,
        position_after: &Position,
        undo: &Undo,
        table: &mut FinnyTable,
    ) -> bool {
        self.ft
            .update_after_move_cached(before, after, position_after, undo, table)
    }
    /// 親の累算値を写し、null moveで消える先獅子を引く。
    pub(crate) fn update_after_null(
        &self,
        before: &FtAccumulator<HalfKpVm>,
        after: &mut FtAccumulator<HalfKpVm>,
        lion_before: Option<Square>,
    ) {
        self.ft.update_after_null(before, after, lion_before);
    }
    /// 両視点の累算値を0〜127へ制限し、手番側、相手側の順で2H成分に連結する。
    /// L1とL2は積和を6ビット算術右シフトして0〜127へ制限し、最後は出力層の積和を返す。
    fn out(&self, acc: &FtAccumulator<HalfKpVm>, side: Color) -> i32 {
        let mut storage = [0; 2048];
        let input = &mut storage[..2 * self.width()];
        for (part, perspective) in [side, side.opposite()].into_iter().enumerate() {
            let row = acc.row(perspective).expect("search checks check_search_root and scores capture of the last royal as mate without evaluation");
            for (dst, &x) in input[part * self.width()..(part + 1) * self.width()]
                .iter_mut()
                .zip(row)
            {
                *dst = x.clamp(0, 127) as u8;
            }
        }
        let d = &self.dense;
        let columns = 2 * self.width();
        let h1: [u8; 32] = std::array::from_fn(|j| {
            activation(affine(
                d.l1_biases[j],
                &d.l1_weights[j * columns..(j + 1) * columns],
                input,
            ))
        });
        let h2: [u8; 32] = std::array::from_fn(|k| {
            activation(affine(
                d.l2_biases[k],
                &d.l2_weights[k * 32..(k + 1) * 32],
                &h1,
            ))
        });
        affine(d.output_bias, &d.output_weights, &h2)
    }
    /// 出力を`fv_scale`で0方向に除算し、±28,999へ制限する。
    /// 探索は根を検査し、最後の王駒の捕獲を評価せず詰みとして扱うため、両視点の文脈を必須とする。
    pub(crate) fn evaluate_accumulator(
        &self,
        acc: &FtAccumulator<HalfKpVm>,
        side_to_move: Color,
    ) -> i32 {
        score(i64::from(self.out(acc, side_to_move)), self.fv_scale)
    }
    /// 手番側の評価値と除算前の出力を返す。王駒の文脈が不正なら拒否する。
    pub(crate) fn breakdown(&self, position: &Position) -> Result<SimpleBreakdown, AnchorError> {
        for side in Color::ALL {
            HalfKpVm::context(position, side)?;
        }
        let out = self.out(&self.refresh(position), position.side_to_move());
        Ok(SimpleBreakdown {
            value: score(i64::from(out), self.fv_scale),
            fv_scale: self.fv_scale,
            out,
        })
    }
}

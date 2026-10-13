//! 同時学習の初期NPZを読み、PSTとFMを復元する。
//! `--pst-init`の端点配列は[`crate::pst::encode_pst_npz`]、`--fm-init`の3配列は[`crate::fm::npz`]に定める。
//! [`super::export`]が書く5配列のNPZも、必要な配列を含む入力として使える。
//! 端点は`<f4`、FMの`V`と`a`は`<f4`または`<f8`、`mask`は`|b1`の0または1だけを受け付ける。

use super::{Gradients, Result};
use crate::{fm, nn::npz, pst};
use minase::eval::{
    features::FEATURE_COUNT,
    pst::{FM_RANK, Pst},
};
use std::{collections::BTreeMap, path::Path};

/// 実数PSTとFM。両者は同じ出力用Kを使う。
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// 鏡映共有の2端点。
    pub pst: pst::Model,
    /// 全特徴の埋め込み、係数、観測マスク。
    pub fm: fm::Model,
}
impl Model {
    /// 有限性、マスク、埋め草と両モデルのKの一致を検査する。
    pub fn validate(&self) -> Result<()> {
        self.pst.validate()?;
        self.fm.validate()?;
        if self.pst.k().to_bits() != self.fm.k().to_bits() {
            return Err("joint PST and FM K differ".into());
        }
        Ok(())
    }
}

pub(super) fn read_f32(
    arrays: &mut BTreeMap<String, npz::Array>,
    key: &str,
    shape: &[usize],
) -> Result<Vec<f32>> {
    let array = arrays
        .remove(key)
        .ok_or_else(|| format!("missing joint NPZ {key}"))?;
    if array.dtype != "<f4" || array.shape != shape {
        return Err(format!("joint NPZ {key} dtype or shape").into());
    }
    let values: Vec<_> = array
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| f32::from_le_bytes(b))
        .collect();
    if values.iter().any(|v| !v.is_finite()) {
        return Err(format!("non-finite joint NPZ {key}").into());
    }
    Ok(values)
}
pub(super) fn read_mask(arrays: &mut BTreeMap<String, npz::Array>) -> Result<Vec<bool>> {
    let array = arrays.remove("mask").ok_or("missing joint NPZ mask")?;
    if array.dtype != "|b1" || array.shape != [FEATURE_COUNT] || array.data.iter().any(|&b| b > 1) {
        return Err("joint NPZ mask dtype, shape or value".into());
    }
    Ok(array.data.iter().map(|&b| b != 0).collect())
}

/// MNPT版3の基準と実数の初期値を照合し、鏡映共有と訓練集合のマスクを適用する。
/// `--base`と`--fm-reference`はともに次元32のFMを要求する。
/// `--pst-init`の量子化結果は基準のPSTに、`--fm-init`を基準のKで量子化した結果は参照FMに完全一致しなければならない。
/// PSTの鏡映対は平均し、FMは数え直したマスクで未観測の行を0に戻す。
/// 戻り値の件数は入力NPZのマスクとの不一致数である。
/// aを縮小せず、補正1/4の変換や推論時の倍率は加えない。
pub fn initialize(
    base: &Pst,
    pst_path: &Path,
    fm_path: &Path,
    reference: &Pst,
    mask: Vec<bool>,
) -> Result<(Model, usize)> {
    if base.fm_weights().is_none() || reference.fm_weights().is_none() {
        return Err("--base and --fm-reference must contain FM weights (rank 32)".into());
    }
    let mut arrays = npz::decode(&std::fs::read(pst_path)?)?;
    let mg = read_f32(&mut arrays, "middlegame", &[FEATURE_COUNT])?;
    let eg = read_f32(&mut arrays, "endgame", &[FEATURE_COUNT])?;
    let endpoints: Vec<_> = mg.into_iter().zip(eg).map(|(m, e)| [m, e]).collect();
    let quantized = pst::export::quantize_weights(
        &endpoints
            .iter()
            .map(|r| r.map(f64::from))
            .collect::<Vec<_>>(),
    )?;
    if quantized != base.weights().as_slice() {
        return Err("quantized initial PST disagrees with --base".into());
    }
    let mut arrays = npz::decode(&std::fs::read(fm_path)?)?;
    let v = fm::npz::array(&mut arrays, "V", &[FEATURE_COUNT, FM_RANK])?;
    let a: [f64; FM_RANK] = fm::npz::array(&mut arrays, "a", &[FM_RANK])?
        .try_into()
        .expect("checked rank");
    let initial_mask = read_mask(&mut arrays)?;
    let v = v.as_chunks::<FM_RANK>().0.to_vec();
    let quantized = fm::quantize(&v, &a, f64::from(base.k()))?;
    if Some(quantized.as_weights()) != reference.fm_weights() {
        return Err("quantized initial FM disagrees with --fm-reference".into());
    }
    if mask.len() != FEATURE_COUNT {
        return Err("joint observation mask shape".into());
    }
    let mismatches = mask
        .iter()
        .zip(&initial_mask)
        .filter(|(a, b)| a != b)
        .count();
    let mut v: Vec<_> = v.iter().map(|r| r.map(|v| v as f32)).collect();
    let a = a.map(|v| v as f32);
    if v.iter().flatten().chain(&a).any(|v| !v.is_finite()) {
        return Err("joint FM weights overflow f32".into());
    }
    for (row, &observed) in v.iter_mut().zip(&mask) {
        if !observed {
            row.fill(0.0);
        }
    }
    let model = Model {
        pst: pst::Model::from_endpoints(&endpoints, base.k())?,
        fm: fm::Model::from_weights(&v, a, mask, base.k())?,
    };
    model.validate()?;
    Ok((model, mismatches))
}

/// PST群とFM群のAdamWであり、両群の更新回数の一致を要求する。
/// 既定のPST群は学習率0.03、重み減衰0で、更新後に保存範囲へ切り詰める。
/// 既定のFM群は学習率0.001、重み減衰0.0001で、更新後に未観測の行を0へ戻す。
/// 各群は更新ごとに全要素を処理する。
#[derive(Clone, Debug, PartialEq)]
pub struct Adam {
    /// 重み減衰0のPST群。
    pub pst: pst::Adam,
    /// Vとaを合わせたFM群。
    pub fm: fm::Adam,
}
impl Adam {
    /// 群ごとの学習率とFMの重み減衰を設定する。
    pub fn new(pst_lr: f64, fm_lr: f64, weight_decay: f64) -> Result<Self> {
        Ok(Self {
            pst: pst::Adam::new(pst_lr)?,
            fm: fm::Adam::new(fm_lr, weight_decay)?,
        })
    }
    /// 両群を更新し、PSTを切り詰め、FMの未観測行を0に戻す。
    pub fn update(
        &mut self,
        model: &mut Model,
        gradients: &Gradients,
        threads: usize,
    ) -> Result<()> {
        model.validate()?;
        self.validate()?;
        crate::nn::parallel::check_threads(threads)?;
        self.pst
            .update_parallel(&mut model.pst, &gradients.pst, threads)?;
        self.fm.update(&mut model.fm, &gradients.fm, threads)?;
        Ok(())
    }
    /// 完了した更新回数。
    pub fn step(&self) -> u64 {
        self.pst.step()
    }
    pub(super) fn validate(&self) -> Result<()> {
        self.pst.validate()?;
        self.fm.validate()?;
        if self.pst.step() != self.fm.step() {
            return Err("joint optimizer steps differ".into());
        }
        Ok(())
    }
}

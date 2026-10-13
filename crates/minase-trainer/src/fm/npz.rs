//! FMの実数重み`fm-float.npz`を符号化する。
//! コンテナは[`crate::nn::npz`]の無圧縮NPZである。
//!
//! | 配列名 | dtype | 形状 | 内容 |
//! |---|---|---|---|
//! | `V` | `<f4` | `[13680, 32]` | 特徴番号順に32成分ずつ埋め込みを置く。 |
//! | `a` | `<f4` | `[32]` | 各成分の出力係数を置く。 |
//! | `mask` | `\|b1` | `[13680]` | 観測済みを1、未観測を0で表す。 |
//!
//! [`FloatWeights::decode`]が読む`fm scale --float`用の5配列の仕様は、
//! 書き手の[`crate::joint`]にある。
//! この読み手は`middlegame`、`endgame`、`V`、`a`について`<f4`と`<f8`の有限値を受け付け、
//! それ以外のメンバーはZIPの検査和だけを検査する。

use super::Result;
use crate::nn::npz;
use minase::eval::{features::FEATURE_COUNT, pst::FM_RANK};
use std::collections::BTreeMap;

pub(crate) fn floats(values: &[f32], shape: &[usize]) -> npz::Array {
    npz::Array {
        dtype: "<f4".into(),
        shape: shape.to_vec(),
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

/// `fm-float.npz`として保存する圧縮なしNPZを作る。
/// `V`はdtype `<f4`、形`[13680, 32]`の特徴番号順の埋め込み、`a`は`<f4`の`[32]`の出力係数、
/// `mask`はdtype `|b1`、形`[13680]`の観測マスクである。
/// PSTの端点を持たないため、`fm scale --float`には使えない。
pub(super) fn encode_model(model: &super::Model) -> Result<Vec<u8>> {
    Ok(npz::encode(&[
        (
            "V",
            floats(
                &model.weights[..FEATURE_COUNT * FM_RANK],
                &[FEATURE_COUNT, FM_RANK],
            ),
        ),
        ("a", floats(model.a(), &[FM_RANK])),
        (
            "mask",
            npz::Array {
                dtype: "|b1".into(),
                shape: vec![FEATURE_COUNT],
                data: model.mask().iter().map(|&v| u8::from(v)).collect(),
            },
        ),
    ])?)
}

pub(super) struct FloatWeights {
    pub endpoints: Vec<[f64; 2]>,
    pub v: Vec<[f64; FM_RANK]>,
    pub a: [f64; FM_RANK],
}

pub(crate) fn array(
    arrays: &mut BTreeMap<String, npz::Array>,
    key: &str,
    shape: &[usize],
) -> Result<Vec<f64>> {
    let array = arrays
        .remove(key)
        .ok_or_else(|| format!("missing NPZ array {key}"))?;
    if array.shape != shape {
        return Err(format!("NPZ {key} must have shape {shape:?}").into());
    }
    let values: Vec<f64> = match array.dtype.as_str() {
        "<f4" => array
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&b| f64::from(f32::from_le_bytes(b)))
            .collect(),
        "<f8" => array
            .data
            .as_chunks::<8>()
            .0
            .iter()
            .map(|&b| f64::from_le_bytes(b))
            .collect(),
        _ => return Err(format!("NPZ {key} must have little-endian f32 or f64 dtype").into()),
    };
    if values.iter().any(|x| !x.is_finite()) {
        return Err(format!("NPZ {key} contains non-finite weights").into());
    }
    Ok(values)
}

impl FloatWeights {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        // maskなど変換に使わない配列は、汎用読み手がZIPの検査和だけを検査する。
        let mut arrays = npz::decode_selected(bytes, &["middlegame", "endgame", "V", "a"])?;
        let mg = array(&mut arrays, "middlegame", &[FEATURE_COUNT])?;
        let eg = array(&mut arrays, "endgame", &[FEATURE_COUNT])?;
        let v = array(&mut arrays, "V", &[FEATURE_COUNT, FM_RANK])?;
        let a = array(&mut arrays, "a", &[FM_RANK])?;
        Ok(Self {
            endpoints: mg.into_iter().zip(eg).map(|(mg, eg)| [mg, eg]).collect(),
            v: v.as_chunks::<FM_RANK>().0.to_vec(),
            a: a.try_into().expect("validated FM rank"),
        })
    }
}

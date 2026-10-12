use super::{Model, Result};
use crate::{
    data::{Dataset, epoch_permutation},
    fm,
    nn::{
        atomic_write,
        npz::{self, Array},
    },
    pst,
};
use minase::eval::{
    features::{FEATURE_COUNT, active_features},
    pst::{FM_RANK, Pst, evaluate_pst},
};
use minase_core::{Position, Square};
use serde::Serialize;
use std::path::Path;

/// 同時学習の初期値または最良モデルの量子化誤差。
#[derive(Clone, Debug, Serialize)]
pub struct QuantizationReport {
    /// 検証標本数。
    pub samples: usize,
    /// SplitMix64のシード。
    pub seed: u64,
    /// 常にvalidation。
    pub split: &'static str,
    /// 標本の通し番号のSHA-256。
    pub indices_sha256: String,
    /// FMの指数。
    pub exponent: u32,
    /// 平均絶対誤差、センチポーン。
    pub mean_absolute_error_cp: f64,
    /// 最大絶対誤差、センチポーン。
    pub max_absolute_error_cp: f64,
    /// 整数FM補正の母標準偏差、センチポーン。
    pub correction_cp_std: f64,
    /// 平均絶対誤差の上限、センチポーン。
    pub limit_cp: f64,
    /// 平均絶対誤差が2cp以下なら真。
    pub passed: bool,
}

pub(super) fn arrays(model: &Model) -> Vec<(&'static str, Array)> {
    let endpoints = model.pst.expanded();
    let mut arrays: Vec<_> = ["middlegame", "endgame"]
        .into_iter()
        .enumerate()
        .map(|(e, name)| {
            (
                name,
                fm::npz::floats(
                    &endpoints.iter().map(|row| row[e]).collect::<Vec<_>>(),
                    &[FEATURE_COUNT],
                ),
            )
        })
        .collect();
    arrays.push((
        "V",
        fm::npz::floats(
            &model.fm.v().iter().flatten().copied().collect::<Vec<_>>(),
            &[FEATURE_COUNT, FM_RANK],
        ),
    ));
    arrays.push(("a", fm::npz::floats(model.fm.a(), &[FM_RANK])));
    arrays.push((
        "mask",
        Array {
            dtype: "|b1".into(),
            shape: vec![FEATURE_COUNT],
            data: model.fm.mask().iter().map(|&v| u8::from(v)).collect(),
        },
    ));
    arrays
}
fn quantized(model: &Model, base: &Pst) -> Result<Pst> {
    model.validate()?;
    let fm = fm::quantize(
        &model
            .fm
            .v()
            .iter()
            .map(|r| r.map(f64::from))
            .collect::<Vec<_>>(),
        &model.fm.a().map(f64::from),
        f64::from(base.k()),
    )?;
    Ok(Pst::from_weights_with_fm(
        &pst::export::quantize(&model.pst)?,
        std::array::from_fn(|s| base.piece_value_of_state(s)),
        base.k(),
        base.rule_set(),
        fm.as_weights(),
    )?)
}
fn integer(network: &Pst, position: &Position, features: &[usize]) -> i32 {
    (i64::from(evaluate_pst(network, position))
        + fm::report::correction(network.fm_weights().expect("joint version 3"), features))
    .clamp(-28999, 28999) as i32
}
fn report(
    dataset: &Dataset,
    model: &Model,
    network: &Pst,
    sample: usize,
    seed: u64,
) -> Result<QuantizationReport> {
    let mut indices = epoch_permutation(dataset.validation_indices(), seed, 0);
    indices.truncate(sample);
    if indices.is_empty() {
        return Err("empty joint quantization sample".into());
    }
    let endpoints = model.pst.expanded();
    let v: Vec<_> = model.fm.v().iter().map(|r| r.map(f64::from)).collect();
    let a = model.fm.a().map(f64::from);
    let fm = network
        .fm_weights()
        .ok_or("joint output must have FM weights")?;
    let mut total = 0.0;
    let mut maximum = 0.0_f64;
    let mut corrections = Vec::with_capacity(indices.len());
    for &index in &indices {
        let position = dataset.record(index)?.to_position()?;
        let mut features = Vec::with_capacity(145);
        active_features(&position, |f| features.push(f));
        let count = Square::all()
            .filter(|&s| position.piece_at(s).is_some())
            .count();
        let ratio = count.saturating_sub(2).min(90) as f64 / 90.0;
        let sums: [f64; 2] =
            std::array::from_fn(|e| features.iter().map(|&f| f64::from(endpoints[f][e])).sum());
        let floating = ratio * sums[0]
            + (1.0 - ratio) * sums[1]
            + f64::from(model.pst.k()) * fm::report::phi(&v, &a, &features);
        let error = (floating - f64::from(integer(network, &position, &features))).abs();
        if !error.is_finite() {
            return Err("non-finite joint quantization error".into());
        }
        total += error;
        maximum = maximum.max(error);
        corrections.push(fm::report::correction(fm, &features) as f64);
    }
    let mean = total / indices.len() as f64;
    Ok(QuantizationReport {
        samples: indices.len(),
        seed,
        split: "validation",
        indices_sha256: fm::record::hash_u64(&indices),
        exponent: fm.exponent,
        mean_absolute_error_cp: mean,
        max_absolute_error_cp: maximum,
        correction_cp_std: fm::report::std(&corrections),
        limit_cp: 2.0,
        passed: mean <= 2.0,
    })
}
pub(super) fn quantization_report(
    dataset: &Dataset,
    model: &Model,
    base: &Pst,
    sample: usize,
    seed: u64,
) -> Result<QuantizationReport> {
    report(dataset, model, &quantized(model, base)?, sample, seed)
}
pub(super) fn write_best(
    dataset: &Dataset,
    model: &Model,
    base: &Pst,
    directory: &Path,
    sample: usize,
    seed: u64,
) -> Result<(QuantizationReport, i32)> {
    atomic_write(
        &directory.join("pst-float.npz"),
        &npz::encode(&arrays(model))?,
    )?;
    let output = quantized(model, base)?;
    atomic_write(&directory.join("pst.bin"), &output.encode())?;
    let written = Pst::decode(&std::fs::read(directory.join("pst.bin"))?)?;
    if written.weights() != output.weights()
        || written.fm_weights() != output.fm_weights()
        || written.k().to_bits() != base.k().to_bits()
        || written.rule_set() != base.rule_set()
        || (0..pst::v0_piece_values().len())
            .any(|s| written.piece_value_of_state(s) != base.piece_value_of_state(s))
    {
        return Err(
            "written joint MNPT differs from exported weights or base search constants".into(),
        );
    }
    let initial = Position::initial();
    let mut features = Vec::new();
    active_features(&initial, |f| features.push(f));
    Ok((
        report(dataset, model, &written, sample, seed)?,
        integer(&written, &initial, &features),
    ))
}

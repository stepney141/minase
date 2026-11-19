//! LayerStackのCPU/CUDA一致検査と更新時間の計測。
use super::{
    gpu::{engine::Evaluator, step::Step},
    *,
};
use crate::nnue::{
    data::{FeatureSet, synthetic::synthetic_batch},
    gpu::{Device, HostRing},
    options::{DeviceKind, TrainingOptions},
    train,
};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(crate) const ATOL: f64 = 1e-6;
pub(crate) const RTOL: f64 = 1e-5;
#[derive(Clone, clap::Args)]
/// CPU/CUDA一致検査の条件。
pub struct CheckArgs {
    #[arg(long)]
    /// 1バッチの局面数。
    pub batch: usize,
    #[arg(long)]
    /// 初期化と合成局面の乱数シード。
    pub seed: u64,
    #[arg(long)]
    /// 教師の勝率尺度、cp/ロジット。
    pub teacher_cp_per_logit: f64,
    #[command(flatten)]
    /// 学習率、重み減衰と教師の混合比。
    pub training: TrainingOptions,
}
#[derive(clap::Args)]
/// 更新時間の計測条件。
pub struct BenchArgs {
    #[command(flatten)]
    /// 合成入力と最適化器の条件。
    pub check: CheckArgs,
    #[arg(long, value_enum, default_value_t = DeviceKind::Cuda)]
    /// 計測する実行デバイス。
    pub device: DeviceKind,
    #[arg(long, default_value_t = 10)]
    /// 計測する更新数。
    pub steps: usize,
    #[arg(long)]
    /// CPUの入力供給の並列度。
    pub threads: Option<usize>,
    /// バッチ供給の計測に使うMNSDファイル。
    #[arg(long, required = true, num_args = 1..)]
    pub data: Vec<PathBuf>,
    /// 前進、後退、最適化と転送の時間内訳を出力する。
    #[arg(long)]
    pub profile: bool,
}
fn compare(report: &mut [f64; 3], actual: f64, expected: f64) -> train::Result<()> {
    let absolute = (actual - expected).abs();
    if !actual.is_finite() || !expected.is_finite() || absolute > ATOL + RTOL * expected.abs() {
        return Err(
            crate::nnue::gpu::Error::Equivalence(format!("GPU={actual}, CPU={expected}")).into(),
        );
    }
    report[0] += 1.0;
    report[1] = report[1].max(absolute);
    if expected != 0.0 {
        report[2] = report[2].max(absolute / expected.abs());
    }
    Ok(())
}
/// 前進、検証、連続更新の重みと両モーメントを照合する。
pub fn check(args: CheckArgs) -> train::Result<Value> {
    args.training.validate()?;
    let scale = Scale::new(args.teacher_cp_per_logit)?;
    let batch = synthetic_batch(FeatureSet::P, args.batch, args.seed)?;
    let features = Features::from_batch(&batch)?;
    let targets = crate::nnue::data::build_targets(
        &batch,
        &[scale.teacher(); 3],
        &[args.training.lambda; 3],
    )?;
    let mut weights = Parameters::initialize(args.seed, scale);
    let mut optimizer = AdamW::new(args.training.optimizer())?;
    let device = Device::new(0)?;
    let mut evaluator = Evaluator::new(device.clone(), &weights, scale, args.batch)?;
    let validation_device_bytes = evaluator.device_bytes()?;
    let actual = evaluator.evaluate(&batch, &targets)?;
    let mut report = [0.0; 3];
    for (feature, actual) in features.iter().zip(&actual.forward) {
        let expected = forward(&weights, feature)?;
        for (actual, expected) in forward_values(actual)
            .into_iter()
            .zip(forward_values(&expected))
        {
            compare(&mut report, actual, expected)?;
        }
    }
    let (expected, lower, upper) = super::trainer::validation(&weights, scale, &batch, &targets)?;
    for (actual, expected) in actual
        .validation
        .sums()
        .into_iter()
        .chain([actual.square_lower as f64, actual.square_upper as f64])
        .zip(
            expected
                .sums()
                .into_iter()
                .chain([lower as f64, upper as f64]),
        )
    {
        compare(&mut report, actual, expected)?;
    }
    drop(evaluator);
    let mut step = Step::new(device.clone(), &weights, &optimizer, scale, args.batch)?;
    let mut ring = HostRing::new(&device, FeatureSet::P, args.batch)?;
    for _ in 0..3 {
        let (loss, gradients) = backward(&weights, scale, &features, &targets)?;
        optimizer.update(&mut weights, &gradients, 1.0)?;
        let ticket = ring.upload(&batch)?;
        let actual = step.enqueue(&mut ring, &ticket, &targets, 1.0)?.collect()?;
        compare(&mut report, actual.loss, loss)?;
        if actual.step != optimizer.step {
            return Err(train::Error::Invalid("CPU/CUDA update count differs").into());
        }
        let gpu_gradients = step.gradients()?;
        for p in Parameter::ALL {
            for (&a, &e) in gpu_gradients[p].iter().zip(&gradients[p]) {
                compare(&mut report, f64::from(a), f64::from(e))?;
            }
        }
        let (gpu, adam) = step.snapshot()?;
        for (gpu, cpu) in [
            (&gpu, &weights),
            (&adam.m, &optimizer.m),
            (&adam.v, &optimizer.v),
        ] {
            for p in Parameter::ALL {
                for (&a, &e) in gpu[p].iter().zip(&cpu[p]) {
                    compare(&mut report, f64::from(a), f64::from(e))?;
                }
            }
        }
    }
    Ok(
        json!({"device": device.name(), "validation_device_bytes": validation_device_bytes, "comparisons": report[0] as u64, "max_absolute_error": report[1], "max_relative_error": report[2], "atol": ATOL, "rtol": RTOL}),
    )
}
fn forward_values(f: &Forward) -> Vec<f64> {
    f.a.iter()
        .flatten()
        .chain(&f.p)
        .chain(&f.y)
        .chain(&f.z)
        .chain(&f.u)
        .copied()
        .chain([f.n, f.psqt[0], f.psqt[1], f.output])
        .collect()
}
/// 実データを供給して更新時間を測る。
pub fn bench(args: BenchArgs) -> train::Result<Value> {
    super::bench::run(args)
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires a CUDA GPU"]
    fn gpu_cli_equivalence_checks_forward_gradients_and_updates() {
        super::check(super::CheckArgs {
            batch: 129,
            seed: 71,
            teacher_cp_per_logit: 1072.65,
            training: crate::nnue::options::TrainingOptions::new(0.75),
        })
        .unwrap();
    }
}

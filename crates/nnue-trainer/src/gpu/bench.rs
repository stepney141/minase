//! 実データの供給、面の環を通る転送、およびGPUの1更新を別々に測る。

use super::{Device, Error, HostRing, Result, Step, StepResult, ring::TransferTiming};
use crate::{
    data::{
        BatchSlot, CANONICAL_ROWS, Dataset, InputFeatureSet, MAX_BATCH_SIZE, build_batches,
        epoch_permutation,
    },
    model::{Masks, Model, Parameter, Shape},
    reference::{AdamW, AdamWConfig, Losses, RemovalReference, targets, v0_piece_values},
};
use minase::eval::features::FEATURE_COUNT;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::{Duration, Instant},
};

/// `bench-step`で使う実データと測定条件。
#[derive(clap::Args)]
pub struct Args {
    /// Input feature set.
    #[arg(long, value_enum)]
    feature_set: InputFeatureSet,
    /// Even hidden width, 2..=512.
    #[arg(long)]
    width: usize,
    /// Positions in each measured batch.
    #[arg(long)]
    batch: usize,
    /// Measured updates, at least 2; one additional update warms up the GPU.
    #[arg(long)]
    steps: usize,
    /// MNSD files, in global-record order, including their provenance sidecars.
    #[arg(long, required = true, num_args = 1..)]
    data: Vec<PathBuf>,
    /// Model initialization and epoch permutation seed.
    #[arg(long, default_value_t = 71)]
    seed: u64,
    /// Batch workers; omit to use physical cores minus 2, with a minimum of 1.
    #[arg(long)]
    workers: Option<usize>,
    /// CUDA device ordinal.
    #[arg(long, default_value_t = 0)]
    device: usize,
    /// Fixed output and teacher K for this timing experiment.
    #[arg(long, default_value_t = 1000.0)]
    k: f32,
}

/// ウォームアップを除いた実測値。
pub struct Report {
    /// CUDAが報告したデバイス名。
    pub device_name: String,
    /// 入力特徴集合。
    pub feature_set: InputFeatureSet,
    /// 第1層の幅。
    pub width: usize,
    /// モデル初期化と局面順列のシード。
    pub seed: u64,
    /// 並列に構築するバッチ数。最終の群だけは少なくなることがある。
    pub workers: usize,
    /// 測定した更新数。
    pub steps: usize,
    /// 1更新の局面数。
    pub batch: usize,
    /// 出力と全教師分類に共通のK。
    pub k: f32,
    /// 計算ストリームのイベントから得た1更新の平均ミリ秒。
    pub compute_ms: f64,
    /// 転送ストリームのイベントから得た1バッチの平均ミリ秒。
    pub transfer_ms: f64,
    /// 1バッチの入力転送のバイト数。
    pub transfer_bytes: usize,
    /// ワーカーの構築時間から求めた毎秒の供給局面数。
    pub worker_positions_per_second: f64,
    /// 最初のバッチの構築開始から最後の損失回収までの秒数。
    pub elapsed_seconds: f64,
    /// CUDAが報告した測定時のGPUメモリ使用量。他のプロセスも含む。
    pub gpu_used_bytes: usize,
    /// CUDAが報告したGPUメモリ総量。
    pub gpu_total_bytes: usize,
    /// GPU入力面と更新器の確保前後でのメモリ使用量の差。
    pub gpu_allocation_bytes: i128,
    /// 測定した更新の平均損失。
    pub losses: Losses,
}

struct PreparedBatch {
    batch: BatchSlot,
    targets: Vec<f32>,
}

struct SupplyStats {
    positions: usize,
    build_time: Duration,
}

#[derive(Default)]
struct Measurements {
    compute_ms: f64,
    transfer_ms: f64,
    binary_cross_entropy: f64,
    auxiliary: f64,
}

impl Measurements {
    fn collect(&mut self, pending: (StepResult, TransferTiming)) -> Result<()> {
        let result = pending.0.collect()?;
        self.compute_ms += f64::from(result.compute_ms);
        self.transfer_ms += f64::from(pending.1.collect()?) + f64::from(result.target_transfer_ms);
        self.binary_cross_entropy += result.losses.bce;
        self.auxiliary += result.losses.auxiliary;
        Ok(())
    }
}

/// 準備と1回のウォームアップの後、実データで指定回数の更新を測定する。
/// 全行を観測済みとして初期化し、教師Kの推定や学習の検証は行わない。
pub fn run(args: Args) -> Result<Report> {
    let shape = Shape::new(args.feature_set, args.width, args.k)?;
    if !(1..=MAX_BATCH_SIZE).contains(&args.batch) {
        return Err(Error::Invalid("--batch must be in 1..=MAX_BATCH_SIZE"));
    }
    if args.steps < 2 {
        return Err(Error::Invalid("--steps must be at least 2"));
    }
    let required = args
        .steps
        .checked_add(1)
        .and_then(|steps| steps.checked_mul(args.batch))
        .ok_or(Error::Invalid("benchmark position count overflow"))?;
    let workers = match args.workers {
        Some(0) => return Err(Error::Invalid("--workers must be positive")),
        Some(n) => n,
        None => physical_workers()?,
    }
    .min(args.steps);
    let dataset = Dataset::open(&args.data, None)?;
    if dataset.training_indices().len() < required {
        return Err(Error::Invalid(
            "not enough training positions for (steps + 1) * batch, including warmup",
        ));
    }
    let mut indices = epoch_permutation(dataset.training_indices(), args.seed, 0);
    indices.truncate(required);
    let teacher_ks = vec![f64::from(args.k); dataset.teacher_classes().len()];
    let warmup = build_batches(&dataset, args.feature_set, &[&indices[..args.batch]])?
        .pop()
        .ok_or(Error::Invalid("warmup worker returned no batch"))?;
    let warmup_targets = targets(&warmup, &teacher_ks)?;
    let model = benchmark_model(shape, args.seed)?;
    let optimizer = AdamW::new(shape, AdamWConfig::default())?;
    let device = Device::new(args.device)?;
    let (free_before, _) = device.memory_info()?;
    let mut ring = HostRing::new(&device, args.feature_set, args.batch)?;
    let mut step = Step::new(
        device.clone(),
        &model,
        &optimizer,
        &RemovalReference::v0(),
        args.batch,
    )?;
    let warmup_ticket = ring.upload(&warmup)?;
    step.enqueue(&mut ring, &warmup_ticket, &warmup_targets)?
        .collect()?;
    drop(warmup);
    let (free_after, total) = device.memory_info()?;
    let transfer_bytes = ring.transfer_bytes() + args.batch * size_of::<f32>();

    let (measurements, supply, elapsed_seconds) = std::thread::scope(|scope| {
        // チャネルには完成バッチを3個まで置き、供給側の群はワーカー数以下にする。
        // 固定メモリの3面とデバイス2面の再利用はHostRingが管理する。
        let (sender, receiver) = sync_channel(3);
        let started = Instant::now();
        let remaining = &indices[args.batch..];
        let producer = std::thread::Builder::new()
            .name("nnue-bench-supply".into())
            .spawn_scoped(scope, || {
                supply_batches(&dataset, &args, remaining, &teacher_ks, workers, sender)
            })
            .map_err(crate::data::Error::Io)?;
        let measurements = consume_batches(&mut ring, &mut step, &receiver, args.steps);
        // CUDA失敗時も送信側の待機を解除して、全ワーカーを回収する。
        drop(receiver);
        let supply = producer
            .join()
            .map_err(|_| crate::data::Error::WorkerPanicked)??;
        let measurements = measurements?;
        Ok::<_, Error>((measurements, supply, started.elapsed().as_secs_f64()))
    })?;
    let steps = args.steps as f64;
    if supply.build_time.is_zero() {
        return Err(Error::Invalid("worker timing resolution"));
    }
    Ok(Report {
        device_name: device.name().to_owned(),
        feature_set: args.feature_set,
        width: args.width,
        seed: args.seed,
        workers,
        steps: args.steps,
        batch: args.batch,
        k: args.k,
        compute_ms: measurements.compute_ms / steps,
        transfer_ms: measurements.transfer_ms / steps,
        transfer_bytes,
        worker_positions_per_second: supply.positions as f64 / supply.build_time.as_secs_f64(),
        elapsed_seconds,
        gpu_used_bytes: total - free_after,
        gpu_total_bytes: total,
        gpu_allocation_bytes: free_before as i128 - free_after as i128,
        losses: Losses {
            bce: measurements.binary_cross_entropy / steps,
            auxiliary: measurements.auxiliary / steps,
        },
    })
}

fn supply_batches(
    dataset: &Dataset,
    args: &Args,
    indices: &[u64],
    teacher_ks: &[f64],
    workers: usize,
    sender: SyncSender<PreparedBatch>,
) -> Result<SupplyStats> {
    let mut stats = SupplyStats {
        positions: 0,
        build_time: Duration::ZERO,
    };
    // バッチ群の構築時間には復号、特徴抽出、逆索引、教師値変換、
    // ワーカーの起動と回収を含め、送信先が満杯の時間は含めない。
    for group in indices.chunks(args.batch * workers) {
        let ranges: Vec<_> = group.chunks(args.batch).collect();
        let started = Instant::now();
        let batches = build_batches(dataset, args.feature_set, &ranges)?;
        let prepared = batches
            .into_iter()
            .map(|batch| {
                let targets = targets(&batch, teacher_ks)?;
                Ok(PreparedBatch { batch, targets })
            })
            .collect::<Result<Vec<_>>>()?;
        stats.build_time += started.elapsed();
        stats.positions += group.len();
        for batch in prepared {
            if sender.send(batch).is_err() {
                // 受信側のエラーを隠さず、打ち切り時にも全ワーカーを回収する。
                return Ok(stats);
            }
        }
    }
    Ok(stats)
}

fn consume_batches(
    ring: &mut HostRing,
    step: &mut Step,
    receiver: &Receiver<PreparedBatch>,
    steps: usize,
) -> Result<Measurements> {
    let mut measurements = Measurements::default();
    let mut pending = None;
    for _ in 0..steps {
        let prepared = receiver
            .recv()
            .map_err(|_| Error::Invalid("batch worker stopped early"))?;
        let ticket = ring.upload(&prepared.batch)?;
        let transfer = ring.transfer_timing(&ticket)?;
        let result = step.enqueue(ring, &ticket, &prepared.targets)?;
        // 次の更新を投入してから直前の損失とイベント時刻を回収する。
        if let Some(previous) = pending.replace((result, transfer)) {
            measurements.collect(previous)?;
        }
    }
    if let Some(last) = pending {
        measurements.collect(last)?;
    }
    Ok(measurements)
}

fn benchmark_model(shape: Shape, seed: u64) -> Result<Model> {
    let mut canonical = vec![true; CANONICAL_ROWS];
    canonical[CANONICAL_ROWS - 1] = false;
    let mut input = vec![true; shape.feature_set().dimensions() + 1];
    input[shape.feature_set().dimensions()] = false;
    let masks = Masks::new(shape.feature_set(), canonical, input)?;
    let values = v0_piece_values();
    let initial: Vec<_> = (0..FEATURE_COUNT)
        .map(|base| {
            let value = if base < 94 * 144 {
                let sign = if base < 47 * 144 { 1.0 } else { -1.0 };
                sign * values[base / 144 % 47] as f32
            } else {
                0.0
            };
            [value; 2]
        })
        .collect();
    let mut model = Model::initialize(shape, &initial, masks, seed, 0.05)?;
    model.parameter_mut(Parameter::Biases).fill(0.25);
    for (i, weight) in model
        .parameter_mut(Parameter::OutputWeights)
        .iter_mut()
        .enumerate()
    {
        *weight = if i.is_multiple_of(2) { 0.01 } else { -0.01 };
    }
    model.constrain()?;
    Ok(model)
}

fn physical_workers() -> Result<usize> {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").map_err(crate::data::Error::Io)?;
    let mut cores = HashSet::new();
    for processor in cpuinfo.split("\n\n").filter(|s| !s.trim().is_empty()) {
        let field = |key: &str| {
            processor.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.trim() == key).then_some(value.trim())
            })
        };
        let package = field("physical id").ok_or(Error::Invalid(
            "CPU topology has no physical id; set --workers explicitly",
        ))?;
        let core = field("core id").ok_or(Error::Invalid(
            "CPU topology has no core id; set --workers explicitly",
        ))?;
        cores.insert((package, core));
    }
    if cores.is_empty() {
        return Err(Error::Invalid(
            "CPU topology is empty; set --workers explicitly",
        ));
    }
    Ok(cores.len().saturating_sub(2).max(1))
}

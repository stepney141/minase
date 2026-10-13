//! 実データの供給、転送、および指定したデバイスの1更新を別々に測る。

use super::{
    Device, Error, HostRing, Result, Step, StepResult, ring::TransferTiming, step::KernelTiming,
};
use crate::{
    data::{Dataset, epoch_permutation},
    nnue::{
        data::{
            BatchSlot, CANONICAL_ROWS, InputFeatureSet, MAX_BATCH_SIZE, build_batches,
            build_targets,
        },
        model::{Masks, Model, Parameter, Shape},
        options::{self, DeviceKind, TrainingOptions},
        reference::{self, AdamW, Losses, RemovalReference, v0_piece_values},
    },
};
use cudarc::driver::LaunchConfig;
use minase::eval::features::FEATURE_COUNT;
use std::{
    fmt::Write,
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::{Duration, Instant},
};

/// `bench-step`で使う実データと測定条件。
#[derive(clap::Args)]
pub struct Args {
    /// 入力の特徴集合を指定する。
    #[arg(long, value_enum)]
    feature_set: InputFeatureSet,
    /// 隠れ層の幅を2以上512以下の偶数で指定する。
    #[arg(long)]
    width: usize,
    /// 測定する1バッチの局面数を指定する。
    #[arg(long)]
    batch: usize,
    /// 測定する更新回数を2以上で指定する。別に1回の更新でウォームアップする。
    #[arg(long)]
    steps: usize,
    /// CUDAイベントでカーネルごとに計時し、起動時の次元と配列の推定バイト数を報告する。
    #[arg(long)]
    profile: bool,
    /// MNSDファイルを記録の通し番号順に指定する。対応する来歴ファイルも用意する。
    #[arg(long, required = true, num_args = 1..)]
    data: Vec<PathBuf>,
    /// モデル初期化とエポック内の局面順列に使うシードを指定する。
    #[arg(long, default_value_t = 71)]
    seed: u64,
    /// バッチ構築とCPU更新のスレッド数を指定する。省略時は物理コア数から2を引く。
    #[arg(long)]
    threads: Option<usize>,
    /// 更新を実行するデバイスを指定する。
    #[arg(long, value_enum, default_value_t = DeviceKind::Cuda)]
    device: DeviceKind,
    /// この測定で固定する出力用Kと教師用Kの共通値を指定する。
    #[arg(long, default_value_t = 1000.0)]
    k: f32,
    /// 学習率、重み減衰、補助損失の係数、および教師値の混合比。
    #[command(flatten)]
    training: TrainingOptions,
}

/// ウォームアップを除いた実測値。
pub struct Report {
    /// 更新を実行したデバイスの種類。
    pub device: DeviceKind,
    /// デバイス名。CPUでは`CPU`。
    pub device_name: String,
    /// 入力特徴集合。
    pub feature_set: InputFeatureSet,
    /// 第1層の幅。
    pub width: usize,
    /// モデル初期化と局面順列のシード。
    pub seed: u64,
    /// 並列に構築するバッチ数。最終の群だけは少なくなることがある。
    pub workers: usize,
    /// 指定された並列度。CPU更新でも同じ値を使う。
    pub threads: usize,
    /// 測定した更新数。
    pub steps: usize,
    /// 1更新の局面数。
    pub batch: usize,
    /// 出力と全教師分類に共通のK。
    pub k: f32,
    /// 1更新の平均ミリ秒。CUDAではイベント、CPUではホストの経過時間を使う。
    pub compute_ms: f64,
    /// 転送ストリームのイベントから得た1バッチの平均ミリ秒。
    pub transfer_ms: f64,
    /// 1バッチの入力転送のバイト数。
    pub transfer_bytes: usize,
    /// ワーカーの構築時間から求めた毎秒の供給局面数。
    pub worker_positions_per_second: f64,
    /// 最初のバッチの構築開始から最後の損失回収までの秒数。
    pub elapsed_seconds: f64,
    /// CUDAが報告した測定時のGPUメモリ使用量。他のプロセスも含む。CPUではNone。
    pub gpu_used_bytes: Option<usize>,
    /// CUDAが報告したGPUメモリ総量。CPUではNone。
    pub gpu_total_bytes: Option<usize>,
    /// GPU入力面と更新器の確保前後でのメモリ使用量の差。CPUではNone。
    pub gpu_allocation_bytes: Option<i128>,
    /// 測定した更新の平均損失。
    pub losses: Losses,
    /// `--profile`指定時だけ作る、ウォームアップを除いたカーネル別の計測表。
    pub profile: Option<String>,
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
    profile: ProfileMeasurements,
}

impl Measurements {
    fn collect(&mut self, pending: (StepResult, TransferTiming)) -> Result<()> {
        let (result, profile) = pending.0.collect_profiled()?;
        self.compute_ms += f64::from(result.compute_ms);
        self.transfer_ms += f64::from(pending.1.collect()?) + f64::from(result.target_transfer_ms);
        self.binary_cross_entropy += result.losses.bce;
        self.auxiliary += result.losses.auxiliary;
        for timing in profile {
            self.profile.add(timing);
        }
        Ok(())
    }
}

struct KernelTotals {
    name: &'static str,
    total_ms: f64,
    calls: usize,
    read_bytes: u64,
    write_bytes: u64,
    configurations: Vec<(LaunchConfig, usize)>,
}

#[derive(Default)]
struct ProfileMeasurements {
    kernels: Vec<KernelTotals>,
}

impl ProfileMeasurements {
    fn add(&mut self, timing: KernelTiming) {
        let index = match self.kernels.iter().position(|k| k.name == timing.name) {
            Some(index) => index,
            None => {
                self.kernels.push(KernelTotals {
                    name: timing.name,
                    total_ms: 0.0,
                    calls: 0,
                    read_bytes: 0,
                    write_bytes: 0,
                    configurations: Vec::new(),
                });
                self.kernels.len() - 1
            }
        };
        let kernel = &mut self.kernels[index];
        kernel.total_ms += f64::from(timing.elapsed_ms);
        kernel.calls += 1;
        kernel.read_bytes += timing.read_bytes;
        kernel.write_bytes += timing.write_bytes;
        if let Some((_, count)) = kernel.configurations.iter_mut().find(|(cfg, _)| {
            cfg.grid_dim == timing.config.grid_dim && cfg.block_dim == timing.config.block_dim
        }) {
            *count += 1;
        } else {
            kernel.configurations.push((timing.config, 1));
        }
    }

    fn table(&self, steps: usize) -> String {
        let total_ms: f64 = self.kernels.iter().map(|k| k.total_ms).sum();
        let updates = steps as f64;
        let mut table = format!(
            "カーネル別計測はウォームアップを除く{steps}更新です。平均時間と起動回数は1更新あたり、合計時間は全更新分です。\n\
             割合の分母はカーネル時間の合計です。格子には各構成の1更新あたりの起動回数を添え、ブロックは格子と同じ順に並べます。\n\
             read/writeは1更新あたりの主要配列の理論バイト数です。キャッシュ・共有メモリを除き、除去の有効列とAdamWの観測行には上限を用います。\n\
             計測イベントの挿入による負荷があるため、通常実行の速度は--profileなしで測定してください。\n"
        );
        writeln!(
            table,
            "{:<22} {:>14} {:>12} {:>12} {:>8} {:<38} {:<25} {:>18} {:>18}",
            "kernel",
            "mean_ms/update",
            "total_ms",
            "calls/update",
            "share_%",
            "grid(x,y,z):calls/update",
            "block(x,y,z)",
            "read_bytes/update",
            "write_bytes/update"
        )
        .unwrap();
        for kernel in &self.kernels {
            let grids = kernel
                .configurations
                .iter()
                .map(|(cfg, calls)| {
                    let (x, y, z) = cfg.grid_dim;
                    format!("({x},{y},{z}):{:.1}", *calls as f64 / updates)
                })
                .collect::<Vec<_>>()
                .join(";");
            let blocks = kernel
                .configurations
                .iter()
                .map(|(cfg, _)| {
                    let (x, y, z) = cfg.block_dim;
                    format!("({x},{y},{z})")
                })
                .collect::<Vec<_>>()
                .join(";");
            // イベント時刻がすべて0なら割合は未定義であり、0%とは表示しない。
            let share = if total_ms == 0.0 {
                "n/a".to_owned()
            } else {
                format!("{:.1}", 100.0 * kernel.total_ms / total_ms)
            };
            writeln!(
                table,
                "{:<22} {:>14.3} {:>12.3} {:>12.1} {:>8} {:<38} {:<25} {:>18.0} {:>18.0}",
                kernel.name,
                kernel.total_ms / updates,
                kernel.total_ms,
                kernel.calls as f64 / updates,
                share,
                grids,
                blocks,
                kernel.read_bytes as f64 / updates,
                kernel.write_bytes as f64 / updates,
            )
            .unwrap();
        }
        write!(
            table,
            "カーネル時間の合計は1更新平均 {:.3} ms、全{steps}更新で {total_ms:.3} msです。",
            total_ms / updates
        )
        .unwrap();
        table
    }
}

/// 1回のウォームアップ後に、2以上の`--steps`回の更新を実データで測る。
/// シードの既定値71とエポック0の順列から`(steps+1)×batch`局面を重複なく使う。
/// モデルは`benchmark_model`で作り、出力と全教師分類に同じKを使う。Kの既定値は1000である。
/// CPUでは転送時間を0とし、`--profile`との併用を拒否する。
/// 教師Kの推定や学習の検証は行わない。
pub fn run(args: Args) -> std::result::Result<Report, Box<dyn std::error::Error>> {
    args.training.validate()?;
    let shape = Shape::new(args.feature_set, args.width, args.k)?;
    if !(1..=MAX_BATCH_SIZE).contains(&args.batch) {
        return Err(Error::Invalid("--batch must be in 1..=MAX_BATCH_SIZE").into());
    }
    if args.steps < 2 {
        return Err(Error::Invalid("--steps must be at least 2").into());
    }
    if args.device == DeviceKind::Cpu && args.profile {
        return Err(Error::Invalid("--profile requires --device cuda").into());
    }
    let required = args
        .steps
        .checked_add(1)
        .and_then(|steps| steps.checked_mul(args.batch))
        .ok_or(Error::Invalid("benchmark position count overflow"))?;
    let threads = options::workers(args.threads)?;
    let workers = threads.min(args.steps);
    // SAFETY: 入力は生成完了済みのMNSDに限り、Datasetの生存中は外部プロセスも含めて
    // 変更・切り詰めしない運用契約とする。この処理も入力ファイルを書き換えない。
    let dataset = unsafe { Dataset::open(&args.data, None, Some(args.training.lambda_override))? };
    if dataset.training_indices().len() < required {
        return Err(Error::Invalid(
            "not enough training positions for (steps + 1) * batch, including warmup",
        )
        .into());
    }
    let mut indices = epoch_permutation(dataset.training_indices(), args.seed, 0);
    indices.truncate(required);
    let teacher_ks = vec![f64::from(args.k); dataset.teacher_classes().len()];
    let model = benchmark_model(shape, args.seed)?;
    if args.device == DeviceKind::Cpu {
        return Ok(run_cpu(&args, &dataset, &indices, threads, model)?);
    }
    let warmup = build_batches(&dataset, args.feature_set, &[&indices[..args.batch]])?
        .pop()
        .ok_or(Error::Invalid("warmup worker returned no batch"))?;
    let warmup_targets = build_targets(&warmup, &teacher_ks, dataset.teacher_lambdas())?;
    let optimizer = AdamW::new(shape, args.training.optimizer())?;
    let device = Device::new(0)?;
    let (free_before, _) = device.memory_info()?;
    let mut ring = HostRing::new(&device, args.feature_set, args.batch)?;
    let mut step = Step::new(
        device.clone(),
        &model,
        &optimizer,
        &RemovalReference::v0(),
        args.batch,
        args.training.removal_penalty,
    )?;
    if args.profile {
        step.enable_profiling();
    }
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
            .map_err(crate::nnue::data::Error::Io)?;
        let measurements = consume_batches(&mut ring, &mut step, &receiver, args.steps);
        // CUDA失敗時も送信側の待機を解除して、全ワーカーを回収する。
        drop(receiver);
        let supply = producer
            .join()
            .map_err(|_| crate::nnue::data::Error::WorkerPanicked)??;
        let measurements = measurements?;
        Ok::<_, Error>((measurements, supply, started.elapsed().as_secs_f64()))
    })?;
    let steps = args.steps as f64;
    if supply.build_time.is_zero() {
        return Err(Error::Invalid("worker timing resolution").into());
    }
    Ok(Report {
        device: args.device,
        device_name: device.name().to_owned(),
        feature_set: args.feature_set,
        width: args.width,
        seed: args.seed,
        workers,
        threads,
        steps: args.steps,
        batch: args.batch,
        k: args.k,
        compute_ms: measurements.compute_ms / steps,
        transfer_ms: measurements.transfer_ms / steps,
        transfer_bytes,
        worker_positions_per_second: supply.positions as f64 / supply.build_time.as_secs_f64(),
        elapsed_seconds,
        gpu_used_bytes: Some(total - free_after),
        gpu_total_bytes: Some(total),
        gpu_allocation_bytes: Some(free_before as i128 - free_after as i128),
        losses: Losses {
            bce: measurements.binary_cross_entropy / steps,
            auxiliary: measurements.auxiliary / steps,
        },
        profile: args.profile.then(|| measurements.profile.table(args.steps)),
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
                let targets = build_targets(&batch, teacher_ks, dataset.teacher_lambdas())?;
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

fn run_cpu(
    args: &Args,
    dataset: &Dataset,
    indices: &[u64],
    threads: usize,
    mut model: Model,
) -> Result<Report> {
    let mut optimizer = AdamW::new(model.shape(), args.training.optimizer())?;
    let removal = RemovalReference::v0();
    let teacher_ks = vec![f64::from(args.k); dataset.teacher_classes().len()];
    let workers = threads.min(args.steps);
    let mut update = |prepared: &PreparedBatch| -> Result<Losses> {
        let gradients = reference::loss_and_gradients_parallel(
            &model,
            &prepared.batch,
            &prepared.targets,
            &removal,
            args.training.removal_penalty,
            threads,
        )?;
        optimizer.update_parallel(&mut model, &gradients.parameters, threads)?;
        Ok(gradients.losses)
    };
    let warmup = build_batches(dataset, args.feature_set, &[&indices[..args.batch]])?
        .pop()
        .ok_or(Error::Invalid("warmup worker returned no batch"))?;
    let warmup_targets = build_targets(&warmup, &teacher_ks, dataset.teacher_lambdas())?;
    update(&PreparedBatch {
        batch: warmup,
        targets: warmup_targets,
    })?;

    let (measurements, supply, elapsed_seconds) = std::thread::scope(|scope| {
        let (sender, receiver) = sync_channel(3);
        let started = Instant::now();
        let producer = std::thread::Builder::new()
            .name("nnue-bench-supply".into())
            .spawn_scoped(scope, || {
                supply_batches(
                    dataset,
                    args,
                    &indices[args.batch..],
                    &teacher_ks,
                    workers,
                    sender,
                )
            })
            .map_err(crate::nnue::data::Error::Io)?;
        let measurements = (|| -> Result<Measurements> {
            let mut measurements = Measurements::default();
            for _ in 0..args.steps {
                let prepared = receiver
                    .recv()
                    .map_err(|_| Error::Invalid("batch worker stopped early"))?;
                let compute_started = Instant::now();
                let losses = update(&prepared)?;
                measurements.compute_ms += compute_started.elapsed().as_secs_f64() * 1000.0;
                measurements.binary_cross_entropy += losses.bce;
                measurements.auxiliary += losses.auxiliary;
            }
            Ok(measurements)
        })();
        drop(receiver);
        let supply = producer
            .join()
            .map_err(|_| crate::nnue::data::Error::WorkerPanicked)??;
        Ok::<_, Error>((measurements?, supply, started.elapsed().as_secs_f64()))
    })?;
    if supply.build_time.is_zero() {
        return Err(Error::Invalid("worker timing resolution"));
    }
    let steps = args.steps as f64;
    Ok(Report {
        device: args.device,
        device_name: "CPU".into(),
        feature_set: args.feature_set,
        width: args.width,
        seed: args.seed,
        workers,
        threads,
        steps: args.steps,
        batch: args.batch,
        k: args.k,
        compute_ms: measurements.compute_ms / steps,
        transfer_ms: 0.0,
        transfer_bytes: 0,
        worker_positions_per_second: supply.positions as f64 / supply.build_time.as_secs_f64(),
        elapsed_seconds,
        gpu_used_bytes: None,
        gpu_total_bytes: None,
        gpu_allocation_bytes: None,
        losses: Losses {
            bce: measurements.binary_cross_entropy / steps,
            auxiliary: measurements.auxiliary / steps,
        },
        profile: None,
    })
}

/// 埋め草以外の全行を観測済みとし、線形はv0、第1層は標準偏差0.05で初期化する。
/// バイアスは0.25、出力重みは+0.01と−0.01を交互に置く。
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

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(name: &'static str, elapsed_ms: f32, grid: u32, bytes: u64) -> KernelTiming {
        KernelTiming {
            name,
            config: LaunchConfig {
                grid_dim: (grid, 1, 1),
                block_dim: (128, 1, 1),
                shared_mem_bytes: 0,
            },
            elapsed_ms,
            read_bytes: bytes,
            write_bytes: bytes / 2,
        }
    }

    #[test]
    fn profile_table_sums_launches_and_averages_over_updates() {
        // 仕様: 同名カーネルをまとめ、起動数ではなく測定した更新数で平均する。
        // 2更新でaは(1+3)+(2+4)=10ms、bは0.5+2=2.5msなので割合は80%と20%。
        let mut measurements = ProfileMeasurements::default();
        for (a1, a2, b, bytes) in [(1.0, 3.0, 0.5, 100), (2.0, 4.0, 2.0, 200)] {
            measurements.add(timing("a", a1, 1, bytes));
            measurements.add(timing("a", a2, 2, bytes * 2));
            measurements.add(timing("b", b, 1, 40));
        }
        let table = measurements.table(2);
        let row = |name| {
            table
                .lines()
                .find_map(|line| {
                    let columns: Vec<_> = line.split_whitespace().collect();
                    (columns.first().copied() == Some(name)).then_some(columns)
                })
                .unwrap()
        };
        assert_eq!(
            row("a"),
            [
                "a",
                "5.000",
                "10.000",
                "2.0",
                "80.0",
                "(1,1,1):1.0;(2,1,1):1.0",
                "(128,1,1);(128,1,1)",
                "450",
                "225",
            ]
        );
        assert_eq!(
            row("b"),
            [
                "b",
                "1.250",
                "2.500",
                "1.0",
                "20.0",
                "(1,1,1):1.0",
                "(128,1,1)",
                "40",
                "20",
            ]
        );
        assert!(table.contains("1更新平均 6.250 ms、全2更新で 12.500 ms"));
    }

    #[test]
    fn zero_kernel_time_has_no_percentage() {
        let mut measurements = ProfileMeasurements::default();
        measurements.add(timing("a", 0.0, 1, 0));
        let table = measurements.table(1);
        let row = table.lines().find(|line| line.starts_with("a ")).unwrap();
        assert_eq!(row.split_whitespace().nth(4), Some("n/a"));
    }
}

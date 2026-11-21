//! Gumbel AlphaZeroによる自己対局を生成し、教師データ形式MNSDの版3で保存する。
//! 規則は`lishogi`を使い、規則上の終局、王手限定の3手以内の勝ち、3,000手上限の順に判定する。
//! 上限に達した対局は引き分けとして、投了した対局も含めて学習に残す。
//! `docs/adr/draw-zero-self-play-games-at-3000-plies-and-keep-them-for-training.md`を参照。
//!
//! 起動引数はすべて必須で、既定値を持たない。
//! `--start-position`は`initial`か`shuffled-setup`、`--simulations`は1以上の全手番共通の探索回数とする。
//! 最初の30手は探索後に訪問回数に比例して抽選し、全探索局面に価値と方策を記録する。
//! `docs/adr/start-zero-self-play-from-the-initial-position-and-sample-the-first-30-moves-by-root-visit-counts.md`を参照。
//! `docs/adr/search-every-zero-self-play-move-with-32-simulations-and-train-on-every-searched-position.md`を参照。
//!
//! 閾値は実行器がAobaZeroの較正の方式で決め、`--resign-threshold`で渡す。
//! 対局番号1、11、21…は投了禁止とし、両者の値の最小値を較正の材料として集計に残す。
//! `docs/adr/calibrate-the-zero-self-play-resignation-threshold-as-aobazero-does-and-record-the-gumbel-choice-value.md`を参照。
//!
//! `--threads`個の各スレッドは推論器を1つと最大`--games-per-thread`局を持つ。
//! `--devices`はGPUの枚数であり、スレッド数はその倍数とする。スレッドtは機器t mod Nを使う。
//! CPUでは機器数1に限り、GPUは照会しない。CUDAとTensorRTではGPUの枚数、機種、容量を検査する。
//! CUDAとTensorRTでは`CUDA_VISIBLE_DEVICES`の設定を拒み、TensorRTでは非空の`XDG_CACHE_HOME`を必要とする。
//! 各局から推論を待つ葉を1つずつ集めてバッチを作り、仮想損失は使わない。
//! `docs/adr/batch-zero-self-play-leaves-across-many-games-with-a-fresh-tree-per-move.md`を参照。
//!
//! 生成開始時に出力先を排他的にロックし、`config.json`で生成条件を保存または照合する。
//! `--games`を上限として`--chunk-games`局ずつを塊にし、対局番号順に1つのMNSDへ書く。
//! 塊0からの訓練局面数の累計が初めて`--target-positions`以上になる塊までをサイクルに含める。
//! 全塊で目標に届かなければ全塊を含め、範囲外の確定済み塊は保持する。
//! 範囲が決まればワーカーを停止して回収し、書きかけの塊を削除する。
//! 再開時は確定済みの集計を使い、保存済みの塊だけで範囲が決まれば推論器を作らない。
//! `docs/adr/commit-zero-self-play-in-fixed-game-chunks-and-close-the-cycle-at-the-target-positions.md`を参照。

mod cli;
mod error;
mod game;
mod output;
mod worker;

pub(super) use cli::Arguments;
use error::Error;
use minase::nn::tensorrt_cache::{query_gpus, validate_cuda_visible_devices};
use minase::nn::{Network, OnnxEvaluator};
use minase_core::Rules;
use std::num::NonZeroUsize;

/// 自己対局には規則セット`lishogi`を使う。
/// `docs/adr/play-self-play-and-judge-trained-networks-under-the-lishogi-rule-set.md`を参照。
const RULES: Rules = Rules::LISHOGI;

/// 手数の上限3,000手。到達しても未終局なら引き分けとして記録する。
const MAX_PLY: u32 = 3_000;

/// 自己対局のQの尺度。Gumbel AlphaZero論文の囲碁とチェスに合わせて1.0を使う。
const GUMBEL_VALUE_SCALE: f32 = 1.0;

/// 1局の探索木の容量16 MiB。容量不足はエラーにする。
/// `docs/adr/batch-zero-self-play-leaves-across-many-games-with-a-fresh-tree-per-move.md`を参照。
const TREE_CAPACITY: usize = 16 * 1024 * 1024;

/// コマンドを実行し、失敗の説明を標準エラーへ出して非0終了する。
pub(super) fn main(arguments: Arguments) {
    if let Err(error) = run(arguments) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: Arguments) -> Result<(), Error> {
    arguments.validate_execution()?;
    if !matches!(arguments.nn_backend, cli::Backend::Cpu) {
        validate_cuda_visible_devices(std::env::var_os("CUDA_VISIBLE_DEVICES").as_deref())?;
        validate_gpu_count(arguments.devices, query_gpus()?.count)?;
    }
    let network = Network::read(&arguments.nn_model)?;
    let provider = arguments.nn_backend.provider(0)?;
    let config = output::Config {
        rules: RULES.to_string(),
        generation_commit: arguments.generation_commit,
        network_sha256: minase::datagen::hex(network.sha256()),
        nn_backend: provider.name().to_owned(),
        seed: arguments.seed,
        start_position: arguments.start_position,
        games: arguments.games,
        chunk_games: arguments.chunk_games,
        simulations: arguments.simulations,
        resign_threshold: arguments.resign_threshold,
        max_ply: MAX_PLY,
    };
    let report = output::generate(
        &arguments.output_dir,
        &config,
        arguments.target_positions,
        network.sha256(),
        arguments.threads,
        arguments.games_per_thread,
        |thread| {
            let provider = arguments
                .nn_backend
                .provider(device_for_thread(thread, arguments.devices))?;
            Ok(Box::new(OnnxEvaluator::new(
                &network,
                &provider,
                arguments.games_per_thread,
            )?))
        },
    )?;
    report.print(&config)
}

/// 照会したGPUの枚数が、使用する枚数の指定と一致することを検査する。
fn validate_gpu_count(requested: NonZeroUsize, available: NonZeroUsize) -> Result<(), Error> {
    if requested != available {
        return Err(Error::DeviceCountMismatch {
            requested,
            available,
        });
    }
    Ok(())
}

/// 0から数えたスレッド番号を、使用する機器の枚数を法とする剰余で機器番号へ対応させる。
fn device_for_thread(thread: usize, devices: NonZeroUsize) -> usize {
    thread % devices
}

#[cfg(test)]
mod tests;

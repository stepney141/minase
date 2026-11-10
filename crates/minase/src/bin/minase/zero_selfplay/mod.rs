//! Gumbel AlphaZeroによる自己対局を生成し、教師データ形式MNSDの版2で保存する。
//! 規則は`engine-default`を使い、対局の進行と規則上の終局の裁定は[`minase_core::Game`]が担う。
//! 探索した手を指しても未終局なら、次の手番側の王手限定の3手以内の勝ちを調べ、勝ちがあれば打ち切る。
//! この打ち切りはMNSDの終局理由12で記録し、投了させない対局にも適用する。
//! `--games`を対局数の上限とし、`--chunk-games`局ずつの塊として確定させる。
//! 塊0から連続する確定済み塊の訓練局面数が、初めて`--target-positions`以上になる塊までをサイクルに含める。
//! 目標の判定には投了ファイルの除外前の局面数を使い、全塊でも目標に届かない場合は全塊を含める。
//!
//! 起動引数はすべて必須で、既定値を持たない。
//! `--output-dir`は出力先、`--nn-model`は評価するONNX形式のネット、
//! `--nn-backend`は推論の実行プロバイダで、`tensorrt`、`cuda`、`cpu`から選ぶ。
//! `--generation-commit`は生成器の40桁のコミットハッシュを受け取り、`git`は呼び出さない。
//! `--start-position`は`shuffled-setup`（組み替えた配置から探索）か、
//! `random-opening`（標準の初期局面から8〜16手を乱択）を選ぶ。
//! 開始局面用の乱数は対局のシードから派生番号0で作り、対局記録には各方式の開始局面を手数0で保存する。
//! `--seed`はサイクルの基本シード、`--simulations`は完全探索の回数nを表し、4以上の4の倍数とする。
//! 探索する手番ごとに確率0.25で完全探索、それ以外は回数n/4の速い探索を行う。
//! 価値と方策の両方を持つ完全探索の項目を訓練局面とし、速い探索は指した手、価値、訪問回数だけを記録する。
//! 投了の判定と較正は完全探索の手番だけで行う。
//! `--playthrough-percent`は投了させずに最後まで指させる割合を1〜100の整数で指定し、100で投了を無効にする。
//! 投了の有効化や再較正を行うサイクルの選択は、生成器を呼ぶ側が担う。
//!
//! `--threads`個の各スレッドは、推論器を1つと、最大`--games-per-thread`局を持つ。
//! `--devices <N>`は使用するGPUの枚数で、`--threads`はNの倍数でなければならない。
//! 0から数えたスレッドtの推論器を、CUDAの機器番号t mod Nに置く。
//! `--nn-backend cpu`ではN=1に限り、GPUは照会しない。
//! TensorRTとCUDAでは、照会したGPUの枚数がNと等しく、全GPUの機種名とメモリ容量が同じでなければ起動を拒む。
//! TensorRTとCUDAでは、`CUDA_VISIBLE_DEVICES`が設定されていれば空値でも起動を拒む。
//! 推論器はONNX Runtimeのセッションを所有し、スレッドの起動前に1つずつ作る。
//! 各局を評価待ちまで進め、そのスレッドで待っている局から葉を1つずつ集めて推論する。
//! バッチの大きさは評価を待つ局の数であり、推論器の最大バッチは`--games-per-thread`とする。
//! 1つの木で評価を待つ葉は高々1つで、仮想損失は使わない。
//! TensorRTのキャッシュは`$XDG_CACHE_HOME/minase/tensorrt`へ置き、`XDG_CACHE_HOME`の未設定や空値はエラーにする。
//! この構成は`docs/adr/batch-zero-self-play-leaves-across-many-games.md`を参照。
//!
//! 生成開始時に出力先を排他的にロックし、生成条件を保存するか、保存済みの条件と照合する。
//! 確定済みの塊だけでサイクルの範囲が決まれば、推論器を作らずに集計を書く。
//! 決まらなければ未確定の塊の対局を番号順にスレッドへ配る。
//! 指定した開始方式で対局を準備し、探索する手番ごとに新しい木を作る。
//! 終えた対局は塊の中で対局番号順に書き、投了で終えた対局は別のファイルに保存する。
//! 塊の確定でサイクルの範囲が決まったらワーカーを停止して回収し、書きかけの塊の一時ファイルを削除する。
//! 含める塊の対局だけから誤った投了の割合、サイクルの集計、および学習ファイルの一覧を求める。
//! 範囲より後の確定済みの塊はファイルを保持するが、集計と学習には含めない。
//! 保存と再開は`docs/adr/commit-zero-self-play-in-fixed-game-chunks-up-to-the-first-prefix-reaching-the-target-positions.md`、
//! 投了の扱いは`docs/adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md`を参照。

mod cli;
mod error;
mod game;
mod output;
mod worker;

pub(super) use cli::Arguments;
use error::Error;
use minase::nn::tensorrt_cache::{query_gpus, validate_cuda_visible_devices};
use minase::nn::{Network, OnnxEvaluator};
use minase::training::records;
use minase_core::{Rules, rules::parse_rule_set};
use std::num::NonZeroUsize;

/// 序盤を含む手数の上限3,000手。到達しても未終局なら対局全体を破棄する。
/// `docs/adr/discard-zero-self-play-games-that-reach-3000-plies.md`を参照。
const MAX_PLY: u32 = 3_000;

/// 投了の閾値1%。選んだ手が訪問済みで、そのQが厳密に下回る場合に使う。
/// `docs/adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md`を参照。
const RESIGN_THRESHOLD: f32 = 0.01;

/// 自己対局のQの尺度。Gumbel AlphaZero論文の囲碁とチェスに合わせて1.0を使う。
const GUMBEL_VALUE_SCALE: f32 = 1.0;

/// 誤った投了の割合の上限5%。厳密に超えたサイクルの投了ファイルを学習の対象から除く。
/// `docs/adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md`を参照。
const FALSE_RESIGNATION_LIMIT: f64 = 0.05;

/// 1局の探索木の容量16 MiB。容量不足はエラーにする。
/// `docs/adr/batch-zero-self-play-leaves-across-many-games.md`を参照。
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
    let rules = engine_default_rules()?;
    let config = output::Config {
        rules: rules.to_string(),
        generation_commit: arguments.generation_commit,
        network_sha256: minase::datagen::hex(network.sha256()),
        nn_backend: provider.name().to_owned(),
        seed: arguments.seed,
        start_position: arguments.start_position,
        games: arguments.games,
        chunk_games: arguments.chunk_games,
        simulations: arguments.simulations,
        playthrough_percent: arguments.playthrough_percent,
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
    thread % devices.get()
}

/// 生成器が使う規則を、公開の規則解析APIから構築する。
fn engine_default_rules() -> Result<Rules, Error> {
    let codes = parse_rule_set("engine-default").map_err(records::Error::RuleSetParse)?;
    Ok(Rules::from_codes(&codes).map_err(records::Error::Rules)?)
}

#[cfg(test)]
mod tests;

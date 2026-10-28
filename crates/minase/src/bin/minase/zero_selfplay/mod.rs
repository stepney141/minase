//! Gumbel AlphaZeroによる自己対局を生成し、教師データ形式MNSDの版2で保存する。
//! 規則は`engine-default`を使い、対局の進行と終局の裁定は[`minase_core::Game`]が担う。
//! 1サイクルの生成量は`--games`で対局数を与え、`--chunk-games`局ずつの塊として確定させる。
//!
//! 起動引数はすべて必須で、既定値を持たない。
//! `--output-dir`は出力先、`--nn-model`は評価するONNX形式のネット、
//! `--nn-backend`は推論の実行プロバイダで、`tensorrt`、`cuda`、`cpu`から選ぶ。
//! `--generation-commit`は生成器の40桁のコミットハッシュを受け取り、`git`は呼び出さない。
//! `--seed`はサイクルの基本シード、`--simulations`は根の評価後に行う1手の探索回数を表す。
//! `--playthrough-percent`は投了させずに最後まで指させる割合を1〜100の整数で指定し、100で投了を無効にする。
//! 投了の有効化や再較正を行うサイクルの選択は、生成器を呼ぶ側が担う。
//!
//! `--threads`個の各スレッドは、推論器を1つと、最大`--games-per-thread`局を持つ。
//! 推論器はONNX Runtimeのセッションを所有し、スレッドの起動前に1つずつ作る。
//! 各局を評価待ちまで進め、そのスレッドで待っている局から葉を1つずつ集めて推論する。
//! バッチの大きさは評価を待つ局の数であり、推論器の最大バッチは`--games-per-thread`とする。
//! 1つの木で評価を待つ葉は高々1つで、仮想損失は使わない。
//! TensorRTのキャッシュは`$XDG_CACHE_HOME/minase/tensorrt`へ置き、`XDG_CACHE_HOME`の未設定や空値はエラーにする。
//! この構成は`docs/adr/batch-zero-self-play-leaves-across-many-games.md`を参照。
//!
//! 生成開始時に出力先を排他的にロックし、生成条件を保存するか、保存済みの条件と照合する。
//! 未確定の塊の対局だけをスレッドへ配り、序盤の乱択後は手ごとに新しい木で探索する。
//! 終えた対局は塊の中で対局番号順に書き、投了で終えた対局は別のファイルに保存する。
//! 全塊の確定後に、最後まで指させた対局から誤った投了の割合を求め、サイクルの集計と学習ファイルの一覧を書く。
//! 保存と再開は`docs/adr/commit-zero-self-play-cycles-in-fixed-game-chunks.md`、
//! 投了の扱いは`docs/adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md`を参照。

mod cli;
mod error;
mod game;
mod output;
mod worker;

pub(super) use cli::Arguments;
use error::Error;
use minase::nn::{Network, OnnxEvaluator};
use minase::training::records;
use minase_core::{Rules, rules::parse_rule_set};

/// 序盤を含む手数の上限1,000手。到達しても未終局なら対局全体を破棄する。
/// `docs/adr/discard-zero-self-play-games-that-reach-1000-plies.md`を参照。
const MAX_PLY: u32 = 1_000;

/// 投了の閾値1%。選んだ手が訪問済みで、そのQが厳密に下回る場合に使う。
/// `docs/adr/resign-zero-self-play-games-below-one-percent-after-bounding-false-resignations.md`を参照。
const RESIGN_THRESHOLD: f32 = 0.01;

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
    let network = Network::read(&arguments.nn_model)?;
    let provider = arguments.nn_backend.provider()?;
    let rules = engine_default_rules()?;
    let config = output::Config {
        rules: rules.to_string(),
        generation_commit: arguments.generation_commit,
        network_sha256: minase::datagen::hex(network.sha256()),
        nn_backend: provider.name().to_owned(),
        seed: arguments.seed,
        games: arguments.games,
        chunk_games: arguments.chunk_games,
        simulations: arguments.simulations,
        playthrough_percent: arguments.playthrough_percent,
        max_ply: MAX_PLY,
    };
    let report = output::generate(
        &arguments.output_dir,
        &config,
        network.sha256(),
        arguments.threads,
        arguments.games_per_thread,
        || {
            Ok(Box::new(OnnxEvaluator::new(
                &network,
                &provider,
                arguments.games_per_thread,
            )?))
        },
    )?;
    report.print(&config)
}

/// 生成器が使う規則を、公開の規則解析APIから構築する。
fn engine_default_rules() -> Result<Rules, Error> {
    let codes = parse_rule_set("engine-default").map_err(records::Error::RuleSetParse)?;
    Ok(Rules::from_codes(&codes).map_err(records::Error::Rules)?)
}

#[cfg(test)]
mod tests;

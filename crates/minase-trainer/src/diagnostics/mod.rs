//! NNUEの分布と局面操作、およびPSTの比較をCPUで診断する。
//! `diag nnue-distribution`、`diag nnue-removal`、`diag nnue-moves`はCPUだけで実行し、
//! 報告を`--output`に指定したJSONファイル1つへ保存する。
//! 既存出力の扱いと失敗時の保存条件は[`run`]に定める。

mod network;
mod nnue_distribution;
mod nnue_moves;
mod nnue_removal;
mod pst_compare;
mod pst_positions;
mod pst_reference;
mod pst_sample;
mod pst_statistics;
mod sample;
mod statistics;

use std::{fs, io::Write, path::Path};

use clap::Subcommand;
use serde::Serialize;

pub use nnue_distribution::DistributionArgs;
pub use nnue_moves::MovesArgs;
pub use nnue_removal::RemovalArgs;

pub use pst_compare::PstCompareArgs;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// CPUだけで実行する評価関数の診断。
#[derive(Subcommand)]
pub enum Command {
    /// 教師分類・位相帯の損失、整数一致、除去と成りでPSTを比較する。
    PstCompare(PstCompareArgs),
    /// 全局面の標本でMNPT版3とMNUE版5の評価分布を比較する。
    NnueDistribution(DistributionArgs),
    /// 検証集合で実数と整数の駒除去差分を検査する。
    NnueRemoval(RemovalArgs),
    /// 全合法手の評価差をNNUE、PST＋FM、PSTのみで比較する。
    NnueMoves(MovesArgs),
}

/// 出力の存在を検査して診断を実行し、報告を保存する。
///
/// NNUE診断では`--output`が既に存在すれば、何も計算せずにエラーを返す。
/// 保存時も`create_new`で開くため、検査後に別プロセスが同じパスを作った場合に上書きしない。
/// 親ディレクトリがなければ作り、`serde_json`の整形出力に改行1つを付けて保存する。
/// `nnue-removal`と`nnue-moves`で報告に`error`欄があれば、保存後にそのエラー文を返す。
/// 標本の範囲外、帯の標本不足、分散0、非有限値、入力形式の不一致など、
/// 報告を組み立てる前のエラーでは報告ファイルを作らない。
/// エラーを返すと`minase-trainer`は終了コード1で終わる。
/// PST比較のディレクトリ作成と途中失敗の扱いは`pst_compare::run`に定める。
pub fn run(command: Command) -> Result<()> {
    let path = match &command {
        Command::PstCompare(args) => &args.output_dir,
        Command::NnueDistribution(args) => &args.output,
        Command::NnueRemoval(args) => &args.output,
        Command::NnueMoves(args) => &args.output,
    };
    if path.try_exists()? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{}: diagnostic output already exists", path.display()),
        )
        .into());
    }
    match command {
        Command::PstCompare(args) => pst_compare::run(&args),
        Command::NnueDistribution(args) => {
            let report = nnue_distribution::run(&args)?;
            save(&args.output, &report, None)
        }
        Command::NnueRemoval(args) => {
            let report = nnue_removal::run(&args)?;
            save(&args.output, &report, report.error.as_deref())
        }
        Command::NnueMoves(args) => {
            let report = nnue_moves::run(&args)?;
            save(&args.output, &report, report.error.as_deref())
        }
    }
}

fn save(path: &Path, report: &impl Serialize, error: Option<&str>) -> Result<()> {
    let mut json = serde_json::to_vec_pretty(report)?;
    json.push(b'\n');
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    // 検査後に別プロセスが作成した場合も上書きしない。
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&json)?;
    file.flush()?;
    if let Some(error) = error {
        return Err(error.into());
    }
    Ok(())
}

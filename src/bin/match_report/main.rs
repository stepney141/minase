//! 保存済み対局から時間制御校正の指標を再計算する。

mod compare;
mod report;
mod run_dir;

use clap::Parser;
use compare::compare;
use report::report;
use std::path::PathBuf;

/// 校正指標集計器のコマンドライン引数。
#[derive(Parser)]
#[command(name = "match_report")]
struct Arguments {
    /// 集計するmatch_runner実行ディレクトリ。
    #[arg(long)]
    run_dir: PathBuf,
    /// 第1、第2主指標の比較基準とする実行ディレクトリ。
    #[arg(long)]
    compare_to: Option<PathBuf>,
}

fn main() {
    let arguments = Arguments::parse();
    let run_report = report(&arguments.run_dir).unwrap_or_else(|error| {
        eprintln!("failed to report {}: {error}", arguments.run_dir.display());
        std::process::exit(1);
    });
    let comparison = arguments.compare_to.as_deref().map(|path| {
        let current = report(path).unwrap_or_else(|error| {
            eprintln!(
                "failed to report comparison run {}: {error}",
                path.display()
            );
            std::process::exit(1);
        });
        compare(&run_report, &current).unwrap_or_else(|error| {
            eprintln!("failed to compare calibration metrics: {error}");
            std::process::exit(1);
        })
    });
    let output = serde_json::json!({
        "run": run_report,
        "comparison_to_current": comparison,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&output).expect("finite report values serialize as JSON")
    );
}

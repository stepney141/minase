//! 評価関数の学習に使う自己対局データの生成と検査。

mod cli;
mod generate;
mod inspect;
mod opening;
mod play;
mod provenance;
mod rescore;
mod statistics;

use clap::Parser;
use cli::{Arguments, Operation};
use generate::generate;
use inspect::inspect;
use minase::{Rules, core::rules::parse_rule_set, datagen::invalid_data};
use provenance::write_provenance;
use rescore::rescore;
use std::{io, num::NonZeroUsize, process};

/// グローバルアロケータ。探索を行う既存バイナリと同じくmimallocを使う。
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// 教師探索の既定ノード上限。
const DEFAULT_NODES: u32 = 100_000;

/// 1局の既定手数上限。
const DEFAULT_MAX_PLY: u16 = 4_000;

/// ワーカーごとの既定置換表容量。
const DEFAULT_HASH_MB: NonZeroUsize = NonZeroUsize::new(16).unwrap();

/// ランダム着手を配置する序盤終了後の手数幅。
const INJECTION_WINDOW: usize = 80;

/// コマンドを実行し、失敗時は説明を標準エラーへ出して非0終了する。
fn main() {
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 解析したサブコマンドを実行する。
fn run() -> io::Result<()> {
    match Arguments::parse().command {
        Operation::Generate(arguments) => generate(&arguments),
        Operation::Inspect(arguments) => inspect(&arguments),
        Operation::Rescore(arguments) => rescore(&arguments),
        Operation::Provenance(arguments) => write_provenance(&arguments),
    }
}

/// `engine-default`を公開規則解析APIから構築する。
fn engine_default_rules() -> io::Result<Rules> {
    let codes =
        parse_rule_set("engine-default").map_err(|error| invalid_data(error.to_string()))?;
    Rules::from_codes(&codes).map_err(|error| invalid_data(error.to_string()))
}

#[cfg(test)]
mod test_support {
    use crate::cli::RescoreArguments;
    use minase::{
        Position,
        search::TranspositionTable,
        training::records::{Header, Outcome, Record, Writer},
    };
    use std::{
        fs::{self, File},
        num::NonZeroUsize,
        path::PathBuf,
        process,
        sync::atomic::{AtomicU64, Ordering},
    };

    /// 小さなMNSDを含む一時ディレクトリ。各テストの入出力を分離する。
    pub(super) struct RescoreFixture {
        pub(super) directory: PathBuf,
        pub(super) arguments: RescoreArguments,
    }

    impl RescoreFixture {
        pub(super) fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "minase-rescore-{}-{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            let input = directory.join("input.mnsd");
            let targets = directory.join("targets.txt");
            fs::write(&targets, b"0\n2\n4\n").unwrap();
            let header = Header::new(
                "engine-default".into(),
                "a".repeat(40),
                [7; 32],
                100_000,
                42,
                0,
            )
            .unwrap();
            let mut writer = Writer::new(File::create(&input).unwrap(), header).unwrap();
            // 同じ局面を別の記録番号にも置き、以前の探索結果からの独立性を検証する。
            for index in 0..5 {
                writer
                    .write_record(&Record::from_position(
                        &Position::initial(),
                        17,
                        Outcome::Draw,
                        1,
                        index,
                    ))
                    .unwrap();
            }
            writer.finish().unwrap();
            let arguments = RescoreArguments {
                input,
                targets,
                output: directory.join("output.mnrs"),
                pst: None,
                nodes: 200,
                hash_mb: NonZeroUsize::new(1).unwrap(),
                concurrency: NonZeroUsize::new(1).unwrap(),
                allow_dirty: true,
            };
            Self {
                directory,
                arguments,
            }
        }
    }

    impl Drop for RescoreFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    /// テスト用の小さい置換表を作る。
    pub(super) fn test_table() -> TranspositionTable {
        TranspositionTable::new(1).expect("one MiB is a valid table size")
    }
}

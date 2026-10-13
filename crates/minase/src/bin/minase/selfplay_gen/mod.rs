//! 評価関数の学習に使う自己対局データの生成と検査。

mod cli;
mod generate;
mod inspect;
mod opening;
mod play;
mod statistics;

pub(super) use cli::Arguments;
use cli::Operation;
use generate::generate;
use inspect::inspect;
use minase::datagen::invalid_data;
use minase_core::{Rules, rules::parse_rule_set};
use std::{io, num::NonZeroUsize, process};

/// 教師探索の既定ノード上限100,000。
/// 1局面の費用をほぼ一定にして生成量を見積もるため、深さではなくノード数で打ち切る。
/// 教師データの生成計画ごとに`--nodes`で明示して変更できる。
/// 理由は`docs/adr/use-fixed-node-searches-for-teacher-values.md`にある。
const DEFAULT_NODES: u32 = 100_000;

/// 1局の既定手数上限4,000手。
/// 上限4,096手の試行から100手刻みの候補を比べ、上限以下で終局した対局の記録局面数を全対局がその上限までに探索した局面数で割った比が最大の候補を選んだ。
/// 理由は`docs/adr/cap-teacher-games-at-4000-plies.md`にある。
const DEFAULT_MAX_PLY: u16 = 4_000;

/// ワーカーごとの既定置換表容量。
const DEFAULT_HASH_MB: NonZeroUsize = NonZeroUsize::new(16).unwrap();

/// 序盤終了手数pから注入を予定する半開区間`[p, p+80)`の幅80手。
/// 注入回数と記録境界は[`play`]の`plan_injections_with_rng`に従う。
const INJECTION_WINDOW: usize = 80;

/// コマンドを実行し、失敗時は説明を標準エラーへ出して非0終了する。
pub(super) fn main(arguments: Arguments) {
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = run(arguments) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 解析したサブコマンドを実行する。
fn run(arguments: Arguments) -> io::Result<()> {
    match arguments.command {
        Operation::Generate(arguments) => generate(&arguments),
        Operation::Inspect(arguments) => inspect(&arguments),
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
    use minase::search::TranspositionTable;
    pub(super) fn test_table() -> TranspositionTable {
        TranspositionTable::new(1).expect("one MiB is a valid table size")
    }
}

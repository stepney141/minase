//! MNPT重みファイルでMNSD局面を評価し、合法な成り手による評価変化を列挙する。
//!
//! PST学習の診断（`tools/train/pst/pst_diagnostics.py`）から呼ばれ、Pythonの整数参照評価と
//! Rustの評価の一致確認、および代表局面の成りの診断に使う。出力はJSON配列であり、
//! 各要素は入力レコードの順に`index`、手番側視点の`eval`、および成り手ごとの
//! 着手前の手番側視点で測った評価差`delta`とMNSD表現の着手後局面`after`を持つ。

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::eval::Pst;
use minase::eval::pst::evaluate;
use minase::eval::training_data::{Outcome, Reader, Record};
use minase::notation::usi;
use minase::{BOARD_SQUARE_COUNT, Game, MoveGenerator, Rules};
use serde::Serialize;

/// コマンドライン引数。
#[derive(Parser)]
#[command(name = "pst_probe")]
struct Arguments {
    /// 評価に使うMNPT重みファイル。
    #[arg(long)]
    pst: PathBuf,
    /// 評価する局面を収めたMNSDファイル。
    #[arg(long)]
    positions: PathBuf,
    /// 各局面の合法な成り手を実際に適用して評価差を列挙する。
    #[arg(long)]
    promotions: bool,
}

/// 1つの成り手の評価差。
#[derive(Serialize)]
struct Promotion {
    /// USI表記の着手。
    r#move: String,
    /// 着手前の手番側視点で測った評価差（着手後の評価の符号を反転して差し引く）。
    delta: i32,
    /// Pythonが評価差を独立に計算するための着手後局面。
    after: AfterPosition,
}

/// PST評価に必要な局面をMNSDと同じ盤面・手番・先獅子コードで表す。
#[derive(Serialize)]
struct AfterPosition {
    /// dense index順のMNSD盤面コード。
    board: Vec<u8>,
    /// 手番側（先手0、後手1）。
    stm: u8,
    /// 先獅子の対象升（存在しない場合は255）。
    lion: u8,
}

/// 1局面の評価結果。
#[derive(Serialize)]
struct Probe {
    /// 入力レコードの通し番号。
    index: usize,
    /// 手番側視点の静的評価。
    eval: i32,
    /// `--promotions`指定時の成り手ごとの評価差。
    #[serde(skip_serializing_if = "Option::is_none")]
    promotions: Option<Vec<Promotion>>,
}

/// 局面を読み、評価と成り手の評価差を計算する。
fn run(arguments: &Arguments) -> Result<Vec<Probe>, String> {
    let bytes = std::fs::read(&arguments.pst).map_err(|error| error.to_string())?;
    let pst = Pst::decode(&bytes).map_err(|error| error.to_string())?;
    let codes = parse_rule_set("engine-default").map_err(|error| error.to_string())?;
    let rules = Rules::from_codes(&codes).map_err(|error| error.to_string())?;
    let generator = MoveGenerator::new(rules.moves);
    let file = File::open(&arguments.positions).map_err(|error| error.to_string())?;
    let mut reader = Reader::new(BufReader::new(file)).map_err(|error| error.to_string())?;
    let mut probes = Vec::new();
    let mut index = 0;
    while let Some(record) = reader.read_record().map_err(|error| error.to_string())? {
        let position = record.to_position().map_err(|error| error.to_string())?;
        let before = evaluate(&pst, &position);
        let promotions = arguments.promotions.then(|| {
            let game = Game::from_position(rules, position.clone());
            game.legal_moves()
                .into_iter()
                .filter(|mv| mv.promote)
                .map(|mv| {
                    let text = usi::text(&position, mv, &generator)
                        .expect("legal move must have a USI text");
                    let mut after = game.clone();
                    after.play(mv).expect("legal move must be playable");
                    let encoded =
                        Record::from_position(after.position(), 0, Outcome::Draw, 1, 0).encode();
                    Promotion {
                        r#move: text,
                        delta: -evaluate(&pst, after.position()) - before,
                        after: AfterPosition {
                            board: encoded[..BOARD_SQUARE_COUNT].to_vec(),
                            stm: encoded[144],
                            lion: encoded[145],
                        },
                    }
                })
                .collect()
        });
        probes.push(Probe {
            index,
            eval: before,
            promotions,
        });
        index += 1;
    }
    Ok(probes)
}

/// 引数を解析して結果をJSONで出力する。
fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(&arguments) {
        Ok(probes) => {
            let json = serde_json::to_string(&probes).expect("probe results serialize");
            let mut stdout = io::stdout().lock();
            if writeln!(stdout, "{json}").is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

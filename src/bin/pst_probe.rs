//! MNPT重みファイルでMNSD局面を評価し、合法な成り手による評価変化を列挙する。
//!
//! PST学習の診断（`tools/train/pst/pst_diagnostics.py`）から呼ばれ、Pythonの整数参照評価と
//! Rustの評価の一致確認、および代表局面の成りの診断に使う。出力はJSON配列であり、
//! 各要素は入力レコードの順に`index`、追加特徴を含む`eval`、学習PSTだけの`pst_eval`、
//! 24列の`king_features`、および成り手ごとの
//! 着手前の手番側視点で測った評価差`delta`とMNSD表現の着手後局面`after`を持つ。
//! `after`にも着手後の手番側視点の同じ評価情報を含む。
//! `--king-features`指定時は、評価の代わりに王の安全度の特徴をMNKF形式で逐次書く。

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::eval::pst::{evaluate, evaluate_pst};
use minase::eval::training_data::{Outcome, Reader, Record, write_king_features};
use minase::eval::{Pst, king_feature_values};
use minase::notation::usi;
use minase::{BOARD_SQUARE_COUNT, Game, MoveGenerator, Position, Rules};
use serde::Serialize;

/// コマンドライン引数。
#[derive(Parser)]
#[command(name = "pst_probe")]
struct Arguments {
    /// 評価に使うMNPT重みファイル。--king-featuresでは読み込まない。
    #[arg(long)]
    pst: PathBuf,
    /// 評価する局面を収めたMNSDファイル。
    #[arg(long)]
    positions: PathBuf,
    /// 各局面の合法な成り手を実際に適用して評価差を列挙する。
    #[arg(long)]
    promotions: bool,
    /// 王の安全度の24特徴をMNKF形式で書く新規ファイル。JSON評価の出力に代わる。
    #[arg(long, conflicts_with = "promotions")]
    king_features: Option<PathBuf>,
    /// 復元できないレコードを理由つきで出力し、残りの評価を続ける。
    #[arg(long, conflicts_with = "king_features")]
    skip_invalid: bool,
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
    /// 着手後の手番側視点の評価と追加特徴。
    #[serde(flatten)]
    evaluation: Evaluation,
}

/// 1局面の評価結果。
#[derive(Serialize)]
struct Probe {
    /// 入力レコードの通し番号。
    index: usize,
    /// 手番側視点の評価と追加特徴。
    #[serde(flatten)]
    evaluation: Option<Evaluation>,
    /// 復元できなかった理由。評価を行ったレコードでは省略する。
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<String>,
    /// `--promotions`指定時の成り手ごとの評価差。
    #[serde(skip_serializing_if = "Option::is_none")]
    promotions: Option<Vec<Promotion>>,
}

/// 補間後の評価値と、補間前に使う追加特徴の値。
#[derive(Serialize)]
struct Evaluation {
    /// 追加特徴を含む手番側視点の静的評価。
    eval: i32,
    /// 学習PSTだけの手番側視点の静的評価。
    pst_eval: i32,
    /// 設計書の列順に並べた24個の追加特徴。
    king_features: Vec<u8>,
}

impl Evaluation {
    fn new(pst: &Pst, position: &Position) -> Self {
        Self {
            eval: evaluate(pst, position),
            pst_eval: evaluate_pst(pst, position),
            king_features: king_feature_values(position, position.side_to_move()).to_vec(),
        }
    }
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
        let position = match record.to_position() {
            Ok(position) => position,
            Err(error) if arguments.skip_invalid => {
                probes.push(Probe {
                    index,
                    evaluation: None,
                    skipped: Some(error.to_string()),
                    promotions: None,
                });
                index += 1;
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let before = Evaluation::new(&pst, &position);
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
                        delta: -evaluate(&pst, after.position()) - before.eval,
                        after: AfterPosition {
                            board: encoded[..BOARD_SQUARE_COUNT].to_vec(),
                            stm: encoded[144],
                            lion: encoded[145],
                            evaluation: Evaluation::new(&pst, after.position()),
                        },
                    }
                })
                .collect()
        });
        probes.push(Probe {
            index,
            evaluation: Some(before),
            skipped: None,
            promotions,
        });
        index += 1;
    }
    if arguments.skip_invalid {
        let skipped = probes
            .iter()
            .filter(|probe| probe.skipped.is_some())
            .count();
        eprintln!("skipped_records={skipped}");
    }
    Ok(probes)
}

/// 引数を解析して結果をJSONで出力する。
fn main() -> ExitCode {
    let arguments = Arguments::parse();
    if let Some(path) = &arguments.king_features {
        let result = (|| {
            let input = BufReader::new(File::open(&arguments.positions)?);
            // 入力と同じファイルや既存の診断結果を切り詰めない。
            let output = BufWriter::new(File::create_new(path)?);
            write_king_features(input, output)
        })();
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }
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

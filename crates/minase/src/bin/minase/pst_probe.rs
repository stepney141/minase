//! MNPT重みファイルでMNSD局面を評価し、合法手による評価変化を列挙する。
//!
//! PST学習の診断（`tools/train/src/minase_train/diagnostics/comparison.py`）から呼ばれ、Pythonの整数参照評価と
//! Rustの評価の一致確認、および代表局面の成りの診断に使う。出力はJSON配列であり、
//! 各要素は入力順に`index`、FM込みの`eval`、PSTだけの`eval_pst`を持つ。
//! `--moves`または`--promotions`で、合法手の評価差と着手後のMNSD表現を加える。

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use minase::eval::Pst;
use minase::eval::pst::evaluate_for_diagnostics;
use minase::training::records::{Outcome, Reader, Record};
use minase_core::notation::usi;
use minase_core::rules::parse_rule_set;
use minase_core::{BOARD_SQUARE_COUNT, Game, MoveGenerator, Rules};
use serde::Serialize;

/// コマンドライン引数。
#[derive(Parser)]
#[command(name = "pst_probe")]
pub(super) struct Arguments {
    /// 評価に使うMNPT重みファイル。
    #[arg(long)]
    pst: PathBuf,
    /// 評価する局面を収めたMNSDファイル。
    #[arg(long)]
    positions: PathBuf,
    /// 各局面の合法な成り手を実際に適用して評価差を列挙する。
    #[arg(long)]
    promotions: bool,
    /// 各局面の全合法手を適用して評価差を列挙する。
    #[arg(long)]
    moves: bool,
    /// 復元できないレコードを理由つきで出力し、残りの評価を続ける。
    #[arg(long)]
    skip_invalid: bool,
}

/// 1つの合法手の評価差。
#[derive(Serialize)]
struct MoveDelta {
    /// USI表記の着手。
    r#move: String,
    /// 盤上の駒数が減る捕獲手。
    capture: bool,
    /// 成りを選んだ手。
    promote: bool,
    /// 入力局面から新たに開始した対局で、この手により終局したか。
    terminal: bool,
    /// 着手前の手番側視点で測った評価差（着手後の評価の符号を反転して差し引く）。
    delta: i32,
    /// FMの補正を含まないPSTだけの評価差。
    delta_pst: i32,
    /// 着手後の配置を着手前の手番側から見た評価。
    after_same: i32,
    /// FMの補正を含まない対応するPST評価。
    after_same_pst: i32,
    /// 着手後の配置を着手後の手番側から見た評価。
    after_opposite: i32,
    /// FMの補正を含まない対応するPST評価。
    after_opposite_pst: i32,
    /// 視点を固定した配置変化の成分。
    placement: i32,
    /// FMの補正を含まない対応するPST評価。
    placement_pst: i32,
    /// 着手後の同一配置における手番交代の成分。
    turn: i32,
    /// FMの補正を含まない対応するPST評価。
    turn_pst: i32,
    /// 学習器との照合に使う着手後局面。
    after: AfterPosition,
}

/// F_t(B') - F_t(B) と -(F_opp(B') + F_t(B')) への分解。
/// その和は探索が用いる -F_opp(B') - F_t(B) に等しい。
fn components(before: i32, after_same: i32, after_opposite: i32) -> (i32, i32, i32) {
    (
        -after_opposite - before,
        after_same - before,
        -(after_opposite + after_same),
    )
}

impl MoveDelta {
    /// 両視点の評価から合法手の寄与を計算する。
    #[allow(clippy::too_many_arguments)]
    fn new(
        text: String,
        before: (i32, i32),
        same: (i32, i32),
        opposite: (i32, i32),
        capture: bool,
        promote: bool,
        terminal: bool,
        after: AfterPosition,
    ) -> Self {
        let (delta, placement, turn) = components(before.0, same.0, opposite.0);
        let (delta_pst, placement_pst, turn_pst) = components(before.1, same.1, opposite.1);
        Self {
            r#move: text,
            capture,
            promote,
            terminal,
            delta,
            delta_pst,
            after_same: same.0,
            after_same_pst: same.1,
            after_opposite: opposite.0,
            after_opposite_pst: opposite.1,
            placement,
            placement_pst,
            turn,
            turn_pst,
            after,
        }
    }
}

/// PST評価に必要な局面をMNSDと同じ盤面・手番・先獅子コードで表す。
#[derive(Clone, Serialize)]
struct AfterPosition {
    /// dense index順のMNSD盤面コード。
    board: Vec<u8>,
    /// 手番側（先手0、後手1）。
    stm: u8,
    /// 先獅子の対象升（存在しない場合は255）。
    lion: u8,
    /// 着手後の手番側視点の静的評価。
    eval: i32,
    /// FMの補正を含まない評価。
    eval_pst: i32,
}

/// 1局面の評価結果。
#[derive(Serialize)]
struct Probe {
    /// 復元できなかった理由。
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<String>,
    /// 入力レコードの通し番号。
    index: usize,
    /// 手番側視点の静的評価。
    #[serde(skip_serializing_if = "Option::is_none")]
    eval: Option<i32>,
    /// FMの補正を含まないPSTだけの静的評価。
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_pst: Option<i32>,
    /// 着手前の同一配置を逆側の視点から見た評価。
    #[serde(skip_serializing_if = "Option::is_none")]
    eval_opposite: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// FMの補正を含まない対応するPST評価。
    eval_opposite_pst: Option<i32>,
    /// `--promotions`指定時の成り手ごとの評価差。
    #[serde(skip_serializing_if = "Option::is_none")]
    promotions: Option<Vec<MoveDelta>>,
    /// `--moves`指定時の全合法手の評価差。
    #[serde(skip_serializing_if = "Option::is_none")]
    moves: Option<Vec<MoveDelta>>,
}

/// MNSDに履歴のない診断は、既存標本と同じ規則に限定する。
fn diagnostic_rules(rule_set: &str) -> Result<Rules, String> {
    let codes = parse_rule_set(rule_set).map_err(|error| error.to_string())?;
    let rules = Rules::from_codes(&codes).map_err(|error| error.to_string())?;
    if rules != Rules::ENGINE_DEFAULT {
        return Err("diagnostic positions must use engine-default rules".into());
    }
    Ok(rules)
}

/// 局面を読み、評価と成り手の評価差を計算する。
fn run(arguments: &Arguments) -> Result<Vec<Probe>, String> {
    let bytes = std::fs::read(&arguments.pst).map_err(|error| error.to_string())?;
    let pst = Pst::decode(&bytes).map_err(|error| error.to_string())?;
    let file = File::open(&arguments.positions).map_err(|error| error.to_string())?;
    let mut reader = Reader::new(BufReader::new(file)).map_err(|error| error.to_string())?;
    let rules = diagnostic_rules(reader.header().rule_set())?;
    let generator = MoveGenerator::new(rules.moves);
    let mut probes = Vec::new();
    let mut index = 0;
    while let Some(record) = reader.read_record().map_err(|error| error.to_string())? {
        let position = match record.to_position() {
            Ok(position) => position,
            Err(error) if arguments.skip_invalid => {
                probes.push(Probe {
                    index,
                    eval: None,
                    eval_pst: None,
                    eval_opposite: None,
                    eval_opposite_pst: None,
                    skipped: Some(error.to_string()),
                    promotions: None,
                    moves: None,
                });
                index += 1;
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        let side = position.side_to_move();
        let before = evaluate_for_diagnostics(&pst, &position, side);
        let before_opposite = evaluate_for_diagnostics(&pst, &position, side.opposite());
        let mut promotions = arguments.promotions.then(Vec::new);
        let mut moves = arguments.moves.then(Vec::new);
        if arguments.promotions || arguments.moves {
            let game = Game::from_position(rules, position.clone());
            for mv in game.legal_moves() {
                if !arguments.moves && !mv.promote {
                    continue;
                }
                let text =
                    usi::text(&position, mv, &generator).expect("legal move must have a USI text");
                let mut after = game.clone();
                after.play(mv).expect("legal move must be playable");
                let capture =
                    after.position().occupied().popcount() < position.occupied().popcount();
                let terminal = after.result().is_some();
                let same = evaluate_for_diagnostics(&pst, after.position(), side);
                let opposite = evaluate_for_diagnostics(&pst, after.position(), side.opposite());
                let encoded =
                    Record::from_position(after.position(), 0, Outcome::Draw, 1, 0).encode();
                let after_position = AfterPosition {
                    board: encoded[..BOARD_SQUARE_COUNT].to_vec(),
                    stm: encoded[144],
                    lion: encoded[145],
                    eval: opposite.0,
                    eval_pst: opposite.1,
                };
                if mv.promote
                    && let Some(promotions) = &mut promotions
                {
                    promotions.push(MoveDelta::new(
                        text.clone(),
                        before,
                        same,
                        opposite,
                        capture,
                        mv.promote,
                        terminal,
                        after_position.clone(),
                    ));
                }
                if let Some(moves) = &mut moves {
                    moves.push(MoveDelta::new(
                        text,
                        before,
                        same,
                        opposite,
                        capture,
                        mv.promote,
                        terminal,
                        after_position,
                    ));
                }
            }
        }
        probes.push(Probe {
            index,
            skipped: None,
            eval: Some(before.0),
            eval_pst: Some(before.1),
            eval_opposite: Some(before_opposite.0),
            eval_opposite_pst: Some(before_opposite.1),
            promotions,
            moves,
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
pub(super) fn main(arguments: Arguments) -> ExitCode {
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

#[cfg(test)]
mod tests {
    use super::{components, diagnostic_rules};

    #[test]
    fn diagnostic_rules_accept_equivalent_codes_and_reject_other_rules() {
        assert_eq!(
            diagnostic_rules("engine-default").unwrap(),
            diagnostic_rules("L0,P0,R1,E0").unwrap()
        );
        assert!(diagnostic_rules("L1,P0,R1,E0").is_err());
        assert!(diagnostic_rules("unknown").is_err());
    }

    #[test]
    fn antisymmetric_evaluation_has_only_placement_component() {
        // 同一配置で両側の評価が逆符号なら、手番成分は0。
        assert_eq!(components(100, 175, -175), (75, 75, 0));
        assert_eq!(components(-100, -175, 175), (-75, -75, 0));
    }

    #[test]
    fn unchanged_placement_with_tempo_has_only_turn_component() {
        // 駒配置価値100、手番の価値30なら両視点は130と-70。
        assert_eq!(components(130, 130, -70), (-60, 0, -60));
        assert_eq!(components(-70, -70, 130), (-60, 0, -60));
    }

    #[test]
    fn components_sum_to_negamax_delta() {
        // 正負、0、静的評価限界を含む独立した3評価で保存則を検査する。
        for before in [-30000, -100, 0, 100, 30000] {
            for same in [-30000, -70, 0, 130, 30000] {
                for opposite in [-30000, -70, 0, 130, 30000] {
                    let (delta, placement, turn) = components(before, same, opposite);
                    assert_eq!(delta, placement + turn);
                    assert_eq!(before + delta, -opposite);
                }
            }
        }
    }
}

//! MNPT重みファイルでMNSD局面を評価し、合法手による評価変化を列挙する。
//!
//! PST学習の診断（`tools/train/pst/pst_diagnostics.py`）から呼ばれ、Pythonの整数参照評価と
//! Rustの評価の一致確認、および代表局面の成りの診断に使う。出力はJSON配列であり、
//! 各要素は入力レコードの順に`index`、手番側視点の`eval`、および成り手ごとの
//! 着手前の手番側視点で測った評価差`delta`を持つ。

use std::fs::File;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use minase::core::rules::parse_rule_set;
use minase::eval::Pst;
use minase::eval::pst::evaluate_for_diagnostics;
use minase::eval::training_data::Reader;
use minase::notation::usi;
use minase::{Game, MoveGenerator, Rules};
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
    /// 各局面の全合法手を適用して評価差を列挙する。
    #[arg(long)]
    moves: bool,
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
    after_same_pst: i32,
    /// 着手後の配置を着手後の手番側から見た評価。
    after_opposite: i32,
    after_opposite_pst: i32,
    /// 視点を固定した配置変化の成分。
    placement: i32,
    placement_pst: i32,
    /// 着手後の同一配置における手番交代の成分。
    turn: i32,
    turn_pst: i32,
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
    fn new(
        text: String,
        before: (i32, i32),
        same: (i32, i32),
        opposite: (i32, i32),
        capture: bool,
        promote: bool,
        terminal: bool,
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
        }
    }
}

/// 1局面の評価結果。
#[derive(Serialize)]
struct Probe {
    /// 入力レコードの通し番号。
    index: usize,
    /// 手番側視点の静的評価。
    eval: i32,
    /// FMの補正を含まないPSTだけの静的評価。
    eval_pst: i32,
    /// 着手前の同一配置を逆側の視点から見た評価。
    eval_opposite: i32,
    eval_opposite_pst: i32,
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
        let position = record.to_position().map_err(|error| error.to_string())?;
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
                    ));
                }
                if let Some(moves) = &mut moves {
                    moves.push(MoveDelta::new(
                        text, before, same, opposite, capture, mv.promote, terminal,
                    ));
                }
            }
        }
        probes.push(Probe {
            index,
            eval: before.0,
            eval_pst: before.1,
            eval_opposite: before_opposite.0,
            eval_opposite_pst: before_opposite.1,
            promotions,
            moves,
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

//! ランダム対局の規則、着手、結果を表示用の文字列へ変換する。

use minase::{GameResult, Move, RuleCode, Square};

use super::game::Outcome;

/// ルールコードの表示文字列を返す。
const fn rule_code_text(code: RuleCode) -> &'static str {
    match code {
        RuleCode::L0 => "L0",
        RuleCode::L1 => "L1",
        RuleCode::L2 => "L2",
        RuleCode::L3 => "L3",
        RuleCode::L4 => "L4",
        RuleCode::P0 => "P0",
        RuleCode::P1 => "P1",
        RuleCode::P2 => "P2",
        RuleCode::P3 => "P3",
        RuleCode::P4 => "P4",
        RuleCode::P5 => "P5",
        RuleCode::P6 => "P6",
        RuleCode::R1 => "R1",
        RuleCode::R2 => "R2",
        RuleCode::R3 => "R3",
        RuleCode::E0 => "E0",
        RuleCode::E1 => "E1",
        RuleCode::E2 => "E2",
        RuleCode::E3 => "E3",
    }
}

/// 採用するルールコード列をカンマ区切りで返す。
pub(super) fn rules_text(codes: &[RuleCode]) -> String {
    codes
        .iter()
        .map(|&code| rule_code_text(code))
        .collect::<Vec<_>>()
        .join(",")
}

/// 升をperftと同じ0起算座標で表記する。
fn square_text(square: Square) -> String {
    format!("({},{})", square.file(), square.rank())
}

/// 着手をperftの`move_text`と同じ形式で表記する。
pub(super) fn move_text(mv: Move) -> String {
    if let Some(mid) = mv.mid {
        format!(
            "double {}->{}->{}{}",
            square_text(mv.from),
            square_text(mid),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    } else {
        format!(
            "move {}->{}{}",
            square_text(mv.from),
            square_text(mv.to),
            if mv.promote { "+" } else { "" }
        )
    }
}

/// 結果を1局1行の表示形式へ変換する。
pub(super) fn outcome_text(outcome: Outcome) -> String {
    match outcome {
        Outcome::Finished(GameResult::Win { winner, reason }) => {
            format!("win winner={winner:?} reason={reason:?}")
        }
        Outcome::Finished(GameResult::Draw { reason }) => {
            format!("draw reason={reason:?}")
        }
        Outcome::Cutoff => "cutoff".to_owned(),
    }
}

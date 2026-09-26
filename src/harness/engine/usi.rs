//! USIの探索情報とオプションの解釈。

use crate::harness::engine::process::EngineProcess;
use crate::harness::engine::{EngineDefaults, EngineEvaluation, EngineScore};
use crate::harness::failure::EngineFailure;
use crate::harness::records::{ScoreBound, StopReasonRecord};

/// USI `info`行から評価値を解析する。
fn parse_usi_evaluation(line: &str) -> Result<Option<EngineEvaluation>, EngineFailure> {
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.first() != Some(&"info") {
        return Ok(None);
    }
    if tokens.get(1) == Some(&"string") {
        return Ok(None);
    }
    let Some(score_index) = tokens.iter().position(|token| *token == "score") else {
        return Ok(None);
    };
    let kind = tokens.get(score_index + 1).ok_or(EngineFailure::Crash)?;
    let value = *tokens.get(score_index + 2).ok_or(EngineFailure::Crash)?;
    let score = match *kind {
        "cp" => EngineScore::Cp(value.parse().map_err(|_| EngineFailure::Crash)?),
        "mate" => match value {
            "+" => EngineScore::MateIn(None),
            "-" => EngineScore::MatedIn(None),
            value => {
                let moves = value.parse::<i32>().map_err(|_| EngineFailure::Crash)?;
                if moves >= 0 {
                    EngineScore::MateIn(Some(moves.unsigned_abs()))
                } else {
                    EngineScore::MatedIn(Some(moves.unsigned_abs()))
                }
            }
        },
        _ => return Err(EngineFailure::Crash),
    };
    let depth = tokens
        .iter()
        .position(|token| *token == "depth")
        .map(|index| {
            tokens
                .get(index + 1)
                .ok_or(EngineFailure::Crash)?
                .parse::<u32>()
                .map_err(|_| EngineFailure::Crash)
        })
        .transpose()?;
    let lower = tokens[score_index + 3..].contains(&"lowerbound");
    let upper = tokens[score_index + 3..].contains(&"upperbound");
    let bound = match (lower, upper) {
        (false, false) => ScoreBound::Exact,
        (true, false) => ScoreBound::Lower,
        (false, true) => ScoreBound::Upper,
        (true, true) => return Err(EngineFailure::Crash),
    };
    Ok(Some(EngineEvaluation {
        depth,
        score,
        bound,
    }))
}

/// 有効な評価行だけで監査値を更新し、通信結果には影響させない。
pub(super) fn observe_usi_evaluation(current: &mut Option<EngineEvaluation>, line: &str) {
    if let Ok(Some(parsed)) = parse_usi_evaluation(line) {
        *current = Some(parsed);
    }
}

/// USI `info string stop`行から停止理由を解析する。
fn parse_usi_stop_reason(line: &str) -> Result<Option<StopReasonRecord>, EngineFailure> {
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.get(..3) != Some(["info", "string", "stop"].as_slice()) {
        return Ok(None);
    }
    let reason = match tokens.as_slice() {
        ["info", "string", "stop", "depth"] => StopReasonRecord::Depth,
        ["info", "string", "stop", "nodes"] => StopReasonRecord::Nodes,
        ["info", "string", "stop", "soft"] => StopReasonRecord::Soft,
        ["info", "string", "stop", "hard"] => StopReasonRecord::Hard,
        ["info", "string", "stop", "external"] => StopReasonRecord::External,
        _ => return Err(EngineFailure::Crash),
    };
    Ok(Some(reason))
}

/// USI `info`行から探索開始後の経過時間を解析する。
fn parse_usi_time(line: &str) -> Result<Option<u64>, EngineFailure> {
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.first() != Some(&"info") || tokens.get(1) == Some(&"string") {
        return Ok(None);
    }
    let Some(index) = tokens.iter().position(|token| *token == "time") else {
        return Ok(None);
    };
    let value = tokens
        .get(index + 1)
        .ok_or(EngineFailure::Crash)?
        .parse()
        .map_err(|_| EngineFailure::Crash)?;
    Ok(Some(value))
}

/// 停止理由と完了反復の経過時間を最新の`info`行で更新する。
pub(super) fn observe_usi_search_data(
    stop_reason: &mut Option<StopReasonRecord>,
    completed_time_ms: &mut Option<u64>,
    line: &str,
) -> Result<(), EngineFailure> {
    if let Some(reason) = parse_usi_stop_reason(line)? {
        *stop_reason = Some(reason);
    }
    if let Some(time) = parse_usi_time(line)? {
        *completed_time_ms = Some(time);
    }
    Ok(())
}

/// USI `option`行から指定したspin optionの既定値を得る。
fn parse_usi_spin_default(line: &str, expected_name: &str) -> Option<u64> {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    if tokens.first() != Some(&"option") || tokens.get(1) != Some(&"name") {
        return None;
    }
    let type_index = tokens.iter().position(|token| *token == "type")?;
    if tokens.get(type_index + 1) != Some(&"spin")
        || tokens[2..type_index].join(" ") != expected_name
    {
        return None;
    }
    let default_index = tokens[type_index + 2..]
        .iter()
        .position(|token| *token == "default")?
        + type_index
        + 2;
    tokens.get(default_index + 1)?.parse().ok()
}

/// 資源に関係する有効なUSI `option`行を記録する。
fn observe_usi_default(defaults: &mut EngineDefaults, line: &str) {
    if let Some(value) =
        parse_usi_spin_default(line, "Threads").and_then(|value| value.try_into().ok())
    {
        defaults.threads = Some(value);
    }
    if let Some(value) = parse_usi_spin_default(line, "USI_Hash") {
        defaults.hash_mb = Some(value);
    }
}

impl EngineProcess {
    /// USI `option`行を`usiok`まで読み、既定資源を返す。
    pub(super) fn read_usi_defaults(&mut self) -> Result<EngineDefaults, EngineFailure> {
        self.send("usi")?;
        let mut defaults = EngineDefaults::default();
        self.receive_until(|line| {
            observe_usi_default(&mut defaults, line);
            line.trim() == "usiok"
        })?;
        Ok(defaults)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usi_resource_probe_parses_exact_spin_option_defaults() {
        let mut defaults = EngineDefaults::default();
        observe_usi_default(
            &mut defaults,
            "option name Threads type spin default 2 min 1 max 20",
        );
        observe_usi_default(
            &mut defaults,
            "option name USI_Hash type spin default 256 min 1 max 65536",
        );
        observe_usi_default(
            &mut defaults,
            "option name Other Threads type spin default 99 min 1 max 100",
        );
        assert_eq!(
            defaults,
            EngineDefaults {
                threads: Some(2),
                hash_mb: Some(256),
            }
        );
        assert_eq!(
            parse_usi_spin_default("option name Threads type check default true", "Threads"),
            None
        );
    }

    // match-harness-efficiency.md「実行記録と再開」「早期投了の検証」:
    // bestmove前の最後のscore付きinfoを保存し、後続の非評価infoでは消去しない。
    #[test]
    fn usi_evaluation_uses_the_last_score_bearing_info_line() {
        assert_eq!(
            parse_usi_evaluation("info depth 1 score cp 12"),
            Ok(Some(EngineEvaluation {
                depth: Some(1),
                score: EngineScore::Cp(12),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(
            parse_usi_evaluation("info score mate -3 depth 5"),
            Ok(Some(EngineEvaluation {
                depth: Some(5),
                score: EngineScore::MatedIn(Some(3)),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(
            parse_usi_evaluation("info score mate +"),
            Ok(Some(EngineEvaluation {
                depth: None,
                score: EngineScore::MateIn(None),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(
            parse_usi_evaluation("info score mate -"),
            Ok(Some(EngineEvaluation {
                depth: None,
                score: EngineScore::MatedIn(None),
                bound: ScoreBound::Exact,
            }))
        );
        assert_eq!(parse_usi_evaluation("info string searching"), Ok(None));
        assert_eq!(
            parse_usi_evaluation("info score cp broken"),
            Err(EngineFailure::Crash)
        );
        let mut observed = None;
        observe_usi_evaluation(&mut observed, "info score cp broken");
        assert_eq!(observed, None);
        observe_usi_evaluation(&mut observed, "info score mate +");
        assert_eq!(observed.unwrap().score, EngineScore::MateIn(None));
    }

    #[test]
    fn usi_search_data_uses_the_last_time_and_known_stop_reason() {
        let mut stop_reason = None;
        let mut completed_time_ms = None;
        observe_usi_search_data(
            &mut stop_reason,
            &mut completed_time_ms,
            "info depth 3 time 12 score cp 0",
        )
        .unwrap();
        observe_usi_search_data(
            &mut stop_reason,
            &mut completed_time_ms,
            "info depth 4 score cp 1 time 34",
        )
        .unwrap();
        observe_usi_search_data(
            &mut stop_reason,
            &mut completed_time_ms,
            "info string stop hard",
        )
        .unwrap();
        assert_eq!(stop_reason, Some(StopReasonRecord::Hard));
        assert_eq!(completed_time_ms, Some(34));

        for (word, expected) in [
            ("depth", StopReasonRecord::Depth),
            ("nodes", StopReasonRecord::Nodes),
            ("soft", StopReasonRecord::Soft),
            ("hard", StopReasonRecord::Hard),
            ("external", StopReasonRecord::External),
        ] {
            assert_eq!(
                parse_usi_stop_reason(&format!("info string stop {word}")),
                Ok(Some(expected))
            );
        }
    }

    #[test]
    fn invalid_usi_search_data_is_a_crash() {
        assert_eq!(
            parse_usi_stop_reason("info string stop unknown"),
            Err(EngineFailure::Crash)
        );
        assert_eq!(
            parse_usi_stop_reason("info string stop"),
            Err(EngineFailure::Crash)
        );
        assert_eq!(
            parse_usi_time("info depth 1 time invalid"),
            Err(EngineFailure::Crash)
        );
        assert_eq!(parse_usi_time("info string time invalid"), Ok(None));
    }
}

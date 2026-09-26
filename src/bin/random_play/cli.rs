//! ランダム対局の引数と規則セットを解析する。

use clap::Parser;
use minase::{RuleCode, core::rules::parse_rule_set};

use super::DEFAULT_MAX_PLY;

/// ランダム対局検証ハーネスのコマンドライン引数。
#[derive(Parser)]
#[command(name = "random_play")]
pub(super) struct Arguments {
    /// 実行する対局数。
    #[arg(
        long,
        default_value_t = 1,
        conflicts_with = "game",
        value_parser = parse_positive_u64
    )]
    pub(super) games: u64,
    /// 全局の乱数列を派生させる基本シード。
    #[arg(long)]
    pub(super) seed: Option<u64>,
    /// 単独で再現する1起算の局番号。
    #[arg(long, value_parser = parse_positive_u64)]
    pub(super) game: Option<u64>,
    /// 採用するローカルルールコード列または規則セット名。
    #[arg(long, required = true, value_parser = parse_rule_set_argument)]
    pub(super) rules: RuleSetArgument,
    /// 1局を打ち切る手数上限。
    #[arg(long, default_value_t = DEFAULT_MAX_PLY, value_parser = parse_positive_u32)]
    pub(super) max_ply: u32,
    /// 毎手の全合法手を複製した対局へ適用して検証する。
    #[arg(long)]
    pub(super) verify_all: bool,
    /// 各局の全手順を表示する。
    #[arg(long)]
    pub(super) verbose: bool,
}

/// 解析済みの`--rules`引数。
#[derive(Clone)]
pub(super) struct RuleSetArgument(pub(super) Vec<RuleCode>);

/// `--rules`の値を規則セット名またはコード列として解析する。
fn parse_rule_set_argument(input: &str) -> Result<RuleSetArgument, String> {
    parse_rule_set(input)
        .map(RuleSetArgument)
        .map_err(|error| error.to_string())
}

/// 0より大きい`u64`を解析する。
fn parse_positive_u64(text: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 0より大きい`u32`を解析する。
fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let value = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::Rules;

    // D8-HARN-16(random-play.mdコマンドライン引数節): `--games`既定1、
    // `--max-ply`既定4096、`--seed`省略可、`--verify-all`・`--verbose`受理。
    // `--rules`は明文で必須、`--game`と`--games`の同時指定はエラー、
    // 局番号は1起算(0は不正)。
    #[test]
    fn cli_defaults_flags_and_conflicts_follow_the_design_document() {
        let defaults = Arguments::try_parse_from(["random_play", "--rules", "engine-default"])
            .expect("the minimal documented invocation must be accepted");
        assert_eq!(defaults.games, 1);
        assert_eq!(defaults.max_ply, 4096);
        assert_eq!(defaults.seed, None);
        assert_eq!(defaults.game, None);
        assert!(!defaults.verify_all);
        assert!(!defaults.verbose);

        let full = Arguments::try_parse_from([
            "random_play",
            "--rules",
            "R1",
            "--games",
            "30",
            "--seed",
            "7",
            "--max-ply",
            "64",
            "--verify-all",
            "--verbose",
        ])
        .expect("all documented options must be accepted together");
        assert_eq!(full.games, 30);
        assert_eq!(full.seed, Some(7));
        assert_eq!(full.max_ply, 64);
        assert!(full.verify_all);
        assert!(full.verbose);

        // --rules省略はエラー(match_runnerと異なり明文で必須)
        assert!(Arguments::try_parse_from(["random_play"]).is_err());
        // --gameと--gamesの同時指定はエラー
        assert!(
            Arguments::try_parse_from([
                "random_play",
                "--rules",
                "engine-default",
                "--game",
                "2",
                "--games",
                "5",
            ])
            .is_err()
        );
        // 局番号は1起算のため0は受理しない
        assert!(
            Arguments::try_parse_from(["random_play", "--rules", "engine-default", "--game", "0",])
                .is_err()
        );
    }

    // RULES.md第33条: 正常なプリセットと不正な併記で共通parserへの接続を検査する。
    #[test]
    fn rules_argument_resolves_presets_case_insensitively_and_rejects_combinations() {
        let preset = Arguments::try_parse_from(["random_play", "--rules", "LISHOGI"])
            .expect("the common rules parser must accept the preset");
        assert_eq!(preset.rules.0, Vec::<RuleCode>::from(Rules::LISHOGI));
        assert!(Arguments::try_parse_from(["random_play", "--rules", "lishogi,P1"]).is_err());
    }
}

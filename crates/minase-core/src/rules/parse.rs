//! 規則セットのプリセット名とコード列の解析。

use core::fmt;

use super::{RuleCode, RuleCodeParseError, Rules};

/// 規則セット文字列の構文エラー。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RuleSetParseError {
    /// 未知の規則コードが含まれている。
    UnknownCode(RuleCodeParseError),
    /// 規則セットのプリセット名が別の要素と併記されている。
    PresetMustBeAlone {
        /// 併記されたプリセット名。
        preset: &'static str,
    },
}

impl fmt::Display for RuleSetParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCode(error) => error.fmt(formatter),
            Self::PresetMustBeAlone { preset } => {
                write!(
                    formatter,
                    "rule set preset '{preset}' must be specified alone"
                )
            }
        }
    }
}

impl std::error::Error for RuleSetParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnknownCode(error) => Some(error),
            Self::PresetMustBeAlone { .. } => None,
        }
    }
}

impl From<RuleCodeParseError> for RuleSetParseError {
    fn from(error: RuleCodeParseError) -> Self {
        Self::UnknownCode(error)
    }
}

/// 規則セット名と、名前が表す規則集合の対応表。
///
/// `engine-default`は[`Rules::ENGINE_DEFAULT`]と同じ規則を表す。
/// `lishogi`の組合せはRULES.md第33条第6項に基づき、名前と組合せの一致は
/// `tests/lishogi_replay.rs`の棋譜リプレイ照合が検証する。
const RULE_SET_PRESETS: &[(&str, Rules)] = &[
    ("engine-default", Rules::ENGINE_DEFAULT),
    ("lishogi", Rules::LISHOGI),
];

/// 規則セット値を、プリセット名またはコンマ区切りの規則コード列として解析する。
///
/// プリセット名は単独で指定し、規則コードとの併記は認めない(第33条第5項・第6項)。
/// 重複および排他制約の検証は`Rules::from_codes`が担う。
pub fn parse_rule_set(input: &str) -> Result<Vec<RuleCode>, RuleSetParseError> {
    if let Some((_, rules)) = RULE_SET_PRESETS
        .iter()
        .find(|(name, _)| input.eq_ignore_ascii_case(name))
    {
        return Ok((*rules).into());
    }

    input
        .split(',')
        .map(|element| {
            if let Some((name, _)) = RULE_SET_PRESETS
                .iter()
                .find(|(name, _)| element.eq_ignore_ascii_case(name))
            {
                Err(RuleSetParseError::PresetMustBeAlone { preset: name })
            } else {
                element.parse().map_err(RuleSetParseError::from)
            }
        })
        .collect()
}

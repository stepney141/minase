//! 対局で採用する規則セットの管理。

mod assembly;
mod code;
mod parse;
mod set;

#[cfg(test)]
mod tests;

pub use assembly::RulesError;
pub use code::{RuleCode, RuleCodeParseError};
pub use parse::{RuleSetParseError, parse_rule_set};
pub use set::{
    ExhaustionRule, LionRule, MoveRules, PromotionRule, RepetitionRule, RuleGroup, Rules,
};

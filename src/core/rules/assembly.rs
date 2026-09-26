//! 規則コード列からの規則セットの組み立てと検査。

use core::fmt;

use super::{
    ExhaustionRule, LionRule, MoveRules, PromotionRule, RepetitionRule, RuleCode, RuleGroup, Rules,
};

/// ルールコード集合の検証エラー。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RulesError {
    /// 同じコードの重複指定(第33条第4項)。
    Duplicate(RuleCode),
    /// 併用できないコードの組合せ(第33条第9項)。
    Conflicting {
        /// 矛盾する組の一方。
        first: RuleCode,
        /// 矛盾する組のもう一方。
        second: RuleCode,
    },
    /// 必須の排他群が指定されていない(第33条第4項)。
    Missing(RuleGroup),
}

impl fmt::Display for RulesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicting { first, second } => {
                write!(
                    formatter,
                    "conflicting rule codes: {first:?} and {second:?}"
                )
            }
            Self::Duplicate(code) => write!(formatter, "duplicate rule code: {code:?}"),
            Self::Missing(group) => {
                let name = match group {
                    RuleGroup::Lion => "lion",
                    RuleGroup::Promotion => "promotion",
                    RuleGroup::Repetition => "repetition",
                    RuleGroup::Exhaustion => "exhaustion",
                };
                write!(formatter, "missing {name} rule")
            }
        }
    }
}

impl std::error::Error for RulesError {}

impl Rules {
    /// コード列を意味検証して規則集合を作る(第33条第4項・第9項)。
    pub fn from_codes(codes: &[RuleCode]) -> Result<Self, RulesError> {
        for (index, &code) in codes.iter().enumerate() {
            if codes[..index].contains(&code) {
                return Err(RulesError::Duplicate(code));
            }
        }

        let mut lion = None;
        let mut promotion = None;
        let mut repetition = None;
        let mut exhaustion = None;
        let mut l4 = false;

        for code in RuleCode::ALL {
            if !codes.contains(&code) {
                continue;
            }
            match code {
                RuleCode::L0 => assign_group(&mut lion, LionRule::L0 { l4: false }, code)?,
                RuleCode::L1 => assign_group(&mut lion, LionRule::L1, code)?,
                RuleCode::L2 | RuleCode::L3 => {}
                RuleCode::L4 => {
                    if lion.is_some_and(|(rule, _)| rule == LionRule::L1) {
                        return Err(RulesError::Conflicting {
                            first: RuleCode::L1,
                            second: RuleCode::L4,
                        });
                    }
                    l4 = true;
                }
                RuleCode::P0 => assign_group(&mut promotion, PromotionRule::P0, code)?,
                RuleCode::P1 => assign_group(&mut promotion, PromotionRule::P1, code)?,
                RuleCode::P2 => assign_group(&mut promotion, PromotionRule::P2, code)?,
                RuleCode::P3 | RuleCode::P4 | RuleCode::P5 | RuleCode::P6 => {}
                RuleCode::R1 => assign_group(&mut repetition, RepetitionRule::R1, code)?,
                RuleCode::R2 => assign_group(&mut repetition, RepetitionRule::R2, code)?,
                RuleCode::R3 => assign_group(&mut repetition, RepetitionRule::R3, code)?,
                RuleCode::E0 => assign_group(&mut exhaustion, ExhaustionRule::E0, code)?,
                RuleCode::E1 => {}
                RuleCode::E2 => assign_group(&mut exhaustion, ExhaustionRule::E2, code)?,
                RuleCode::E3 => assign_group(&mut exhaustion, ExhaustionRule::E3, code)?,
            }
        }

        let lion = lion
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Lion))?;
        let promotion = promotion
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Promotion))?;
        let repetition = repetition
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Repetition))?;
        let exhaustion = exhaustion
            .map(|(rule, _)| rule)
            .ok_or(RulesError::Missing(RuleGroup::Exhaustion))?;

        Ok(Self {
            moves: MoveRules {
                lion: match lion {
                    LionRule::L0 { .. } => LionRule::L0 { l4 },
                    LionRule::L1 => LionRule::L1,
                },
                l2: codes.contains(&RuleCode::L2),
                l3: codes.contains(&RuleCode::L3),
                promotion,
                p3: codes.contains(&RuleCode::P3),
                p4: codes.contains(&RuleCode::P4),
                p5: codes.contains(&RuleCode::P5),
                p6: codes.contains(&RuleCode::P6),
            },
            repetition,
            e1: codes.contains(&RuleCode::E1),
            exhaustion,
        })
    }
}

/// 排他群のスロットへ規則を入れる。既に埋まっていれば先に入れたコードとの`Conflicting`を返す。
fn assign_group<T: Copy>(
    slot: &mut Option<(T, RuleCode)>,
    value: T,
    second: RuleCode,
) -> Result<(), RulesError> {
    if let Some((_, first)) = *slot {
        return Err(RulesError::Conflicting { first, second });
    }
    *slot = Some((value, second));
    Ok(())
}

//! 規則コードと規則セットの構築・解析の検証。

use super::*;

// ---------- 第30〜33条 規則セットの検証 ----------

#[test]
fn article_29_30_all_rule_codes_round_trip_through_text() {
    assert_eq!(RuleCode::ALL.len(), 19);
    for code in RuleCode::ALL {
        assert_eq!(code.to_string().parse::<RuleCode>(), Ok(code));
    }
    assert!("R0".parse::<RuleCode>().is_err());
}

#[test]
fn article_33_4_from_codes_accepts_complete_rule_sets() {
    let codes = [
        RuleCode::L0,
        RuleCode::L2,
        RuleCode::L3,
        RuleCode::L4,
        RuleCode::P1,
        RuleCode::P3,
        RuleCode::P4,
        RuleCode::P5,
        RuleCode::P6,
        RuleCode::R2,
        RuleCode::E1,
        RuleCode::E2,
    ];
    let rules = Rules::from_codes(&codes).unwrap();
    assert_eq!(Vec::<RuleCode>::from(rules), codes);
}

#[test]
fn article_33_4_from_codes_reports_duplicate_conflict_and_missing_in_contract_order() {
    assert_eq!(
        Rules::from_codes(&[
            RuleCode::R1,
            RuleCode::R2,
            RuleCode::R1,
            RuleCode::L0,
            RuleCode::P0,
            RuleCode::E0,
        ]),
        Err(RulesError::Duplicate(RuleCode::R1))
    );
    assert_eq!(
        Rules::from_codes(&[
            RuleCode::L1,
            RuleCode::L4,
            RuleCode::P0,
            RuleCode::R1,
            RuleCode::E0,
        ]),
        Err(RulesError::Conflicting {
            first: RuleCode::L1,
            second: RuleCode::L4,
        })
    );
    assert_eq!(
        Rules::from_codes(&[
            RuleCode::L1,
            RuleCode::L0,
            RuleCode::P0,
            RuleCode::R1,
            RuleCode::E0,
        ]),
        Err(RulesError::Conflicting {
            first: RuleCode::L0,
            second: RuleCode::L1,
        })
    );
    assert_eq!(
        Rules::from_codes(&[
            RuleCode::L0,
            RuleCode::P0,
            RuleCode::P1,
            RuleCode::R1,
            RuleCode::R2,
            RuleCode::E0,
        ]),
        Err(RulesError::Conflicting {
            first: RuleCode::P0,
            second: RuleCode::P1,
        })
    );
    assert_eq!(
        Rules::from_codes(&[RuleCode::L4, RuleCode::P0, RuleCode::R1, RuleCode::E0]),
        Err(RulesError::Missing(RuleGroup::Lion))
    );
    assert_eq!(
        Rules::from_codes(&[]),
        Err(RulesError::Missing(RuleGroup::Lion))
    );
}

#[test]
fn article_33_5_and_33_6_presets_expand_from_rule_constants() {
    for name in ["engine-default", "Engine-Default"] {
        let codes = parse_rule_set(name).unwrap();
        assert_eq!(Rules::from_codes(&codes), Ok(Rules::ENGINE_DEFAULT));
        assert_eq!(codes, Vec::<RuleCode>::from(Rules::ENGINE_DEFAULT));
    }
    for name in ["lishogi", "Lishogi"] {
        let codes = parse_rule_set(name).unwrap();
        assert_eq!(Rules::from_codes(&codes), Ok(Rules::LISHOGI));
        assert_eq!(codes, Vec::<RuleCode>::from(Rules::LISHOGI));
    }
    for invalid in ["engine-default,L1", "engine-default,lishogi"] {
        assert!(parse_rule_set(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn rule_set_parse_errors_preserve_the_failure_kind() {
    let unknown = parse_rule_set("L0,unknown").unwrap_err();
    assert!(matches!(
        unknown,
        RuleSetParseError::UnknownCode(ref error) if error.input() == "unknown"
    ));
    assert!(std::error::Error::source(&unknown).is_some());

    assert_eq!(
        parse_rule_set("lishogi,R1"),
        Err(RuleSetParseError::PresetMustBeAlone { preset: "lishogi" })
    );
}

#[test]
fn rule_set_display_includes_all_four_base_groups() {
    assert_eq!(Rules::ENGINE_DEFAULT.to_string(), "L0,P0,R1,E0");
    assert_eq!(Rules::LISHOGI.to_string(), "L1,L2,P0,P3,R1,E1,E3");
}

#[test]
fn rules_error_display_identifies_the_missing_group() {
    assert_eq!(
        RulesError::Missing(RuleGroup::Lion).to_string(),
        "missing lion rule"
    );
    for (group, name) in [
        (RuleGroup::Promotion, "promotion"),
        (RuleGroup::Repetition, "repetition"),
        (RuleGroup::Exhaustion, "exhaustion"),
    ] {
        assert!(RulesError::Missing(group).to_string().contains(name));
    }
}

#[test]
fn article_25_2_repetition_rule_is_mandatory_and_exclusive() {
    // D3-025-01: 反復規則はR1・R2・R3のいずれか1つの明示が必須であり、
    // 未指定は明示的なエラー、2つ以上の同時指定は競合として拒否される。
    assert_eq!(
        Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::E0]),
        Err(RulesError::Missing(RuleGroup::Repetition))
    );

    for pair in [
        [RuleCode::R1, RuleCode::R2],
        [RuleCode::R1, RuleCode::R3],
        [RuleCode::R2, RuleCode::R3],
    ] {
        let codes = [RuleCode::L0, RuleCode::P0, pair[0], pair[1], RuleCode::E0];
        assert!(matches!(
            Rules::from_codes(&codes),
            Err(RulesError::Conflicting { .. })
        ));
    }
}

#[test]
fn article_33_9_e2_and_e3_conflict() {
    // D3-032-09: E2とE3は同時に採用できず、有効な規則セットとして扱われない。
    assert!(matches!(
        Rules::from_codes(&[
            RuleCode::L0,
            RuleCode::P0,
            RuleCode::R1,
            RuleCode::E2,
            RuleCode::E3,
        ]),
        Err(RulesError::Conflicting { .. })
    ));
}

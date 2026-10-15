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
    use RuleCode::*;
    let cases: &[(&str, &[RuleCode], RulesError)] = &[
        (
            "第33条4項: 重複は競合より先",
            &[R1, R2, R1, L0, P0, E0],
            RulesError::Duplicate(R1),
        ),
        (
            "第33条4・9項: L1とL4",
            &[L1, L4, P0, R1, E0],
            RulesError::Conflicting {
                first: L1,
                second: L4,
            },
        ),
        (
            "第33条4項: 入力順によらずL0とL1",
            &[L1, L0, P0, R1, E0],
            RulesError::Conflicting {
                first: L0,
                second: L1,
            },
        ),
        (
            "第33条4項: 成りの競合は反復より先",
            &[L0, P0, P1, R1, R2, E0],
            RulesError::Conflicting {
                first: P0,
                second: P1,
            },
        ),
        (
            "第30条・第33条9項: P1とP2",
            &[L0, P1, P2, R1, E0],
            RulesError::Conflicting {
                first: P1,
                second: P2,
            },
        ),
        (
            "第33条4項: 修飾規則だけでは獅子規則を満たさない",
            &[L4, P0, R1, E0],
            RulesError::Missing(RuleGroup::Lion),
        ),
        (
            "第33条4項: 空入力では獅子規則を先に報告",
            &[],
            RulesError::Missing(RuleGroup::Lion),
        ),
        (
            "第25条2項・第33条4項: 反復規則は必須",
            &[L0, P0, E0],
            RulesError::Missing(RuleGroup::Repetition),
        ),
        (
            "第25条2項・第33条4項: 反復規則は排他",
            &[L0, P0, R1, R2, E0],
            RulesError::Conflicting {
                first: R1,
                second: R2,
            },
        ),
        (
            "第33条9項: E2とE3",
            &[L0, P0, R1, E2, E3],
            RulesError::Conflicting {
                first: E2,
                second: E3,
            },
        ),
    ];
    for (article, codes, expected) in cases {
        assert_eq!(
            Rules::from_codes(codes),
            Err(*expected),
            "{article}: {codes:?}"
        );
    }
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
    let source = std::error::Error::source(&unknown)
        .unwrap()
        .downcast_ref::<RuleCodeParseError>()
        .expect("unknown code must retain RuleCodeParseError as its source");
    assert_eq!(source.input(), "unknown");
    let RuleSetParseError::UnknownCode(inner) = &unknown else {
        panic!("expected an unknown code error");
    };
    assert_eq!(source, inner);

    // プリセット名と規則コードの併記を拒否する。
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
fn implementation_contract_rules_error_display_identifies_the_missing_group() {
    assert_eq!(
        RulesError::Missing(RuleGroup::Lion).to_string(),
        "missing lion rule"
    );
    for (group, expected) in [
        (RuleGroup::Promotion, "missing promotion rule"),
        (RuleGroup::Repetition, "missing repetition rule"),
        (RuleGroup::Exhaustion, "missing exhaustion rule"),
    ] {
        assert_eq!(
            RulesError::Missing(group).to_string(),
            expected,
            "{group:?}"
        );
    }
}

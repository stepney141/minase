use super::*;

// D8-HARN-21/27（ponder.md「対局ハーネスの対局進行」「同時対局数」）。
#[test]
fn ponder_arguments_require_two_usi_time_controls() {
    for extra in [
        vec![],
        vec!["--candidate-limit", "time=2000+10"],
        vec!["--concurrency", "3"],
    ] {
        let mut args = vec![
            "match_runner",
            "--run-dir",
            "unused",
            "--each",
            "time=1000+10",
            "--ponder",
        ];
        args.extend(extra);
        args.push("gsprt");
        let parsed = Arguments::try_parse_from(args).unwrap();
        assert!(parsed.ponder);
        assert!(parsed.validate_ponder().is_ok());
    }
    for extra in [
        vec!["--each", "depth=4"],
        vec!["--each", "nodes=1000"],
        vec!["--baseline-limit", "depth=1"],
        vec!["--candidate-limit", "nodes=1"],
        vec!["--baseline", "cecp:engine"],
        vec!["--candidate", "cecp:engine"],
    ] {
        let mut args = vec!["match_runner", "--run-dir", "unused", "--ponder"];
        if extra[0] != "--each" {
            args.extend(["--each", "time=1000+10"]);
        }
        args.extend(extra);
        args.push("gsprt");
        assert!(
            Arguments::try_parse_from(args)
                .unwrap()
                .validate_ponder()
                .is_err()
        );
    }
    assert!(
        Arguments::try_parse_from([
            "match_runner",
            "--run-dir",
            "unused",
            "--ponder",
            "true",
            "gsprt"
        ])
        .is_err()
    );
    let plain =
        Arguments::try_parse_from(["match_runner", "--run-dir", "unused", "gsprt"]).unwrap();
    assert!(!plain.ponder);
}

// D8-HARN-27（ponder.md設計判断「同時対局数」）。
#[test]
fn ponder_concurrency_reserves_two_engines_per_game() {
    for (threads, on, off) in [(1, 7, 15), (2, 3, 7)] {
        for (a, b) in [(threads, 1), (1, threads)] {
            assert_eq!(
                default_concurrency(Some(16), Some(a), Some(b), true),
                Ok(on)
            );
            assert_eq!(
                default_concurrency(Some(16), Some(a), Some(b), false),
                Ok(off)
            );
        }
    }
    for (cores, a, b) in [
        (Some(2), Some(1), Some(1)),
        (None, Some(1), Some(1)),
        (Some(16), None, Some(1)),
        (Some(16), Some(1), None),
    ] {
        assert!(default_concurrency(cores, a, b, true).is_err());
    }
}

// D8-HARN-26（ponder.md保存形式）: 不合法な予想手も記録としては有効である。
#[test]
fn saved_predictions_validate_tokens_and_protocol_without_requiring_legality() {
    let opening = generate_opening(Rules::ENGINE_DEFAULT, std::num::NonZeroU64::new(1).unwrap());
    let side = stored_color(opening.game.position().side_to_move());
    let mut record = GameRecord {
        candidate_color: side,
        candidate_seed: 1,
        baseline_seed: 1,
        wall_time_ns: 100,
        candidate_cpu_time_ns: None,
        baseline_cpu_time_ns: None,
        candidate_peak_rss_bytes: None,
        baseline_peak_rss_bytes: None,
        turns: vec![TurnRecord {
            side,
            think_time_ns: 1,
            evaluation: None,
            stop_reason: None,
            completed_time_ms: None,
            ponder: Some("not-a-move".to_owned()),
            response: TurnResponse::Resigned,
        }],
        termination: TerminationRecord::Resigned { loser: side },
    };
    record.wall_time_ns = 100;
    let mut conditions = SavedGameConditions {
        candidate_protocol: Protocol::Usi,
        baseline_protocol: Protocol::Usi,
        candidate_limit: parse_search_limit("time=1000+0").unwrap(),
        baseline_limit: parse_search_limit("time=1000+0").unwrap(),
        response_timeout: Duration::from_secs(1),
    };
    assert!(validate_saved_game(&record, &opening, 100, conditions).is_ok());
    for invalid in ["", "two tokens", " trailing", "trailing "] {
        record.turns[0].ponder = Some(invalid.to_owned());
        assert!(validate_saved_game(&record, &opening, 100, conditions).is_err());
    }
    record.turns[0].ponder = Some("not-a-move".to_owned());
    conditions.candidate_protocol = Protocol::Cecp;
    assert!(validate_saved_game(&record, &opening, 100, conditions).is_err());
}

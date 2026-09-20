use super::*;

// 台本の各要素は受信行の接頭辞、応答前の遅延(ms)、出力行、CPU消費(ms)。
fn fake(steps: Vec<(&str, u64, String, u64)>, limit: &str) -> PlayerConfig {
    let script = format!(
        r#"import sys,json,time
steps = json.loads({steps:?})
for raw in sys.stdin:
    line = raw.strip()
    if line == 'usi':
        print('option name Threads type spin default 1 min 1 max 4\nusiok', flush=True)
        continue
    if line == 'isready':
        print('readyok', flush=True)
        continue
    if line.startswith('setoption ') or line == 'usinewgame':
        continue
    if not steps:
        print('unexpected command: ' + line, file=sys.stderr, flush=True)
        sys.exit(1)
    expected, delay, output, cpu = steps.pop(0)
    if not line.startswith(expected):
        print('expected ' + expected + ', received ' + line, file=sys.stderr, flush=True)
        sys.exit(1)
    time.sleep(delay / 1000)
    end = time.process_time() + cpu / 1000
    while time.process_time() < end:
        pass
    if '$CHECK_MISS_CLOCK' in output:
        words = line.split()
        assert int(words[words.index('btime') + 1]) <= 880, line
        assert int(words[words.index('wtime') + 1]) == 1000, line
        assert int(words[words.index('binc') + 1]) == 50, line
        output = output.replace('$CHECK_MISS_CLOCK', '')
    if output:
        print(output.replace('$INPUT', line), flush=True)
"#,
        steps = serde_json::to_string(&steps).unwrap()
    );
    PlayerConfig {
        text: "scripted USI".to_owned(),
        identity: EngineIdentity::Random {
            sha256: "test".to_owned(),
        },
        path: PathBuf::from("python3"),
        args: vec!["-u".to_owned(), "-c".to_owned(), script],
        protocol: Protocol::Usi,
        is_random: false,
        limit: parse_search_limit(limit).unwrap(),
        hash_mb: None,
        rules_source: "engine-default".to_owned(),
    }
}

fn step(expected: &str, output: impl Into<String>) -> (&str, u64, String, u64) {
    (expected, 0, output.into(), 0)
}

fn position(history: &[String]) -> String {
    if history.is_empty() {
        "position startpos".to_owned()
    } else {
        format!("position startpos moves {}", history.join(" "))
    }
}

fn line_of_play(count: usize) -> (Vec<String>, Vec<Move>) {
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    let mut text = Vec::new();
    let mut moves = Vec::new();
    for _ in 0..count {
        let selected = game.legal_moves()[0];
        text.push(
            usi::text(
                game.position(),
                selected,
                &MoveGenerator::new(game.rules().moves),
            )
            .unwrap(),
        );
        moves.push(selected);
        assert!(!matches!(
            game.play(selected).unwrap(),
            GameStatus::Finished(_)
        ));
    }
    (text, moves)
}

fn seed() -> NonZeroU64 {
    NonZeroU64::new(1).unwrap()
}

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

// D8-HARN-22/23/25/26（ponder.md「対局ハーネスの対局進行」）。
// 双方の台本がposition/go/ponderhit/stopの順序と、的中前のinfoの保持を検査する。
#[test]
fn both_engines_ponder_hit_and_stop_on_resignation() {
    let (m, _) = line_of_play(4);
    let a = fake(
        vec![
            step("position startpos", ""),
            step(
                "go btime 10000 wtime 10000",
                format!("bestmove {} ponder {}", m[0], m[1]),
            ),
            step(&position(&m[..2]), ""),
            step(
                "go ponder btime ",
                "info depth 3 score cp 71 time 30\ninfo string stop soft",
            ),
            step("ponderhit", format!("bestmove {} ponder {}", m[2], m[3])),
            step(&position(&m), ""),
            step("go ponder btime ", ""),
            ("stop", 0, "bestmove discarded".to_owned(), 80),
        ],
        "time=10000+50",
    );
    let b = fake(
        vec![
            step(&position(&m[..1]), ""),
            step("go btime ", format!("bestmove {} ponder {}", m[1], m[2])),
            step(&position(&m[..3]), ""),
            step("go ponder btime ", "info depth 4 score cp 82 time 40"),
            step("ponderhit", "bestmove resign"),
        ],
        "time=10000+50",
    );
    let result = play_game(
        Game::new(Rules::ENGINE_DEFAULT),
        vec![],
        vec![],
        100,
        Color::Black,
        &a,
        seed(),
        &b,
        seed(),
        Duration::from_secs(2),
        true,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.record.turns.len(), 4);
    assert!(matches!(
        result.record.termination,
        TerminationRecord::Resigned {
            loser: StoredColor::White
        }
    ));
    assert_eq!(result.record.turns[0].ponder.as_ref(), Some(&m[1]));
    assert_eq!(
        result.record.turns[2].stop_reason,
        Some(StopReasonRecord::Soft)
    );
    assert_eq!(result.record.turns[2].completed_time_ms, Some(30));
    assert_eq!(
        result.record.turns[2].evaluation.as_ref().unwrap().score,
        ScoreRecord::Cp { value: 71 }
    );
    assert_eq!(
        result.record.turns[3].evaluation.as_ref().unwrap().score,
        ScoreRecord::Cp { value: 82 }
    );
    // CPUを消費するstop応答の後で資源を読まなければ、この下限を満たせない。
    #[cfg(target_os = "linux")]
    assert!(result.record.candidate_cpu_time_ns.unwrap() >= 80_000_000);
    let mut counts = [PonderCounts::default(), PonderCounts::default()];
    count_ponder_game(
        &result.record,
        Game::new(Rules::ENGINE_DEFAULT),
        true,
        100,
        &mut counts,
    )
    .unwrap();
    assert_eq!(
        counts[0],
        PonderCounts {
            predictions: 2,
            illegal_predictions: 0,
            starts: 2,
            hits: 1,
            moves: 2
        }
    );
    assert_eq!(
        counts[1],
        PonderCounts {
            predictions: 1,
            illegal_predictions: 0,
            starts: 1,
            hits: 1,
            moves: 1
        }
    );
    let mut off = [PonderCounts::default(), PonderCounts::default()];
    count_ponder_game(
        &result.record,
        Game::new(Rules::ENGINE_DEFAULT),
        false,
        100,
        &mut off,
    )
    .unwrap();
    assert_eq!(off[0].predictions, 2);
    assert_eq!(off[0].starts + off[0].hits + off[1].starts + off[1].hits, 0);
}

// D8-HARN-23/24（ponder.md「対局ハーネスの対局進行」）。
// 外れの読み捨て後にinfoが混ざらず、stopの遅れが時計と消費時間に入る。
#[test]
fn ponder_miss_discards_old_output_and_debits_stop_time() {
    for early in [false, true] {
        let (m, moves) = line_of_play(2);
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        game.play(moves[0]).unwrap();
        let z = game
            .legal_moves()
            .into_iter()
            .find(|mv| *mv != moves[1])
            .unwrap();
        let z_text =
            usi::text(game.position(), z, &MoveGenerator::new(game.rules().moves)).unwrap();
        let history = vec![m[0].clone(), z_text];
        let stale = "info depth 99 score cp 999 time 999\ninfo string stop hard\nbestmove invalid";
        let a = fake(
            vec![
                step(&position(&m), ""),
                step(
                    "go ponder btime 1000 wtime 1000 binc 50 winc 50 byoyomi 0",
                    if early { stale } else { "" },
                ),
                (
                    "stop",
                    if early { 0 } else { 120 },
                    if early {
                        String::new()
                    } else {
                        stale.to_owned()
                    },
                    0,
                ),
                step(&position(&history), ""),
                (
                    "go btime ",
                    80,
                    if early {
                        "bestmove resign".to_owned()
                    } else {
                        "$CHECK_MISS_CLOCKbestmove resign".to_owned()
                    },
                    0,
                ),
            ],
            "time=1000+50",
        );
        let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
        let mut clocks = GameClocks::new(Color::Black, a.limit, a.limit);
        process.start_ponder(
            &game,
            &m[..1],
            Some(&m[1]),
            &clocks.think_request(Color::Black, a.limit),
        );
        let response = process
            .bestmove(&history, &[moves[0], z], &clocks, Color::Black, a.limit)
            .unwrap();
        assert_eq!(response.response, EngineResponse::Resigned);
        assert!(response.evaluation.is_none());
        assert!(response.stop_reason.is_none());
        assert!(response.completed_time_ms.is_none());
        assert!(response.elapsed >= Duration::from_millis(if early { 80 } else { 200 }));
        clocks
            .get_mut(Color::Black)
            .unwrap()
            .update(response.elapsed)
            .unwrap();
        assert_eq!(
            clocks.get(Color::Black).unwrap().remaining,
            Duration::from_millis(1050).saturating_sub(response.elapsed)
        );
    }
}

// D8-HARN-24（ponder.md「対局ハーネスの対局進行」）。
#[test]
fn ponder_miss_uses_one_response_deadline() {
    let (m, moves) = line_of_play(2);
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    game.play(moves[0]).unwrap();
    let a = fake(
        vec![
            step(&position(&m), ""),
            step("go ponder", ""),
            ("stop", 180, "bestmove stale".to_owned(), 0),
            step("position startpos", ""),
            ("go btime", 180, "bestmove resign".to_owned(), 0),
        ],
        "time=1000+0",
    );
    let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
    process.timeout = Duration::from_millis(300);
    let clocks = GameClocks::new(Color::Black, a.limit, a.limit);
    process.start_ponder(
        &game,
        &m[..1],
        Some(&m[1]),
        &clocks.think_request(Color::Black, a.limit),
    );
    assert!(matches!(
        process.bestmove(&[], &[], &clocks, Color::Black, a.limit),
        Err(EngineFailure::Timeout)
    ));
}

// D8-HARN-22/24（ponder.md「対局ハーネスの対局進行」）。
#[test]
fn ponder_hit_excludes_opponent_time_and_uses_updated_clock() {
    let (m, moves) = line_of_play(2);
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    game.play(moves[0]).unwrap();
    let a = fake(
        vec![
            step(&position(&m), ""),
            step(
                "go ponder btime 950 wtime 1000 binc 50 winc 50 byoyomi 0",
                "info depth 3 score cp 7 time 20",
            ),
            ("ponderhit", 30, "bestmove resign".to_owned(), 0),
        ],
        "time=1000+50",
    );
    let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
    let mut clocks = GameClocks::new(Color::Black, a.limit, a.limit);
    clocks
        .get_mut(Color::Black)
        .unwrap()
        .update(Duration::from_millis(100))
        .unwrap();
    process.start_ponder(
        &game,
        &m[..1],
        Some(&m[1]),
        &clocks.think_request(Color::Black, a.limit),
    );
    let opponent_started = Instant::now();
    std::thread::sleep(Duration::from_millis(250));
    let result = process
        .bestmove(&m, &moves, &clocks, Color::Black, a.limit)
        .unwrap();
    assert!(result.elapsed >= Duration::from_millis(30));
    assert!(opponent_started.elapsed() - result.elapsed >= Duration::from_millis(250));
    assert_eq!(result.evaluation.unwrap().score, EngineScore::Cp(7));
}

// D8-HARN-22/26（ponder.md「対局ハーネスの対局進行」）。
#[test]
fn absent_illegal_and_terminal_predictions_do_not_start_ponder() {
    let (m, _) = line_of_play(1);
    for prediction in [None, Some("not-a-move")] {
        let response = match prediction {
            Some(p) => format!("bestmove {} ponder {p}", m[0]),
            None => format!("bestmove {}", m[0]),
        };
        let a = fake(
            vec![step("position startpos", ""), step("go btime", response)],
            "time=1000+0",
        );
        let b = fake(
            vec![step(&position(&m), ""), step("go btime", "bestmove resign")],
            "time=1000+0",
        );
        let result = play_game(
            Game::new(Rules::ENGINE_DEFAULT),
            vec![],
            vec![],
            100,
            Color::Black,
            &a,
            seed(),
            &b,
            seed(),
            Duration::from_secs(2),
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(matches!(
            result.record.termination,
            TerminationRecord::Resigned {
                loser: StoredColor::White
            }
        ));
        let mut counts = [PonderCounts::default(), PonderCounts::default()];
        count_ponder_game(
            &result.record,
            Game::new(Rules::ENGINE_DEFAULT),
            true,
            100,
            &mut counts,
        )
        .unwrap();
        assert_eq!(counts[0].starts, 0);
        assert_eq!(
            counts[0].illegal_predictions,
            u64::from(prediction.is_some())
        );
    }
    let board =
        minase::notation::sfen::parse_sfen("12/12/12/5k6/12/12/5R6/12/12/12/12/K11 b").unwrap();
    let game = Game::from_position(Rules::ENGINE_DEFAULT, board);
    assert!(matches!(ponder_move(&game, "7g7d"), Ok((_, false))));
    let a = fake(vec![], "time=1000+0");
    let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
    let clocks = GameClocks::new(Color::White, a.limit, a.limit);
    process.start_ponder(
        &game,
        &[],
        Some("7g7d"),
        &clocks.think_request(Color::White, a.limit),
    );
    process.send("isready").unwrap();
    process.wait_for("readyok").unwrap();
    assert!(process.pondering.is_none());
}

// D8-HARN-25（ponder.md「対局ハーネスの対局進行」）。
#[test]
fn ponder_cleanup_keeps_result_when_stop_times_out_and_drop_reaps() {
    let (m, moves) = line_of_play(2);
    for responds in [true, false] {
        let a = fake(
            vec![
                step(&position(&m), ""),
                step("go ponder", ""),
                step("stop", if responds { "bestmove stale" } else { "" }),
            ],
            "time=1000+0",
        );
        let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
        process.timeout = Duration::from_millis(100);
        let pid = process.child.id();
        let mut game = Game::new(Rules::ENGINE_DEFAULT);
        game.play(moves[0]).unwrap();
        let clocks = GameClocks::new(Color::Black, a.limit, a.limit);
        process.start_ponder(
            &game,
            &m[..1],
            Some(&m[1]),
            &clocks.think_request(Color::Black, a.limit),
        );
        let recorded = recorded_game(
            PlayedGame::Cutoff { plies: 1 },
            Color::Black,
            seed(),
            seed(),
            vec![],
            Instant::now(),
            Some(&mut process),
            None,
        );
        assert_eq!(recorded.record.termination, TerminationRecord::Cutoff);
        drop(process);
        #[cfg(target_os = "linux")]
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }
    // 測定中断の早期returnでも、Dropが先読みを止めてプロセスを回収する。
    let a = fake(
        vec![
            step(&position(&m), ""),
            step("go ponder", ""),
            ("stop", 60, "bestmove stale".to_owned(), 0),
        ],
        "time=1000+0",
    );
    let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    game.play(moves[0]).unwrap();
    let clocks = GameClocks::new(Color::Black, a.limit, a.limit);
    process.start_ponder(
        &game,
        &m[..1],
        Some(&m[1]),
        &clocks.think_request(Color::Black, a.limit),
    );
    let start = Instant::now();
    drop(process);
    assert!(start.elapsed() >= Duration::from_millis(60));
}

// D8-HARN-22（ponder.md「先読みの開始」）: 送信失敗は次の手番で通常の異常とする。
#[test]
fn ponder_start_send_failure_returns_to_idle() {
    let (m, moves) = line_of_play(2);
    let a = fake(vec![], "time=1000+0");
    let mut process = EngineProcess::start(&a, 1, Duration::from_secs(2)).unwrap();
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    game.play(moves[0]).unwrap();
    let clocks = GameClocks::new(Color::Black, a.limit, a.limit);
    drop(process.input.take());
    process.start_ponder(
        &game,
        &m[..1],
        Some(&m[1]),
        &clocks.think_request(Color::Black, a.limit),
    );
    assert!(process.pondering.is_none());
    assert!(matches!(
        process.bestmove(&m, &moves, &clocks, Color::Black, a.limit),
        Err(EngineFailure::Crash)
    ));
}

// D8-HARN-25（ponder.md「終局時の先読みの停止」）。
#[test]
fn ponder_stops_before_resources_on_cutoff_and_opponent_failures() {
    let (m, _) = line_of_play(2);
    for (max_ply, reply, delay, expected) in [
        (1, "bestmove resign", 0, TerminationRecord::Cutoff),
        (
            100,
            "bestmove bad-move",
            0,
            TerminationRecord::Forfeit {
                loser: StoredColor::White,
                reason: FailureKind::IllegalMove,
            },
        ),
        (
            100,
            "",
            0,
            TerminationRecord::Forfeit {
                loser: StoredColor::White,
                reason: FailureKind::Timeout,
            },
        ),
        (
            100,
            "bestmove resign",
            80,
            TerminationRecord::Forfeit {
                loser: StoredColor::White,
                reason: FailureKind::TimeForfeit,
            },
        ),
    ] {
        let a = fake(
            vec![
                step("position startpos", ""),
                step("go btime", format!("bestmove {} ponder {}", m[0], m[1])),
                step(&position(&m), ""),
                step("go ponder", ""),
                ("stop", 0, "bestmove ignored".to_owned(), 60),
            ],
            "time=1000+0",
        );
        let b = fake(
            vec![
                step(&position(&m[..1]), ""),
                ("go btime", delay, reply.to_owned(), 0),
            ],
            if delay == 0 {
                "time=1000+0"
            } else {
                "time=10+0"
            },
        );
        let result = play_game(
            Game::new(Rules::ENGINE_DEFAULT),
            vec![],
            vec![],
            max_ply,
            Color::Black,
            &a,
            seed(),
            &b,
            seed(),
            Duration::from_millis(500),
            true,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.record.termination, expected);
        #[cfg(target_os = "linux")]
        assert!(result.record.candidate_cpu_time_ns.unwrap() >= 60_000_000);
    }
}

// D8-HARN-26（ponder.md保存形式）: 不合法な予想手も記録としては有効である。
#[test]
fn saved_predictions_validate_tokens_and_protocol_without_requiring_legality() {
    let opening = generate_opening(Rules::ENGINE_DEFAULT, seed());
    let side = stored_color(opening.game.position().side_to_move());
    let mut record = recorded_game(
        PlayedGame::Finished {
            plies: opening.game.ply_count(),
            outcome: GameOutcome::Resigned {
                winner: color_from_stored(side).opposite(),
            },
        },
        color_from_stored(side),
        seed(),
        seed(),
        vec![TurnRecord {
            side,
            think_time_ns: 1,
            evaluation: None,
            stop_reason: None,
            completed_time_ms: None,
            ponder: Some("not-a-move".to_owned()),
            response: TurnResponse::Resigned,
        }],
        Instant::now(),
        None,
        None,
    )
    .record;
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

// D8-HARN-22/25/26（ponder.md「対局ハーネスの対局進行」）。
#[test]
fn ponder_stops_on_opponent_terminal_move_and_never_starts_after_terminal_move() {
    let board =
        minase::notation::sfen::parse_sfen("12/12/12/p4k6/12/12/5R6/12/12/12/12/K11 w").unwrap();
    let game = Game::from_position(Rules::ENGINE_DEFAULT, board);
    let a = fake(
        vec![
            step("position startpos", ""),
            step("go btime", "bestmove 12d12e ponder 7g7h"),
            step("position startpos moves 12d12e 7g7h", ""),
            step("go ponder", ""),
            ("stop", 0, "bestmove stale".to_owned(), 60),
        ],
        "time=1000+0",
    );
    let b = fake(
        vec![
            step("position startpos moves 12d12e", ""),
            step("go btime", "bestmove 7g7d ponder bad-move"),
        ],
        "time=1000+0",
    );
    let result = play_game(
        game,
        vec![],
        vec![],
        100,
        Color::White,
        &a,
        seed(),
        &b,
        seed(),
        Duration::from_secs(2),
        true,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(matches!(
        result.record.termination,
        TerminationRecord::AdjudicatedWin {
            winner: StoredColor::Black,
            ..
        }
    ));
    assert_eq!(result.record.turns.len(), 2);
    #[cfg(target_os = "linux")]
    assert!(result.record.candidate_cpu_time_ns.unwrap() >= 60_000_000);
}

// D8-HARN-26（ponder.md保存形式）: --ponderを省いても予想手は失わない。
#[test]
fn predictions_are_saved_when_ponder_is_disabled() {
    let (m, _) = line_of_play(2);
    let a = fake(
        vec![
            step("position startpos", ""),
            step("go btime", format!("bestmove {} ponder {}", m[0], m[1])),
        ],
        "time=1000+0",
    );
    let b = fake(
        vec![
            step(&position(&m[..1]), ""),
            step("go btime", "bestmove resign"),
        ],
        "time=1000+0",
    );
    let result = play_game(
        Game::new(Rules::ENGINE_DEFAULT),
        vec![],
        vec![],
        100,
        Color::Black,
        &a,
        seed(),
        &b,
        seed(),
        Duration::from_secs(2),
        false,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(matches!(
        result.record.termination,
        TerminationRecord::Resigned {
            loser: StoredColor::White
        }
    ));
    assert_eq!(result.record.turns[0].ponder.as_ref(), Some(&m[1]));
    let mut counts = [PonderCounts::default(), PonderCounts::default()];
    count_ponder_game(
        &result.record,
        Game::new(Rules::ENGINE_DEFAULT),
        false,
        100,
        &mut counts,
    )
    .unwrap();
    assert_eq!(
        counts[0],
        PonderCounts {
            predictions: 1,
            illegal_predictions: 0,
            starts: 0,
            hits: 0,
            moves: 1
        }
    );
}

use super::*;
use crate::{DrawReason, WinReason};

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

#[cfg(target_os = "linux")]
#[test]
fn linux_resource_probe_reports_current_process_cpu_and_memory() {
    let usage = process_resource_usage(process::id());
    assert!(usage.cpu_time_ns.is_some());
    assert!(usage.peak_rss_bytes.is_some_and(|bytes| bytes > 0));
}

// D8-HARN-01（sprt.md「エンジンの指定方法」、match-harness.md「エンジン指定と
// 解決」）: specは`commit:<hash>`・USI起動コマンド・`random`・CECP起動コマンドの
// 4形式であり、起動コマンドの2語目以降は引数になる。
#[test]
fn engine_specs_cover_the_documented_four_forms() {
    let random = parse_player_spec("random").expect("the reserved spec must be accepted");
    assert_eq!(random.kind, PlayerKind::Random);

    let commit = parse_player_spec("commit:0045833")
        .expect("a commit spec must be accepted without resolving it");
    assert_eq!(commit.kind, PlayerKind::Commit("0045833".to_owned()));

    // 起動コマンド形式: 2語目以降は起動引数として渡す
    let command = parse_player_spec("target/release/minase --protocol usi --rules R1")
        .expect("a command line spec must be accepted");
    assert_eq!(
        command.kind,
        PlayerKind::Command {
            program: PathBuf::from("target/release/minase"),
            args: vec![
                "--protocol".to_owned(),
                "usi".to_owned(),
                "--rules".to_owned(),
                "R1".to_owned(),
            ],
        }
    );

    let hachu = parse_player_spec("cecp:../hachu-debian/hachu")
        .expect("a CECP command without arguments must be accepted");
    assert_eq!(hachu.text, "cecp:../hachu-debian/hachu");
    assert_eq!(
        hachu.kind,
        PlayerKind::Cecp {
            program: PathBuf::from("../hachu-debian/hachu"),
            args: Vec::new(),
        }
    );
    let minase =
        parse_player_spec("cecp:target/release/minase --protocol cecp --rules engine-default")
            .expect("a CECP command with arguments must be accepted");
    assert_eq!(
        minase.kind,
        PlayerKind::Cecp {
            program: PathBuf::from("target/release/minase"),
            args: vec![
                "--protocol".to_owned(),
                "cecp".to_owned(),
                "--rules".to_owned(),
                "engine-default".to_owned(),
            ],
        }
    );

    // 4形式に含まれない入力は拒否される。旧`depth=N` specの削除は
    // match-harness.md適用範囲に明文がある。空リビジョンとcommit形式への
    // 起動引数付与の拒否は[実装契約](SPEC_UNCLEAR-05関連)。
    for invalid in [
        "",
        "depth=1",
        "depth=2,nodes=1000",
        "commit:",
        "commit:abc --x",
        "cecp:",
        "cecp:   ",
    ] {
        assert!(
            parse_player_spec(invalid).is_err(),
            "spec {invalid:?} must be rejected"
        );
    }
}

// D8-HARN-01境界（sprt.md「エンジンの指定方法」、match-harness.md「CECP
// セッション管理」）と1手固定時間の仕様: CECPに写せないnodes、固定時間以外の
// 秒読み、秒未満の時間単位は解決時にInvalidInputとして拒否する。
#[test]
fn cecp_resolution_rejects_unsupported_search_limits() {
    for limit in [
        parse_search_limit("nodes=1").unwrap(),
        parse_search_limit("depth=1,nodes=1").unwrap(),
        parse_search_limit("time=1000+0,byoyomi=1000").unwrap(),
        parse_search_limit("time=0+1000,byoyomi=1000").unwrap(),
        parse_search_limit("time=0+0,byoyomi=1500").unwrap(),
        parse_search_limit("time=1500+0").unwrap(),
        parse_search_limit("time=1000+1500").unwrap(),
    ] {
        let spec = parse_player_spec("cecp:engine").unwrap();
        let error = resolve_player(spec, limit, None, "R1", Vec::new())
            .err()
            .expect("the unsupported CECP limit must fail resolution");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

// D8-HARN-01/20（sprt.md「エンジンの指定方法」、match-harness.md「CECP
// セッション管理」）と1手固定時間の仕様: 深さ固定、秒単位時間制御、
// 整数秒の秒読みだけを使う固定時間はCECP設定へ解決できる。
#[test]
fn cecp_resolution_accepts_supported_limits_and_marks_the_protocol() {
    let depth = resolve_player(
        parse_player_spec("cecp:engine --option value").unwrap(),
        parse_search_limit("depth=4").unwrap(),
        None,
        "R1",
        Vec::new(),
    )
    .unwrap();
    assert_eq!(depth.protocol, Protocol::Cecp);
    assert_eq!(depth.path, PathBuf::from("engine"));
    assert_eq!(depth.args, ["--option", "value"]);
    assert_eq!(
        probe_engine_defaults(&depth, Duration::from_secs(1)).unwrap(),
        EngineDefaults {
            threads: None,
            hash_mb: Some(256),
        }
    );

    for limit in ["time=61000+2000", "time=0+0,byoyomi=2000"] {
        let time = resolve_player(
            parse_player_spec("cecp:engine").unwrap(),
            parse_search_limit(limit).unwrap(),
            None,
            "R1",
            Vec::new(),
        )
        .unwrap();
        assert_eq!(time.protocol, Protocol::Cecp);
    }
}

// D8-HARN-20（match-harness.md「CECPセッション管理」）: `level`の持ち時間は
// 60秒の倍数なら分だけ、それ以外は分:秒の2桁表記になり、加算は秒で送る。
#[test]
fn cecp_level_formats_minutes_seconds_and_increment() {
    assert_eq!(
        cecp_level_text(TimeControl {
            base_ms: 60_000,
            increment_ms: 1_000,
            byoyomi_ms: 0,
        }),
        "level 0 1 1"
    );
    assert_eq!(
        cecp_level_text(TimeControl {
            base_ms: 61_000,
            increment_ms: 2_000,
            byoyomi_ms: 0,
        }),
        "level 0 1:01 2"
    );
}

// 1手固定時間の仕様: 2000 msの秒読みを`st 2`として送る。
#[test]
fn cecp_fixed_time_formats_byoyomi_in_seconds() {
    assert_eq!(
        cecp_fixed_time_text(TimeControl {
            base_ms: 0,
            increment_ms: 0,
            byoyomi_ms: 2_000,
        }),
        "st 2"
    );
}

// SPEC_UNCLEAR-05 [実装契約]: 不明リビジョンの解決は対局実行前に失敗する。
// 診断文言は契約ではないため種別(Err)だけを検証する。
#[test]
fn unknown_commit_revision_fails_resolution() {
    assert!(
        normalize_commit(
            Path::new(env!("CARGO_MANIFEST_DIR")),
            "definitely-not-a-minase-commit",
        )
        .is_err()
    );
}

// D8-HARN-09(sprt.md・search.md実施状況): 思考制限は
// `depth=N|nodes=M|time=<base_ms>+<inc_ms>[,byoyomi=<ms>]`。
// depthとnodesの併記受理はSPEC_UNCLEAR-08につき[実装契約]。
#[test]
fn search_limits_accept_the_documented_grammar() {
    assert_eq!(
        parse_search_limit("depth=4"),
        Ok(SearchLimit::Fixed {
            depth: Some(4),
            nodes: None
        })
    );
    assert_eq!(
        parse_search_limit("nodes=100000"),
        Ok(SearchLimit::Fixed {
            depth: None,
            nodes: Some(100_000)
        })
    );
    assert_eq!(
        parse_search_limit("time=10000+100"),
        Ok(SearchLimit::Time(TimeControl {
            base_ms: 10_000,
            increment_ms: 100,
            byoyomi_ms: 0,
        }))
    );
    assert_eq!(
        parse_search_limit("time=0+0,byoyomi=1000"),
        Ok(SearchLimit::Time(TimeControl {
            base_ms: 0,
            increment_ms: 0,
            byoyomi_ms: 1_000,
        }))
    );
    // [実装契約] 併記形
    assert_eq!(
        parse_search_limit("depth=3,nodes=400"),
        Ok(SearchLimit::Fixed {
            depth: Some(3),
            nodes: Some(400)
        })
    );
}

// SPEC_UNCLEAR-08 [実装契約]: 文法に合致しない制限は拒否される。
// エラー文言は契約ではない。
#[test]
fn search_limits_reject_malformed_inputs() {
    for invalid in [
        "",
        "byoyomi=1000",
        "time=1000",
        "time=1000+10+5",
        "time=1000+10,depth=1",
        "depth=1,time=1000+10",
        "nodes=1,depth=1",
        "depth=0",
        "nodes=0",
        "depth=1,nodes=2,x=3",
    ] {
        assert!(
            parse_search_limit(invalid).is_err(),
            "limit {invalid:?} must be rejected"
        );
    }
}

// 置換表容量の検証: 0、CECPでの2の冪以外、randomへの指定は拒否する。
#[test]
fn hash_resolution_rejects_invalid_or_inapplicable_sizes() {
    for (spec, hash_mb) in [
        ("engine", 0),
        ("cecp:engine", 0),
        ("cecp:engine", 3),
        ("cecp:engine", 255),
        ("cecp:engine", 257),
        ("random", 256),
    ] {
        let error = resolve_player(
            parse_player_spec(spec).unwrap(),
            parse_search_limit("depth=1").unwrap(),
            Some(hash_mb),
            "R1",
            Vec::new(),
        )
        .err()
        .expect("invalid or inapplicable hash size must fail resolution");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

// CECPの`memory`コマンド: CECPには明示容量を送り、省略時だけ256 MBを使う。
#[test]
fn cecp_memory_command_uses_explicit_size_or_default() {
    for (hash_mb, expected) in [
        (None, "memory 256"),
        (Some(1), "memory 1"),
        (Some(512), "memory 512"),
    ] {
        let player = resolve_player(
            parse_player_spec("cecp:engine").unwrap(),
            parse_search_limit("depth=1").unwrap(),
            hash_mb,
            "R1",
            Vec::new(),
        )
        .unwrap();
        assert_eq!(cecp_memory_text(player.hash_mb), expected);
    }
}

// D8-HARN-10(search.md実施状況): 実測思考時間が`残り時間 + byoyomi`を
// 「超えた」場合だけ時間切れ失権となる。同値は超過ではない。
#[test]
fn time_forfeit_requires_exceeding_remaining_plus_byoyomi() {
    let mut clock = Clock::new(TimeControl {
        base_ms: 1_000,
        increment_ms: 0,
        byoyomi_ms: 50,
    });
    // ちょうど残り+秒読みの消費は失権ではない
    assert_eq!(clock.update(Duration::from_millis(1_050)), Ok(()));
    // 残りは消費に応じて減少する(現在値契約。加算時間0で会計の曖昧さを避ける)
    assert_eq!(clock.remaining_ms(), 0);
    assert_eq!(clock.update(Duration::from_millis(50)), Ok(()));
    assert_eq!(
        clock.update(Duration::from_millis(51)),
        Err(EngineFailure::TimeForfeit)
    );
}

// D8-HARN-10(search.md実施状況＋sprt.mdペア対局): 毎手のgoは両者の時計の
// 現在値をbtime/wtime/binc/winc/byoyomiで送る。ペア内の先後入替で同一
// エンジンの時計がbtime側とwtime側を交差する。
#[test]
fn go_time_arguments_reflect_current_clocks_and_cross_colors() {
    let player_a = SearchLimit::Time(TimeControl {
        base_ms: 10_000,
        increment_ms: 100,
        byoyomi_ms: 1_000,
    });
    let player_b = SearchLimit::Time(TimeControl {
        base_ms: 20_000,
        increment_ms: 200,
        byoyomi_ms: 2_000,
    });
    // プレイヤーAが後手の局では、Aの時計がwtime/winc側に載る
    let clocks = GameClocks::new(Color::White, player_a, player_b);
    assert_eq!(
        clocks.go_text(Color::Black),
        "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 2000"
    );
    // byoyomiは手番側の値を送る
    assert_eq!(
        clocks.go_text(Color::White),
        "btime 20000 wtime 10000 binc 200 winc 100 byoyomi 1000"
    );

    // 現在値契約: 消費後のgoは消費250 msと加算100 msを残り時間へ反映する
    let simple = SearchLimit::Time(TimeControl {
        base_ms: 1_000,
        increment_ms: 100,
        byoyomi_ms: 0,
    });
    let mut clocks = GameClocks::new(Color::Black, simple, simple);
    clocks
        .get_mut(Color::Black)
        .expect("a time-controlled player must have a clock")
        .update(Duration::from_millis(250))
        .expect("a small consumption must not forfeit");
    assert_eq!(
        clocks.go_text(Color::White),
        "btime 850 wtime 1000 binc 100 winc 100 byoyomi 0"
    );

    // 固定制限側のgo引数(sprt.mdの`--each depth=4`等に対応)
    assert_eq!(
        SearchLimit::Fixed {
            depth: Some(3),
            nodes: Some(400),
        }
        .fixed_go_text(),
        Some("depth 3 nodes 400".to_owned())
    );

    // D8-HARN-20（match-harness.md「CECPセッション管理」）: CECPへは固定制限で
    // 十分大きい固定時計を、時間制御で現在値を10 ms単位へ切り捨てて送る。
    let fixed = clocks.think_request(
        Color::Black,
        SearchLimit::Fixed {
            depth: Some(1),
            nodes: None,
        },
    );
    assert_eq!(fixed.own_cs, 3_000_000);
    assert_eq!(fixed.opponent_cs, 3_000_000);
    let timed = clocks.think_request(Color::Black, simple);
    assert_eq!(timed.own_cs, 85);
    assert_eq!(timed.opponent_cs, 100);
}

// 1手固定時間の仕様: CECPの両時計は残り時間と秒読みの合計を送り、
// 時計を持たない相手には0を送る。USIには秒読みを独立して渡す。
#[test]
fn cecp_time_requests_include_byoyomi() {
    let fixed_time = parse_search_limit("time=0+0,byoyomi=2000").unwrap();
    let opponent = parse_search_limit("time=1007+0,byoyomi=1007").unwrap();
    for color in [Color::Black, Color::White] {
        let clocks = GameClocks::new(color, fixed_time, opponent);
        let request = clocks.think_request(color, fixed_time);
        assert_eq!(request.own_cs, 200);
        // 合計2014 msを切り捨てる。個別の切り捨てでは200になってしまう。
        assert_eq!(request.opponent_cs, 201);
        let request = clocks.think_request(color.opposite(), opponent);
        assert_eq!(request.own_cs, 201);
        assert_eq!(request.opponent_cs, 200);
    }

    let clocks = GameClocks::new(
        Color::Black,
        fixed_time,
        parse_search_limit("depth=1").unwrap(),
    );
    let request = clocks.think_request(Color::Black, fixed_time);
    assert_eq!(request.own_cs, 200);
    assert_eq!(request.opponent_cs, 0);
    assert_eq!(
        request.go_text,
        "btime 0 wtime 0 binc 0 winc 0 byoyomi 2000"
    );
}

// D8-HARN-06(1)/D8-HARN-11(sprt.md異常時の裁定節・match-harness.md): 審判層が
// 対局進行の正であり、合法手リストにない`bestmove`は不正着手として分類する。
#[test]
fn referee_rejects_bestmove_outside_the_legal_move_list() {
    let game = Game::new(Rules::ENGINE_DEFAULT);
    let legal = game.legal_moves()[0];
    let legal_text = usi::text(
        game.position(),
        legal,
        &MoveGenerator::new(game.rules().moves),
    )
    .unwrap();
    assert_eq!(
        validate_bestmove(&game, &legal_text, Protocol::Usi),
        Ok(legal)
    );
    // 表記として解釈できない応答
    assert_eq!(
        validate_bestmove(&game, "not-a-move", Protocol::Usi),
        Err(EngineFailure::IllegalMove)
    );
    // 表記としては読めるが初期局面では指せない着手(空升からの移動)
    assert_eq!(
        validate_bestmove(&game, "6f6g", Protocol::Usi),
        Err(EngineFailure::IllegalMove)
    );
}

// D8-HARN-20（match-harness.md「CECPセッション管理」、hachu.md第8節）:
// `@@@@`は合法手中の正準じっとから移動元の内部升番号が最小の手へ一意に割り当てる。
#[test]
fn cecp_null_move_selects_the_lowest_origin_canonical_jitto() {
    let low = Move {
        from: Square::new(2, 3).unwrap(),
        mid: None,
        to: Square::new(2, 3).unwrap(),
        promote: false,
    };
    let high = Move {
        from: Square::new(9, 8).unwrap(),
        mid: None,
        to: Square::new(9, 8).unwrap(),
        promote: false,
    };
    let ordinary = Move {
        from: Square::new(0, 0).unwrap(),
        mid: None,
        to: Square::new(0, 1).unwrap(),
        promote: false,
    };
    assert_eq!(canonical_jitto(&[high, ordinary, low]), Some(low));
    assert_eq!(canonical_jitto(&[ordinary]), None);
}

// D8-HARN-06(2)(3)(sprt.md異常時の裁定節): プロセス終了・パイプ切断は
// クラッシュ、応答期限超過は応答タイムアウトとして分類する。
#[test]
fn response_channel_classifies_disconnect_and_timeout() {
    let (sender, lines) = mpsc::channel();
    drop(sender);
    assert_eq!(
        receive_until(&lines, Duration::from_secs(1), |_| false),
        Err(EngineFailure::Crash)
    );

    let (_sender, lines) = mpsc::channel();
    assert_eq!(
        receive_until(&lines, Duration::from_millis(1), |_| false),
        Err(EngineFailure::Timeout)
    );
}

// D8-HARN-11(match-harness.md USIセッション管理節): ハーネスは`bestmove`を
// 受けて着手を適用する。それ以外の行(infoなど)は応答待ちで読み飛ばす。
#[test]
fn response_channel_waits_for_bestmove_ignoring_other_lines() {
    let (sender, lines) = mpsc::channel();
    sender
        .send(Ok("info depth 1 score cp 0".to_owned()))
        .expect("the receiver must be alive");
    sender
        .send(Ok("bestmove 1a1b".to_owned()))
        .expect("the receiver must be alive");
    assert_eq!(
        receive_until(&lines, Duration::from_secs(1), |line| {
            line.split_whitespace().next() == Some("bestmove")
        }),
        Ok("bestmove 1a1b".to_owned())
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

// D8-HARN-06/20（match-harness.md「CECPセッション管理」「異常時裁定」）:
// CECP応答はpongまで読み切り、分割レグを連結し、拒否を反則へ、結果行だけを投了へ
// 分類する。着手後の結果行はHaChuの王駒捕獲出力なので着手を優先する。
#[test]
fn cecp_response_lines_are_interpreted_through_pong() {
    fn receive(lines_to_send: &[&str]) -> Result<EngineResponse, EngineFailure> {
        let (sender, lines) = mpsc::channel();
        for &line in lines_to_send {
            sender.send(Ok(line.to_owned())).unwrap();
        }
        sender.send(Ok("pong 12".to_owned())).unwrap();
        receive_cecp_response(&lines, Duration::from_secs(1), 12, Instant::now())
            .map(|(response, _)| response)
    }

    assert_eq!(
        receive(&["# debug", "move e7d8,", "move d8d7"]),
        Ok(EngineResponse::Move("e7d8,d8d7".to_owned()))
    );
    assert_eq!(
        receive(&["Illegal move (repetition): e7d8"]),
        Err(EngineFailure::RejectedMove)
    );
    assert_eq!(receive(&["0-1 {resign}"]), Ok(EngineResponse::Resigned));
    assert_eq!(
        receive(&["move c9i3+", "1-0 {royal capture}"]),
        Ok(EngineResponse::Move("c9i3+".to_owned()))
    );
}

// D8-HARN-20（match-harness.md「CECPセッション管理」）: 応答なしのpongは
// プロトコル違反であり、クラッシュ相当に分類する。
#[test]
fn cecp_pong_without_a_move_or_result_is_a_crash() {
    let (sender, lines) = mpsc::channel();
    sender.send(Ok("pong 3".to_owned())).unwrap();
    assert_eq!(
        receive_cecp_response(&lines, Duration::from_secs(1), 3, Instant::now())
            .map(|(response, _)| response),
        Err(EngineFailure::Crash)
    );
}

// D8-HARN-14（sprt.md「異常時の裁定」）: `engine_failures:`は不正着手・
// クラッシュ・応答タイムアウト・時間切れ・着手拒否の理由別件数を報告する。
// 理由別件数の合計は反則負けとして算入された局数と一致する(保存則)。
#[test]
fn failure_reasons_are_counted_separately_and_conserved() {
    let mut counts = FailureCounts::default();
    counts.record(EngineFailure::IllegalMove);
    counts.record(EngineFailure::Crash);
    counts.record(EngineFailure::Timeout);
    counts.record(EngineFailure::TimeForfeit);
    counts.record(EngineFailure::RejectedMove);
    assert_eq!(
        counts,
        FailureCounts {
            illegal_moves: 1,
            crashes: 1,
            timeouts: 1,
            time_forfeits: 1,
            rejected_moves: 1,
        }
    );
    // 集計の合成でも件数は保存される
    let mut total = FailureCounts::default();
    total.add(counts);
    total.add(counts);
    assert_eq!(
        total.illegal_moves
            + total.crashes
            + total.timeouts
            + total.time_forfeits
            + total.rejected_moves,
        10
    );
}

// sprt.md: 各局の勝敗を候補側の得点へ変換し、反則負けも敗北として扱う。
// match-harness.md: 投了を異常件数へ算入しない。
#[test]
fn game_outcomes_map_to_candidate_scores_and_resignation_is_not_failure() {
    let win_black = GameOutcome::Adjudicated(GameResult::Win {
        winner: Color::Black,
        reason: WinReason::RoyalCapture,
    });
    let draw = GameOutcome::Adjudicated(GameResult::Draw {
        reason: DrawReason::Repetition,
    });
    let forfeit_win_black = GameOutcome::Forfeit {
        winner: Color::Black,
        reason: EngineFailure::Crash,
    };
    let resignation_win_black = GameOutcome::Resigned {
        winner: Color::Black,
    };

    // 1局の得点は半点単位: 勝ち2、引き分け1、負け0(候補の色に依存)
    assert_eq!(half_points(win_black, Color::Black), 2);
    assert_eq!(half_points(win_black, Color::White), 0);
    assert_eq!(half_points(draw, Color::Black), 1);
    assert_eq!(half_points(draw, Color::White), 1);
    // 反則負けも通常の勝敗として得点化される
    assert_eq!(half_points(forfeit_win_black, Color::Black), 2);
    assert_eq!(half_points(forfeit_win_black, Color::White), 0);
    assert_eq!(half_points(resignation_win_black, Color::Black), 2);
    assert_eq!(half_points(resignation_win_black, Color::White), 0);

    // D8-HARN-06（match-harness.md「異常時裁定」）: 投了は通常の敗北であり、
    // engine_failuresには算入しない。
    let resigned = PlayedGame::Finished {
        plies: 12,
        outcome: resignation_win_black,
    };
    assert_eq!(
        played_game_text(resigned),
        "plies=12 result=win winner=Black reason=Resigned"
    );
    let mut resignation_failures = FailureCounts::default();
    record_game_failure(resigned, &mut resignation_failures);
    assert_eq!(resignation_failures, FailureCounts::default());
}

// random-play.md: 乱数系列を停止させる0の派生値は非零定数へ置換する。
#[test]
fn pair_seed_replaces_the_zero_output() {
    // 派生値が0になる入力でも非ゼロへ置換される(random-play.mdの
    // 仕様式の逆算により、splitmix64の出力0の原像は0x61C8_8646_80B5_83EB)
    assert_eq!(
        derive_seed(0x61C8_8646_80B5_83EB - 5, 5).get(),
        0x9E37_79B9_7F4A_7C15
    );
}

// D8-HARN-02(sprt.mdペア対局と再現性節): 開始局面は初期局面から8〜12手
// だけランダムに進めて作り、ペアシードから決定的に再現される。
#[test]
fn openings_stay_within_8_to_12_plies_and_vary_between_pairs() {
    let rules = Rules::ENGINE_DEFAULT;
    let base_seed = 20_260_814_u64;
    let mut all_moves = Vec::new();
    for pair_number in 1..=4 {
        let pair_seed = derive_seed(base_seed, pair_number);
        let opening = generate_opening(rules, pair_seed);
        assert!(
            (8..=12).contains(&opening.moves.len()),
            "opening length {} is outside the documented 8..=12 range",
            opening.moves.len()
        );
        // エンジンへ送るUSI表記列は開始手順と同数
        assert_eq!(opening.usi_moves.len(), opening.moves.len());
        all_moves.push(opening.moves);
    }
    // ペア間独立: 異なるペア番号がすべて同じ開始手順なら派生が退化している
    assert!(all_moves.windows(2).any(|pair| pair[0] != pair[1]));
}

// phase2.md §4: 握手後、isreadyより前に、重複する名前も含め列順で送る。
#[test]
fn usi_options_are_sent_in_order_before_isready() {
    let mut player = resolve_player(
        parse_player_spec("python3").unwrap(),
        parse_search_limit("depth=1").unwrap(),
        Some(64),
        "R1",
        vec![
            ("Tune_First".to_owned(), "17".to_owned()),
            ("Tune_Second".to_owned(), "-3".to_owned()),
            ("Tune_First".to_owned(), "19".to_owned()),
        ],
    )
    .unwrap();
    player.args = vec![
        "-u".to_owned(),
        "-c".to_owned(),
        r#"
import sys
expected = [
    'usi',
    'setoption name RuleSet value R1',
    'setoption name USI_Hash value 64',
    'setoption name ResignValue value 99999',
    'setoption name Tune_First value 17',
    'setoption name Tune_Second value -3',
    'setoption name Tune_First value 19',
    'isready',
    'usinewgame',
]
for command in expected:
    actual = sys.stdin.readline().strip()
    assert actual == command, (command, actual)
    if command == 'usi':
        print('usiok', flush=True)
    elif command == 'isready':
        print('readyok', flush=True)
print('initialized', flush=True)
for _ in sys.stdin:
    pass
"#
        .to_owned(),
    ];
    let process = EngineProcess::start(&player, 1, Duration::from_secs(2)).unwrap();
    process.wait_for("initialized").unwrap();
}

// phase2.md §4: CECPで追加のUSI設定があれば、プロセスを起動する前に拒否する。
#[test]
fn cecp_resolution_rejects_nonempty_usi_options() {
    let result = resolve_player(
        parse_player_spec("cecp:engine-that-does-not-exist").unwrap(),
        parse_search_limit("depth=1").unwrap(),
        None,
        "R1",
        vec![("Tune_First".to_owned(), "17".to_owned())],
    );
    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
}

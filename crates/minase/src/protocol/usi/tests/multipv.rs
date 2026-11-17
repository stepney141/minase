//! MultiPVの設定、順位つき出力、および探索終了の各経路を検査する。

use super::*;

fn assert_info_format(output: &str, multipv: bool) {
    let lines: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("info depth "))
        .collect();
    assert_eq!(lines.len(), 2, "{output}");
    for (line, depth) in lines.iter().zip([1, 2]) {
        let prefix = if multipv {
            format!("info depth {depth} multipv 1 score ")
        } else {
            format!("info depth {depth} score ")
        };
        assert!(line.starts_with(&prefix), "{output}");
        assert_eq!(line.contains(" multipv "), multipv, "{output}");
    }
    assert_bestmove_matches_final_pv(output);
}

#[test]
fn multipv_handshake_declares_the_shared_option_after_draw_value_white() {
    let output = single_root_session("usi\n");
    let lines: Vec<_> = output.lines().collect();
    let white = lines
        .iter()
        .position(|line| line.starts_with("option name DrawValueWhite "))
        .unwrap();
    assert_eq!(
        lines[white + 1],
        "option name MultiPV type spin default 1 min 1 max 35856"
    );
}

#[test]
fn multipv_accepts_bounds_and_restores_single_pv_output() {
    assert_info_format(
        &single_root_session("position startpos\ngo depth 2\n"),
        false,
    );
    for value in [1, 2, 35856] {
        let output = single_root_session(&format!(
            "setoption name mUlTiPv value {value}\nposition startpos\ngo depth 2\n"
        ));
        assert!(error_lines(&output).is_empty(), "{output}");
        assert_info_format(&output, value >= 2);
    }
    let output = single_root_session(
        "setoption name MultiPV value 2\nsetoption name MultiPV value 1\nposition startpos\ngo depth 2\n",
    );
    assert!(error_lines(&output).is_empty(), "{output}");
    assert_info_format(&output, false);
}

#[test]
fn multipv_rejects_bad_values_and_preserves_the_previous_setting() {
    for value in [
        "0",
        "35857",
        "abc",
        "1.5",
        "-1",
        "２",
        "9999999999999999999999999999999999999999",
    ] {
        let output = single_root_session(&format!(
            "setoption name MultiPV value 2\nsetoption name MultiPV value {value}\nposition startpos\ngo depth 2\n"
        ));
        assert_eq!(
            error_lines(&output),
            ["info string error: MultiPV must be an integer from 1 to 35856"]
        );
        assert_info_format(&output, true);
    }
    let output = single_root_session(
        "setoption name MultiPV value 2\nsetoption name MultiPV\nposition startpos\ngo depth 2\n",
    );
    assert_eq!(
        error_lines(&output),
        ["info string error: MultiPV requires a value"]
    );
    assert_info_format(&output, true);
}

#[test]
fn multipv_lines_are_ranked_and_share_statistics_even_without_a_score() {
    let context = SearchContext {
        id: 1,
        position: Position::initial(),
        rules: Rules::JCSA,
        infinite: false,
        ponder: false,
        last_info_depth: None,
        show_multipv: true,
    };
    let mut output = Vec::new();
    write_pv_lines(
        &mut output,
        &context,
        10,
        Duration::from_millis(100),
        &[
            PvLine {
                depth: 2,
                score: Some(30),
                moves: vec![],
            },
            PvLine {
                depth: 1,
                score: None,
                moves: vec![],
            },
        ],
    )
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert_eq!(
        text,
        "info depth 2 multipv 1 score cp 30 nodes 10 nps 100 time 100 pv\ninfo depth 1 multipv 2 nodes 10 nps 100 time 100 pv\n"
    );
    assert!(!text.lines().nth(1).unwrap().contains(" score "));
}

#[test]
fn multipv_changes_preserve_tables_and_take_effect_after_the_current_search() {
    let mut protocol = test_protocol();
    run(
        &mut protocol,
        &format!(
            "setoption name RuleSet value lishogi\nsetoption name USI_Hash value 1\n{}\n",
            single_root_position()
        ),
    );
    let output = run(
        &mut protocol,
        "go depth 2\nsetoption name MultiPV value 2\n",
    );
    assert_info_format(&output, false);
    let before = run(&mut protocol, "tt\n");
    assert!(before.contains("current depth "), "{before}");
    protocol
        .searcher
        .as_mut()
        .unwrap()
        .histories_mut_for_test()
        .workers[0][0][60][60] = 123;
    assert_eq!(
        run(&mut protocol, "setoption name MultiPV value 35856\n"),
        ""
    );
    assert_eq!(run(&mut protocol, "tt\n"), before);
    assert_eq!(
        protocol
            .searcher
            .as_ref()
            .unwrap()
            .histories_for_test()
            .workers[0][0][60][60],
        123
    );
    assert_info_format(&run(&mut protocol, "go depth 2\n"), true);
}

/// 本数オプションの境界値を、探索候補が1手の局面で検査する。
fn single_root_position() -> String {
    use minase_core::PieceKind;
    use minase_core::test_util::{position, sq};
    let board = position(
        Color::Black,
        &[
            (sq(10, 11), Color::Black, PieceKind::King),
            (sq(9, 11), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::Black, PieceKind::Pawn),
            (sq(9, 10), Color::Black, PieceKind::Pawn),
            (sq(10, 10), Color::Black, PieceKind::Pawn),
            (sq(11, 10), Color::Black, PieceKind::Pawn),
            (sq(0, 10), Color::Black, PieceKind::Pawn),
            (sq(0, 11), Color::Black, PieceKind::GoldGeneral),
            (sq(11, 0), Color::White, PieceKind::King),
            (sq(6, 3), Color::White, PieceKind::GoldGeneral),
        ],
    );
    assert_eq!(
        Game::from_position(Rules::LISHOGI, board.clone())
            .legal_moves()
            .len(),
        1
    );
    format!("position sfen {} - 1", to_sfen(&board))
}

fn single_root_session(input: &str) -> String {
    lishogi_session(&input.replace("position startpos", &single_root_position()))
}

/// USIのまとめ送信と、根からの合法な読み筋を検査する。
fn assert_ranked_output(output: &str, count: usize) {
    let info: Vec<_> = output
        .lines()
        .filter(|line| line.starts_with("info depth "))
        .collect();
    assert!(!info.is_empty(), "{output}");
    assert_eq!(info.len() % count, 0, "{output}");
    let mut best = None;
    for group in info.chunks(count) {
        let depth = group[0].split_whitespace().nth(2).unwrap();
        let mut first_moves = Vec::new();
        for (rank, line) in group.iter().enumerate() {
            let expected = if count == 1 {
                format!("info depth {depth} score ")
            } else {
                format!("info depth {depth} multipv {} score ", rank + 1)
            };
            assert!(line.starts_with(&expected), "{output}");
            assert_eq!(line.contains(" multipv "), count > 1);
            let (_, pv) = line.split_once(" pv ").unwrap();
            let moves: Vec<_> = pv.split_whitespace().collect();
            assert!(!first_moves.contains(&moves[0]));
            first_moves.push(moves[0]);
            let mut game = Game::new(Rules::LISHOGI);
            for text in &moves {
                let mv = usi::parse(game.position(), text).unwrap();
                assert!(game.legal_moves().contains(&mv));
                game.play(mv).unwrap();
            }
            if rank == 0 {
                best = Some(moves[0]);
            }
        }
    }
    let returned = bestmoves(output);
    assert_eq!(returned.len(), 1, "{output}");
    assert_eq!(returned[0].split_whitespace().nth(1), best);
}

#[test]
fn multipv_natural_stop_and_ponderhit_emit_complete_legal_groups() {
    for count in [1, 3] {
        let output = lishogi_session(&format!(
            "setoption name MultiPV value {count}\nposition startpos\ngo depth 3\n"
        ));
        assert_ranked_output(&output, count);
        for ponder in [false, true] {
            ponder_dialogue(|commands, lines| {
                for command in [
                    format!("setoption name MultiPV value {count}"),
                    "position startpos".to_owned(),
                    if ponder {
                        "go ponder depth 3"
                    } else {
                        "go infinite"
                    }
                    .to_owned(),
                ] {
                    commands.send(Ok(command)).unwrap();
                }
                let mut output = String::new();
                loop {
                    let line = receive_line(lines);
                    assert!(!line.starts_with("bestmove "));
                    let reached = line.starts_with("info depth 3 ")
                        && (count == 1 || line.contains(&format!(" multipv {count} ")));
                    output.push_str(&line);
                    if reached {
                        break;
                    }
                }
                commands
                    .send(Ok(if ponder { "ponderhit" } else { "stop" }.to_owned()))
                    .unwrap();
                loop {
                    let line = receive_line(lines);
                    let done = line.starts_with("bestmove ");
                    output.push_str(&line);
                    if done {
                        break;
                    }
                }
                assert_ranked_output(&output, count);
            });
        }
    }
}

#[test]
fn multipv_finished_prints_all_lines_only_when_the_selected_worker_is_deeper() {
    for final_depth in [2, 3] {
        let mut protocol = test_protocol();
        run(
            &mut protocol,
            "setoption name RuleSet value lishogi\nsetoption name USI_Hash value 1\nsetoption name MultiPV value 3\nposition startpos\n",
        );
        let mut output = Vec::new();
        let mut active = protocol.start_go(&["depth", "1"], &mut output).unwrap();
        // 実ワーカーを完了させ、通知内容だけを契約の固定入力へ置き換える。
        let search_id = loop {
            let Some(ActiveSearch::Running { handle, .. }) = &active else {
                panic!()
            };
            let event = handle
                .events()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            if matches!(event, SearchEvent::Finished { .. }) {
                break event.search_id();
            }
        };
        let roots = Game::new(Rules::LISHOGI).legal_moves();
        let lines = |depth| {
            roots[..3]
                .iter()
                .map(|&mv| PvLine {
                    depth,
                    score: Some(0),
                    moves: vec![mv],
                })
                .collect::<Vec<_>>()
        };
        protocol
            .handle_search_event(
                &mut active,
                SearchEvent::Progress {
                    search_id,
                    nodes: 10,
                    elapsed: Duration::ZERO,
                    lines: lines(2),
                },
                &mut output,
            )
            .unwrap();
        protocol
            .handle_search_event(
                &mut active,
                SearchEvent::Finished {
                    partial_score: None,
                    search_id,
                    best_move: roots[0],
                    score: 0,
                    depth: final_depth,
                    nodes: 20,
                    elapsed: Duration::ZERO,
                    lines: lines(final_depth),
                    stop_reason: StopReason::DepthCompleted,
                },
                &mut output,
            )
            .unwrap();
        let text = String::from_utf8(output).unwrap();
        let expected_groups = if final_depth == 2 { 1 } else { 2 };
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("info depth "))
                .count(),
            3 * expected_groups
        );
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("info depth 3 "))
                .count(),
            if final_depth == 3 { 3 } else { 0 }
        );
        assert_ranked_output(&text, 3);
    }
}

#[test]
fn multipv_repetition_scores_keep_cp_while_ordinary_proofs_use_mate() {
    let context = SearchContext {
        id: 1,
        position: Position::initial(),
        rules: Rules::JCSA,
        infinite: false,
        ponder: false,
        last_info_depth: None,
        show_multipv: true,
    };
    let scores = [
        search::MATE - 5,
        search::REPETITION_WIN - 5,
        -search::REPETITION_WIN + 5,
        -search::MATE + 5,
    ];
    let lines: Vec<_> = scores
        .into_iter()
        .map(|score| PvLine {
            depth: 1,
            score: Some(score),
            moves: vec![],
        })
        .collect();
    let mut output = Vec::new();
    write_pv_lines(
        &mut output,
        &context,
        10,
        Duration::from_millis(100),
        &lines,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    // usi-extensions.md: 王駒捕獲までの5手から、詰み後の2手を引いて3手と表示する。
    let expected = format!(
        "info depth 1 multipv 1 score mate 3 nodes 10 nps 100 time 100 pv\n\
         info depth 1 multipv 2 score cp {} nodes 10 nps 100 time 100 pv\n\
         info depth 1 multipv 3 score cp {} nodes 10 nps 100 time 100 pv\n\
         info depth 1 multipv 4 score mate -3 nodes 10 nps 100 time 100 pv\n",
        search::REPETITION_WIN - 5,
        -search::REPETITION_WIN + 5,
    );
    assert_eq!(output, expected);
}

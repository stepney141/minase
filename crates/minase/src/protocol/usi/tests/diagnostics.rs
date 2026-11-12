//! 診断コマンドの入出力、対局からの独立性、および探索中の受付順序を検査する。

use super::*;

fn count(output: &str, depth: u32) -> u64 {
    output
        .lines()
        .find_map(|line| line.strip_prefix(&format!("info string perft({depth}): ")))
        .unwrap()
        .parse()
        .unwrap()
}

fn assert_ended(output: &str) {
    assert!(!output.contains("error:"), "{output}");
    assert!(
        output.lines().all(|line| line.starts_with("info string ")),
        "{output}"
    );
    assert_eq!(output.lines().last(), Some("info string end"));
}

#[test]
fn perft_matches_moves_and_divide_sums_and_preserves_state() {
    let mut protocol = UsiProtocol::new();
    assert_eq!(
        run(
            &mut protocol,
            "setoption name RuleSet value JCSA\nposition startpos\n"
        ),
        ""
    );
    let before = run(&mut protocol, "state\nd\nmoves\n");
    let moves = moves_sets(&before).remove(0);
    let first = run(&mut protocol, "perft 1\n");
    assert_ended(&first);
    assert_eq!(count(&first, 1), moves.len() as u64);
    let divided = run(&mut protocol, "perft 2 divide\n");
    assert_ended(&divided);
    let branches: Vec<u64> = divided
        .lines()
        .filter(|line| {
            line.starts_with("info string move ") || line.starts_with("info string double ")
        })
        .map(|line| line.rsplit_once(": ").unwrap().1.parse().unwrap())
        .collect();
    assert_eq!(branches.len(), moves.len());
    assert_eq!(branches.iter().sum::<u64>(), count(&divided, 2));
    assert_eq!(
        count(&run(&mut protocol, "perft 2\n"), 2),
        count(&divided, 2)
    );
    assert_eq!(run(&mut protocol, "state\nd\nmoves\n"), before);
}

#[test]
fn perft_zero_counts_the_root_and_emits_no_depth_lines() {
    let output = session(&[RuleCode::R1], "position startpos\nperft 0\n");
    assert_ended(&output);
    assert!(output.contains("info string total nodes: 1\n"));
    assert!(!output.contains("perft("));
}

/// 不正な入力はエラー1行だけを返し、後続の握手を処理する。
#[test]
fn invalid_diagnostic_arguments_leave_the_session_running() {
    for command in [
        "perft",
        "perft x",
        "perft -1",
        "perft 4294967296",
        "perft 深さ",
        "perft 0 divide",
        "perft 1 foo",
        "perft 1 divide divide",
        "perft 1 2",
        "bench depth 0",
        "bench depth",
        "bench threads 0",
        "bench repetitions 0",
        "bench foo 1",
        "bench depth 1 depth 2",
        "bench threads 1 threads 2",
        "bench repetitions 1 repetitions 2",
        "bench depth -1",
        "bench threads 257",
        "bench depth 4294967296",
        "bench threads 18446744073709551616",
        "bench repetitions 18446744073709551616",
        "bench 深さ 1",
    ] {
        let output = session(
            &[RuleCode::R1],
            &format!("position startpos\n{command}\nusi\n"),
        );
        assert!(
            output
                .lines()
                .next()
                .unwrap()
                .starts_with("info string error: "),
            "{command}: {output}"
        );
        assert_eq!(error_lines(&output).len(), 1, "{command}: {output}");
        assert!(!output.contains("info string end"), "{command}: {output}");
        assert_eq!(output.lines().last(), Some("usiok"));
    }
    let output = session(
        &[RuleCode::R1],
        &format!("bench depth {}\nusi\n", search::MAX_PLY + 1),
    );
    assert_eq!(error_lines(&output).len(), 1);
    assert_eq!(output.lines().last(), Some("usiok"));
}

#[test]
fn perft_requires_depth_and_rules() {
    let mut protocol = UsiProtocol::new();
    assert_eq!(
        run(&mut protocol, "perft 1\n"),
        "info string error: required option RuleSet is not set\n"
    );
    assert_eq!(
        run(
            &mut protocol,
            "setoption name RuleSet value JCSA\nperft 1\n"
        ),
        "info string error: perft requires an active or finished game\n"
    );
    assert_eq!(
        run(&mut protocol, "position startpos\nperft\n")
            .lines()
            .count(),
        1
    );
    assert_ended(&run(&mut protocol, "perft 1\n"));
}

/// chu-shogi.md第30条P0/P2で、敵陣内の歩兵の成りの機会が変わる。
#[test]
fn perft_uses_active_move_rules_until_the_next_game() {
    let sfen = "k11/12/4P7/12/9g2/5+o3n2/12/9R2/12/12/12/11K b 7f 41";
    let mut counts = Vec::new();
    for (rules, next) in [
        ("L1,P0,R1,E2", "L1,P2,R1,E2"),
        ("L1,P2,R1,E2", "L1,P0,R1,E2"),
    ] {
        let mut protocol = UsiProtocol::new();
        assert_eq!(
            run(
                &mut protocol,
                &format!(
                    "setoption name RuleSet value {rules}\nposition sfen {sfen}\nsetoption name RuleSet value {next}\n"
                )
            ),
            ""
        );
        let moves = moves_sets(&run(&mut protocol, "moves\n")).remove(0);
        let output = run(&mut protocol, "perft 1\n");
        assert_ended(&output);
        assert_eq!(count(&output, 1), moves.len() as u64);
        counts.push(count(&output, 1));
    }
    assert_ne!(counts[0], counts[1]);
}

#[test]
fn perft_counts_a_finished_game_without_adjudication() {
    let mut protocol = UsiProtocol::new();
    run(
        &mut protocol,
        &format!(
            "setoption name RuleSet value L0,P0,R1,E2\nposition sfen {ROYAL_SFEN} moves 7g7d\n"
        ),
    );
    let before = run(&mut protocol, "state\n");
    assert_eq!(before.trim_end(), ROYAL_FINISHED_STATE);
    assert_ended(&run(&mut protocol, "perft 2\n"));
    assert_eq!(run(&mut protocol, "state\n"), before);
}

/// positionの4欄以上の形式で拡張状態を受け取り、拒否した状態では計数を始めない。
#[test]
fn parse_position_accepts_basic_and_extended_and_rejects_invalid_state() {
    let mut protocol = UsiProtocol::new();
    run(&mut protocol, "setoption name RuleSet value JCSA\n");
    let initial = format!("{INITIAL_BOARD} - 1");
    for sfen in [
        format!("{INITIAL_BOARD} -"),
        format!("{INITIAL_BOARD} 6i 1"),
        "k11/12/4P7/12/12/12/12/12/12/12/12/11K b - 1 8c".into(),
    ] {
        let output = run(&mut protocol, &format!("position sfen {sfen}\nperft 1\n"));
        assert_eq!(error_lines(&output).len(), 2, "{output}");
        assert!(!output.contains("perft(1)"));
        assert!(!output.contains("info string end"));
    }
    assert_eq!(
        run(&mut protocol, &format!("position sfen {initial}\n")),
        ""
    );
    assert_ended(&run(&mut protocol, "perft 1\n"));
}

fn positions(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter(|line| line.starts_with("info string position="))
        .map(|line| line.split_once(" elapsed=").unwrap().0)
        .collect()
}

#[test]
fn bench_needs_no_setup_and_is_reproducible() {
    let mut protocol = UsiProtocol::new();
    let first = run(&mut protocol, "bench depth 1\n");
    assert_ended(&first);
    assert_eq!(
        positions(&first).len(),
        minase_core::test_util::BENCH_SFENS.len()
    );
    assert_eq!(
        first
            .lines()
            .filter(|line| line.starts_with("info string summary:"))
            .count(),
        1
    );
    assert_eq!(
        first
            .lines()
            .filter(|line| line.starts_with("info string accumulators:"))
            .count(),
        1
    );
    let second = run(&mut protocol, "bench threads 1 repetitions 1 depth 1\n");
    assert_ended(&second);
    assert_eq!(positions(&first), positions(&second));
    assert!(protocol.engine().is_err());
}

#[test]
fn bench_repetitions_report_runs_and_median() {
    let output = run(&mut UsiProtocol::new(), "bench repetitions 2 depth 1\n");
    assert_ended(&output);
    assert_eq!(
        output
            .lines()
            .filter(|line| line.starts_with("info string run="))
            .count(),
        2
    );
    assert_eq!(
        output
            .lines()
            .filter(|line| line.starts_with("info string median:"))
            .count(),
        1
    );
    assert_eq!(
        output
            .lines()
            .filter(|line| line.starts_with("info string accumulators:"))
            .count(),
        3
    );
    #[cfg(feature = "search-stats")]
    assert_eq!(
        output
            .lines()
            .filter(|line| line.starts_with("info string stats:"))
            .count(),
        3
    );
}

#[test]
fn bench_and_perft_preserve_game_settings_and_search_state() {
    for diagnostic in ["bench depth 1", "perft 1"] {
        let mut protocol = UsiProtocol::new();
        run(
            &mut protocol,
            concat!(
                "setoption name USI_Hash value 1\nsetoption name Threads value 2\n",
                "setoption name ResignValue value 10000\nsetoption name DrawValueBlack value 12\n",
                "setoption name RuleSet value L1,P0,R2,E2\nposition startpos moves 6i6h\n",
                "setoption name RuleSet value JCSA\nisready\ngo depth 1\n"
            ),
        );
        let before = run(&mut protocol, "state\nd\ntt\n");
        assert!(!before.contains("error:"), "{before}");
        assert_eq!(protocol.searcher.as_ref().unwrap().threads().get(), 2);
        assert_ended(&run(&mut protocol, &format!("{diagnostic}\n")));
        assert_eq!(run(&mut protocol, "state\nd\ntt\n"), before);
        assert_eq!(protocol.searcher.as_ref().unwrap().threads().get(), 2);
        assert_eq!(protocol.resign_value, 10000);
        assert_eq!(protocol.draw_value_black, 12);
        let next = run(&mut protocol, "usinewgame\nposition startpos\nstate\n");
        assert!(next.starts_with("state rules L0,P0,R1,E0 "), "{next}");
    }
}

#[test]
fn bench_works_with_mcts_without_network_preparation() {
    let mut protocol = UsiProtocol::with_searcher(Searcher::mcts().unwrap());
    assert_ended(&run(&mut protocol, "bench depth 1\n"));
    assert!(protocol.engine().is_err());
}

#[test]
fn perft_queued_during_search_follows_bestmove() {
    ponder_dialogue(|commands, lines| {
        for command in [
            "setoption name USI_Hash value 1",
            "position startpos",
            "go infinite",
            "perft 1",
            "go depth 1",
        ] {
            commands.send(Ok(command.into())).unwrap();
        }
        // 重複goへのエラーが、perftを受信済みであることの目印になる。
        loop {
            let line = receive_line(lines);
            assert!(
                !line.contains("perft(") && !line.starts_with("bestmove "),
                "{line}"
            );
            if line == "info string error: go is already running\n" {
                break;
            }
        }
        commands.send(Ok("stop".into())).unwrap();
        loop {
            let line = receive_line(lines);
            assert!(!line.contains("perft("), "{line}");
            if line.starts_with("bestmove ") {
                break;
            }
        }
        assert!(receive_line(lines).starts_with("info string perft(1): "));
        loop {
            let line = receive_line(lines);
            assert!(!line.contains("error:"), "{line}");
            if line == "info string end\n" {
                break;
            }
        }
    });
}

/// 各行が次の行を書き始める前にフラッシュされることを観測する。
#[test]
fn diagnostics_flush_every_line() {
    #[derive(Default)]
    struct FlushedLines {
        pending: Vec<u8>,
        lines: Vec<String>,
    }
    impl Write for FlushedLines {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            assert!(
                !self.pending.contains(&b'\n'),
                "previous line was not flushed"
            );
            self.pending.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            if !self.pending.is_empty() {
                let line = String::from_utf8(std::mem::take(&mut self.pending)).unwrap();
                assert_eq!(line.lines().count(), 1);
                self.lines.push(line);
            }
            Ok(())
        }
    }
    for input in [
        "setoption name RuleSet value JCSA\nposition startpos\nperft 1\n",
        "bench depth 1\n",
    ] {
        let mut output = FlushedLines::default();
        UsiProtocol::new()
            .run(&mut Cursor::new(input), &mut output)
            .unwrap();
        assert!(output.pending.is_empty());
        assert_eq!(output.lines.last().unwrap(), "info string end\n");
    }
}

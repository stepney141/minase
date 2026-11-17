//! MultiPVフェーズ1の契約に従い、設定と1本の読み筋の出力を検査する。

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
    let output = lishogi_session("usi\n");
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
    assert_info_format(&lishogi_session("position startpos\ngo depth 2\n"), false);
    for value in [1, 2, 35856] {
        let output = lishogi_session(&format!(
            "setoption name mUlTiPv value {value}\nposition startpos\ngo depth 2\n"
        ));
        assert!(error_lines(&output).is_empty(), "{output}");
        assert_info_format(&output, value >= 2);
    }
    let output = lishogi_session(
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
        let output = lishogi_session(&format!(
            "setoption name MultiPV value 2\nsetoption name MultiPV value {value}\nposition startpos\ngo depth 2\n"
        ));
        assert_eq!(
            error_lines(&output),
            ["info string error: MultiPV must be an integer from 1 to 35856"]
        );
        assert_info_format(&output, true);
    }
    let output = lishogi_session(
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
    let mut protocol = UsiProtocol::new();
    run(
        &mut protocol,
        "setoption name RuleSet value lishogi\nsetoption name USI_Hash value 1\nposition startpos\n",
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

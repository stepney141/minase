//! デバッグ表示からperftへ局面を受け渡す契約を検査する。

use std::io::Cursor;
use std::process::Command;

use minase::core::rules::parse_rule_set;
use minase::protocol::{Engine, Protocol, UsiProtocol};

/// debugging-tools.md「perftの規則と拡張SFEN」: dの拡張SFENを渡すと深さ1がmovesの件数と一致する。
#[test]
fn perft_accepts_d_output_with_lion_and_deferred_promotion() {
    for rules in ["L1,L2,P1,R1,E2", "L0,P2,R1,E2", "L1,P0,P5,R1,E2"] {
        let mut engine = Engine::new(parse_rule_set(rules).unwrap()).unwrap();
        let mut protocol = UsiProtocol::new(&engine);
        let mut output = Vec::new();
        protocol
            .run(
                &mut engine,
                &mut Cursor::new(concat!(
                    "position sfen k11/12/4P7/12/9g2/5+o3n2/12/9R2/12/12/12/11K b 7f 41 8c\n",
                    "d\nmoves\n"
                )),
                &mut output,
            )
            .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("error:"), "{output}");
        let sfen = output
            .lines()
            .find_map(|line| line.strip_prefix("info string sfen "))
            .unwrap();
        let moves = output
            .lines()
            .find_map(|line| line.strip_prefix("moves "))
            .unwrap();
        let expected = moves.split_whitespace().count();
        // 先獅子と成り権保留が実際に手集合を変える局面を使う。
        assert!(!moves.split_whitespace().any(|mv| mv == "3h3f"));
        let mut without_lion = engine.game().position().clone();
        without_lion.set_lion_capture(None).unwrap();
        let generator = minase::MoveGenerator::new(engine.game().rules().moves);
        let mut unprotected = Vec::new();
        generator.generate_moves(&without_lion, &mut unprotected);
        let capture = minase::notation::usi::parse(&without_lion, "3h3f").unwrap();
        assert!(unprotected.contains(&capture));

        let result = Command::new(env!("CARGO_BIN_EXE_perft"))
            .args(["1", "--rules", rules, "--sfen", sfen])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .starts_with(&format!("perft(1): {expected}\n"))
        );
    }
}

/// debugging-tools.md「perftの規則と拡張SFEN」: 規則の省略はCLIのエラーになる。
#[test]
fn perft_requires_rules() {
    let output = Command::new(env!("CARGO_BIN_EXE_perft"))
        .arg("1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--rules")
    );
    assert!(output.stdout.is_empty());
}

/// debugging-tools.md「perftの規則と拡張SFEN」: 2欄・4欄・省略時は同じ計数行を保ち、不正欄を拒否する。
#[test]
fn perft_accepts_basic_and_four_field_sfen_and_rejects_invalid_state() {
    let board = minase::to_sfen(&minase::Position::initial());
    let mut counts = Vec::new();
    for sfen in [None, Some(board.clone()), Some(format!("{board} - 1"))] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_perft"));
        command.args(["1", "--rules", "engine-default"]);
        if let Some(sfen) = sfen {
            command.args(["--sfen", &sfen]);
        }
        let result = command.output().unwrap();
        assert!(result.status.success());
        let output = String::from_utf8(result.stdout).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("perft(1): "));
        assert!(lines[1].starts_with("total nodes: "));
        assert!(lines[2].starts_with("elapsed: "));
        assert!(lines[3].starts_with("nodes/second: "));
        counts.push(lines[0].to_owned());
    }
    assert!(counts.iter().all(|line| *line == counts[0]));
    for sfen in [
        format!("{board} -"),
        format!("{board} 6i 1"),
        "k11/12/4P7/12/12/12/12/12/12/12/12/11K b - 1 8c".to_owned(),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_perft"))
            .args(["1", "--rules", "engine-default", "--sfen", &sfen])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
    }
}

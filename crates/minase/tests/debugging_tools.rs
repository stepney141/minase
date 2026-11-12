//! デバッグ表示からperftへ局面を受け渡す契約を検査する。

use std::io::{Cursor, Write};
use std::process::{Command, Output, Stdio};

use minase::protocol::{Protocol, UsiProtocol};

fn usi_session(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["--protocol", "usi"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// debugging-tools.md「perftの規則と拡張SFEN」: dの拡張SFENを渡すと深さ1がmovesの件数と一致する。
#[test]
fn perft_accepts_d_output_with_lion_and_deferred_promotion() {
    let rules = "L1,L2,P1,R1,E2";
    let mut protocol = UsiProtocol::new();
    let mut output = Vec::new();
    use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
    use minase_core::test_util::{position_from_codes, sq};
    use minase_core::{
        Color::{Black, White},
        PieceCode,
        PieceKind::*,
        PositionBuilder, Square,
    };
    let pos = position_from_codes(
        Black,
        &[
            (sq(0, 11), PieceCode::new(White, King).unwrap()),
            (sq(11, 0), PieceCode::new(Black, King).unwrap()),
            (sq(4, 9), PieceCode::new(Black, Pawn).unwrap()),
            (sq(9, 7), PieceCode::new(White, GoldGeneral).unwrap()),
            (sq(5, 6), PieceCode::new_promoted(White, Lion).unwrap()),
            (sq(9, 6), PieceCode::new(White, Lion).unwrap()),
            (sq(9, 4), PieceCode::new(Black, Rook).unwrap()),
        ],
    );
    let mut builder = PositionBuilder::new(Black);
    for square in Square::all() {
        if let Some(piece) = pos.piece_at(square) {
            builder.put(square, piece).unwrap();
        }
    }
    builder.mark_promotion_deferred(sq(4, 9)).unwrap();
    let setup = SetupPosition::new(builder.finish().unwrap(), Some(sq(5, 6)), 41).unwrap();
    let input = format!("position sfen {}\nd\nmoves\n", to_extended_sfen(&setup));
    protocol
        .run(
            &mut Cursor::new(format!("setoption name RuleSet value {rules}\n{input}")),
            &mut output,
        )
        .unwrap();
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("error:"), "{output}");
    let sfen = output
        .lines()
        .find_map(|line| line.strip_prefix("sfen "))
        .unwrap();
    let moves = output
        .lines()
        .find_map(|line| line.strip_prefix("moves "))
        .unwrap();
    let expected = moves.split_whitespace().count();
    // 先獅子と成り権保留が実際に手集合を変える局面を使う。
    assert!(!moves.split_whitespace().any(|mv| mv == "3h3f"));
    let mut without_lion = protocol.engine().unwrap().game().position().clone();
    without_lion.set_lion_capture(None).unwrap();
    let generator =
        minase_core::MoveGenerator::new(protocol.engine().unwrap().game().rules().moves);
    let mut unprotected = Vec::new();
    generator.generate_moves(&without_lion, &mut unprotected);
    let capture = minase_core::notation::usi::parse(&without_lion, "3h3f").unwrap();
    assert!(unprotected.contains(&capture));

    let result = usi_session(&format!(
        "setoption name RuleSet value {rules}\nposition sfen {sfen}\nperft 1\nquit\n"
    ));
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .starts_with(&format!("info string perft(1): {expected}\n"))
    );
}

/// 規則の省略はUSIのエラーとなり、セッションは継続する。
#[test]
fn perft_requires_rules() {
    let output = usi_session("perft 1\nusi\nquit\n");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("info string error: required option RuleSet is not set\n"));
    assert!(stdout.ends_with("usiok\n"));
    assert!(!stdout.contains("info string end"));
}

/// 不正な拡張SFENでは計数結果を出力しない。
#[test]
fn perft_rejects_invalid_state_without_output() {
    let board = minase_core::to_sfen(&minase_core::Position::initial());
    let result = usi_session(&format!(
        "setoption name RuleSet value JCSA\nposition sfen {board} -\nperft 1\nquit\n"
    ));
    assert!(result.status.success());
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 2);
    assert!(
        stdout
            .lines()
            .all(|line| line.starts_with("info string error: "))
    );
}

/// benchは対局の初期化なしで成功し、終了コードと終端行の両方を返す。
#[test]
fn bench_succeeds_as_a_usi_command() {
    let result = usi_session("bench depth 1\nquit\n");
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(stdout.lines().last(), Some("info string end"));
}

/// 診断はUSIだけから受け付け、削除したサブコマンドは引数エラーにする。
#[test]
fn diagnostic_subcommands_are_rejected() {
    for diagnostic in [vec!["bench"], vec!["perft", "1"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_minase"))
            .arg("dev")
            .args(diagnostic)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stdout.is_empty());
    }
}

/// 実行時の失敗は終端行を出さず、後続のコマンドを処理せずに終了コード1を返す。
#[test]
fn diagnostic_runtime_failure_ends_the_process() {
    let result = usi_session(&format!(
        "bench depth 1 repetitions {}\nusi\nquit\n",
        usize::MAX
    ));
    assert_eq!(result.status.code(), Some(1));
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    assert!(
        stdout.starts_with("info string error: bench allocation failed: "),
        "{stdout}"
    );
}

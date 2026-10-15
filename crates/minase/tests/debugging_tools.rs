//! デバッグ表示からperftへ局面を受け渡す契約を検査する。

use std::io::Cursor;
use std::process::Command;

use minase::protocol::{Engine, Protocol, UsiProtocol};
use minase_core::rules::parse_rule_set;

/// debugging-tools.md「perftの規則と拡張SFEN」: dの拡張SFENを渡すと深さ1がmovesの件数と一致する。
#[test]
fn perft_accepts_d_output_with_lion_and_deferred_promotion() {
    let rules = "L1,L2,P1,R1,E2";
    let mut engine = Engine::new(parse_rule_set(rules).unwrap()).unwrap();
    let mut protocol = UsiProtocol::new(&engine);
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
        .run(&mut engine, &mut Cursor::new(input), &mut output)
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
    let mut without_lion = engine.game().position().clone();
    without_lion.set_lion_capture(None).unwrap();
    let generator = minase_core::MoveGenerator::new(engine.game().rules().moves);
    let mut unprotected = Vec::new();
    generator.generate_moves(&without_lion, &mut unprotected);
    let capture = minase_core::notation::usi::parse(&without_lion, "3h3f").unwrap();
    assert!(unprotected.contains(&capture));

    let result = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["dev", "perft"])
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

/// debugging-tools.md「perftの規則と拡張SFEN」: 規則の省略はCLIのエラーになる。
#[test]
fn perft_requires_rules() {
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["dev", "perft"])
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

/// 不正な拡張SFENでは計数結果を出力せず終了する。
#[test]
fn perft_rejects_invalid_state_without_output() {
    let board = minase_core::to_sfen(&minase_core::Position::initial());
    let result = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args([
            "dev",
            "perft",
            "1",
            "--rules",
            "engine-default",
            "--sfen",
            &format!("{board} -"),
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
}

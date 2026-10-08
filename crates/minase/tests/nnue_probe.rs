//! nnue_probeのJSON契約をプロセス境界で検査する。

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;
use std::process::Command;

use minase::training::records::{Header, Outcome, Record, Writer};
use minase_core::notation::usi;
use minase_core::rules::parse_rule_set;
use minase_core::{
    Color, Game, MoveGenerator, PieceCode, PieceKind, Position, PositionBuilder, Rules, Square,
};
use serde_json::Value;

/// 歩兵が歩兵を取りながら敵陣へ入り、成りと不成を選べる局面。
fn promotion_position(side: Color) -> Position {
    let mut builder = PositionBuilder::new(side);
    for (file, rank, color, kind) in [
        (0, 0, Color::Black, PieceKind::King),
        (11, 11, Color::White, PieceKind::King),
        (5, 7, Color::Black, PieceKind::Pawn),
        (5, 8, Color::White, PieceKind::Pawn),
    ] {
        let (rank, color) = if side == Color::Black {
            (rank, color)
        } else {
            (11 - rank, color.opposite())
        };
        builder
            .put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, kind).unwrap(),
            )
            .unwrap();
    }
    builder.finish().unwrap()
}

fn probe(positions: &Path, weights: &str, flags: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["dev", "nnue-probe", "--net", weights, "--positions"])
        .arg(positions)
        .args(flags)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// 全合法手を欠落・重複なく返し、成りの選別と着手前視点の差分を保存する。
#[test]
fn flags_preserve_json_fields_and_side_relative_deltas() {
    let path = std::env::temp_dir().join(format!("minase-nnue-probe-{}.bin", std::process::id()));
    let positions = [
        Position::initial(),
        promotion_position(Color::Black),
        promotion_position(Color::White),
    ];
    let header = Header::new("engine-default".into(), "0".repeat(40), [0; 32], 1, 0, 0).unwrap();
    let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
    for position in &positions {
        writer
            .write_record(&Record::from_position(position, 0, Outcome::Draw, 1, 0))
            .unwrap();
    }
    writer.finish().unwrap();
    let nnue = minase::eval::weights().unwrap();
    let rules = Rules::from_codes(&parse_rule_set("engine-default").unwrap()).unwrap();
    let generator = MoveGenerator::new(rules.moves);
    for flags in [
        vec![],
        vec!["--promotions"],
        vec!["--moves"],
        vec!["--promotions", "--moves"],
    ] {
        let result = probe(
            &path,
            concat!(env!("CARGO_MANIFEST_DIR"), "/nets/nnue.bin"),
            &flags,
        );
        assert_eq!(result.as_array().unwrap().len(), positions.len());
        for (index, position) in positions.iter().enumerate() {
            let entry = &result[index];
            assert_eq!(entry["index"], index);
            let before = minase::eval::evaluate(&nnue, position);
            assert_eq!(entry["eval"], before);
            assert!(entry.get("eval_pst").is_none());
            assert_eq!(
                entry["eval_opposite"],
                minase::eval::nnue::evaluate_for_diagnostics(
                    &nnue,
                    position,
                    position.side_to_move().opposite()
                )
            );
            let game = Game::from_position(rules, position.clone());
            let expected: BTreeMap<_, _> = game
                .legal_moves()
                .into_iter()
                .map(|mv| {
                    let mut after = game.clone();
                    after.play(mv).unwrap();
                    let delta = -minase::eval::evaluate(&nnue, after.position()) - before;
                    (
                        usi::text(position, mv, &generator).unwrap(),
                        (mv.promote, delta),
                    )
                })
                .collect();
            if index > 0 {
                assert!(expected.values().any(|&(promote, _)| promote));
            }
            for (field, flag, only_promotions) in [
                ("moves", "--moves", false),
                ("promotions", "--promotions", true),
            ] {
                if !flags.contains(&flag) {
                    assert!(entry.get(field).is_none());
                    continue;
                }
                let rows = entry[field].as_array().unwrap();
                assert_eq!(
                    rows.len(),
                    expected
                        .values()
                        .filter(|&&(promote, _)| !only_promotions || promote)
                        .count()
                );
                let mut seen = std::collections::BTreeSet::new();
                for row in rows {
                    let text = row["move"].as_str().unwrap();
                    assert!(seen.insert(text));
                    let &(promote, delta) = expected.get(text).unwrap();
                    assert!(!only_promotions || promote);
                    assert_eq!(row["delta"], delta);
                    assert!(
                        row.as_object()
                            .unwrap()
                            .keys()
                            .all(|key| !key.ends_with("_pst"))
                    );
                    let mv = usi::parse(position, text).unwrap();
                    let mut after = game.clone();
                    after.play(mv).unwrap();
                    let same = minase::eval::nnue::evaluate_for_diagnostics(
                        &nnue,
                        after.position(),
                        position.side_to_move(),
                    );
                    let opposite = minase::eval::evaluate(&nnue, after.position());
                    assert_eq!(row["after_same"], same);
                    assert_eq!(row["after_opposite"], opposite);
                    assert_eq!(row["placement"], same - before);
                    assert_eq!(row["turn"], -opposite - same);
                    assert_eq!(row["after"]["eval"], opposite);
                    assert!(row["after"].get("eval_pst").is_none());
                    assert_eq!(
                        row["capture"],
                        after.position().occupied().popcount() < position.occupied().popcount()
                    );
                    assert_eq!(row["promote"], mv.promote);
                    assert_eq!(row["terminal"], after.result().is_some());
                    let encoded =
                        Record::from_position(after.position(), 0, Outcome::Draw, 1, 0).encode();
                    assert_eq!(row["after"]["board"], serde_json::json!(encoded[..144]));
                    assert_eq!(row["after"]["stm"], encoded[144]);
                    assert_eq!(row["after"]["lion"], encoded[145]);
                }
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}

/// 不正な復元局面を省略しても、後続の入力番号と評価は保存する。
#[test]
fn skip_invalid_preserves_indices_and_malformed_network_fails() {
    use minase::training::records::{HEADER_LEN, RECORD_LEN};
    let path = std::env::temp_dir().join(format!(
        "minase-nnue-probe-invalid-{}.bin",
        std::process::id()
    ));
    let header = Header::new("engine-default".into(), "0".repeat(40), [0; 32], 1, 0, 0).unwrap();
    let mut writer = Writer::new(File::create(&path).unwrap(), header).unwrap();
    let position = Position::initial();
    for _ in 0..3 {
        writer
            .write_record(&Record::from_position(&position, 0, Outcome::Draw, 1, 0))
            .unwrap();
    }
    writer.finish().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    // 先獅子のない局面に、麒麟の成りで生じた先獅子という矛盾を与える。
    bytes[HEADER_LEN + RECORD_LEN + 145] = 60;
    bytes[HEADER_LEN + RECORD_LEN + 146] = 1;
    std::fs::write(&path, bytes).unwrap();
    let net = concat!(env!("CARGO_MANIFEST_DIR"), "/nets/nnue.bin");
    let result = probe(&path, net, &["--skip-invalid"]);
    assert_eq!(result.as_array().unwrap().len(), 3);
    for i in [0, 2] {
        assert_eq!(result[i]["index"], i);
        assert_eq!(
            result[i]["eval"],
            minase::eval::evaluate(&minase::eval::weights().unwrap(), &position)
        );
    }
    assert_eq!(result[1]["index"], 1);
    assert!(result[1]["skipped"].is_string());
    assert!(result[1].get("eval").is_none());
    let fail = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["dev", "nnue-probe", "--net", net, "--positions"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(fail.status.code(), Some(1));
    let invalid_net = path.with_extension("mnue");
    std::fs::write(&invalid_net, b"MNUE").unwrap();
    let fail = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["dev", "nnue-probe", "--net"])
        .arg(&invalid_net)
        .arg("--positions")
        .arg(&path)
        .arg("--skip-invalid")
        .output()
        .unwrap();
    assert_eq!(fail.status.code(), Some(1));
    std::fs::remove_file(invalid_net).unwrap();
    std::fs::remove_file(path).unwrap();
}

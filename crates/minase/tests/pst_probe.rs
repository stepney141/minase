//! pst_probeのJSON契約をプロセス境界で検査する。

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
        .args(["dev", "pst-probe", "--pst", weights, "--positions"])
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
    let path = std::env::temp_dir().join(format!("minase-pst-probe-{}.bin", std::process::id()));
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
            concat!(env!("CARGO_MANIFEST_DIR"), "/nets/pst-init.bin"),
            &flags,
        );
        assert_eq!(result.as_array().unwrap().len(), positions.len());
        for (index, position) in positions.iter().enumerate() {
            let entry = &result[index];
            assert_eq!(entry["index"], index);
            assert_eq!(entry["eval"], 0);
            assert_eq!(entry["eval_pst"], 0);
            let game = Game::from_position(rules, position.clone());
            let expected: BTreeMap<_, _> = game
                .legal_moves()
                .into_iter()
                .map(|mv| {
                    // 固定した初期駒価値は歩兵100、金将378。成り捕獲は100+(378−100)。
                    let delta = if mv.promote {
                        378
                    } else if position
                        .piece_at(mv.to)
                        .is_some_and(|piece| piece.color() != Some(position.side_to_move()))
                    {
                        100
                    } else {
                        0
                    };
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
                    assert_eq!(row["delta_pst"], delta);
                }
            }
        }
    }
    // 全特徴の第1成分を2、指数を1にする。どの特徴対も補正は1cp。
    // n特徴のFM補正はn(n-1)/2となり、学習済みの重みに依存しない。
    let fm_path = path.with_extension("mnpt");
    let mut bytes = include_bytes!("../nets/pst-init.bin").to_vec();
    bytes[4..8].copy_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&32_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&[1; 32]);
    for _ in 0..13_680 {
        bytes.extend_from_slice(&2_i16.to_le_bytes());
        bytes.extend_from_slice(&[0; 62]);
    }
    use sha2::{Digest, Sha256};
    let checksum = Sha256::digest(&bytes[80..]);
    bytes[48..80].copy_from_slice(&checksum);
    std::fs::write(&fm_path, bytes).unwrap();
    let candidate = probe(
        &path,
        fm_path.to_str().unwrap(),
        &["--moves", "--promotions"],
    );
    for (index, position) in positions.iter().enumerate() {
        let n = i64::from(position.occupied().popcount());
        let correction = n * (n - 1) / 2;
        assert_eq!(candidate[index]["eval"], correction);
        assert_eq!(candidate[index]["eval_pst"], 0);
        assert_eq!(candidate[index]["eval_opposite"], correction);
        let game = Game::from_position(rules, position.clone());
        for row in candidate[index]["moves"].as_array().unwrap() {
            let mv = usi::parse(position, row["move"].as_str().unwrap()).unwrap();
            let mut after = game.clone();
            after.play(mv).unwrap();
            let captured = i64::from(after.position().occupied().popcount()) < n;
            let material_delta = if mv.promote {
                378
            } else if captured {
                100
            } else {
                0
            };
            let after_n = n - i64::from(captured);
            let after_correction = after_n * (after_n - 1) / 2;
            assert_eq!(row["capture"], captured);
            assert_eq!(row["promote"], mv.promote);
            assert_eq!(row["terminal"], after.result().is_some());
            assert_eq!(row["delta_pst"], material_delta);
            assert_eq!(row["delta"], material_delta - correction - after_correction);
            assert_eq!(
                row["placement"],
                material_delta + after_correction - correction
            );
            assert_eq!(row["turn"], -2 * after_correction);
            assert_eq!(row["after_same"], material_delta + after_correction);
            assert_eq!(row["after_opposite"], -material_delta + after_correction);
            assert_eq!(row["after"]["eval"], row["after_opposite"]);
            assert_eq!(row["after"]["eval_pst"], -material_delta);
            let encoded = Record::from_position(after.position(), 0, Outcome::Draw, 1, 0).encode();
            assert_eq!(row["after"]["board"], serde_json::json!(encoded[..144]));
            assert_eq!(row["after"]["stm"], encoded[144]);
            assert_eq!(row["after"]["lion"], encoded[145]);
            if mv.promote {
                assert!(
                    candidate[index]["promotions"]
                        .as_array()
                        .unwrap()
                        .contains(row)
                );
            }
        }
    }
    std::fs::remove_file(fm_path).unwrap();
    std::fs::remove_file(path).unwrap();
}

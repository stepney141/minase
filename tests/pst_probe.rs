//! pst_probeのJSON契約をプロセス境界で検査する。

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;
use std::process::Command;

use minase::core::rules::parse_rule_set;
use minase::eval::training_data::{Header, Outcome, Record, Writer};
use minase::notation::usi;
use minase::{
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
    let output = Command::new(env!("CARGO_BIN_EXE_pst_probe"))
        .args(["--pst", weights, "--positions"])
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
    // 学習済みFMの初期局面評価はPython参照値52cp、固定PSTは67cp。
    let candidate = probe(
        &path,
        concat!(env!("CARGO_MANIFEST_DIR"), "/nets/pst.bin"),
        &["--moves", "--promotions"],
    );
    assert_eq!(candidate[0]["eval"], 52);
    assert_eq!(candidate[0]["eval_pst"], 67);
    // Python参照で、両手番の歩兵捕獲はFM込み165cp、成り捕獲は360cp。
    for index in [1, 2] {
        assert_eq!(candidate[index]["eval"], 34);
        assert_eq!(candidate[index]["eval_pst"], 184);
        let mut pawn_moves = 0;
        for row in candidate[index]["moves"].as_array().unwrap() {
            let mv = usi::parse(&positions[index], row["move"].as_str().unwrap()).unwrap();
            if positions[index].piece_at(mv.from).unwrap().kind() != Some(PieceKind::Pawn) {
                continue;
            }
            pawn_moves += 1;
            let (delta, delta_pst) = if mv.promote { (360, 268) } else { (165, -159) };
            assert_eq!(row["delta"], delta);
            assert_eq!(row["delta_pst"], delta_pst);
            if mv.promote {
                assert_eq!(candidate[index]["promotions"], serde_json::json!([row]));
            }
        }
        assert_eq!(pawn_moves, 2);
    }
    std::fs::remove_file(path).unwrap();
}

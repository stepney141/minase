//! 段階9のMNKF交換形式をCLIの出力で検証する。

use std::fs;
use std::io::Cursor;
use std::process::Command;

use minase::eval::training_data::{Header, Outcome, Record, Writer};
use minase::{Color, PieceCode, PieceKind, PositionBuilder, Square};
use sha2::{Digest, Sha256};

#[test]
fn probe_writes_the_specified_header_digest_and_ordered_rows() {
    // 指示書の交換形式: 56バイトのヘッダと、入力順の24バイト行。
    // 盤端と中央の孤立した王だけなら、項目1「双方の歩がない」以外は0。
    let directory = std::env::temp_dir().join(format!("minase-king-probe-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let input = directory.join("input.bin");
    let output = directory.join("features.bin");
    let header = Header::new("lishogi".into(), "0".repeat(40), [0; 32], 1, 1, 0).unwrap();
    let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
    for (color, coordinate) in [(Color::Black, (0, 0)), (Color::White, (5, 5))] {
        let mut builder = PositionBuilder::new(color);
        builder
            .put(
                Square::new(coordinate.0, coordinate.1).unwrap(),
                PieceCode::new(color, PieceKind::King).unwrap(),
            )
            .unwrap();
        let record = Record::from_position(&builder.finish().unwrap(), 0, Outcome::Draw, 1, 0);
        writer.write_record(&record).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    fs::write(&input, &bytes).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_pst_probe"))
        .args([
            "--pst",
            concat!(env!("CARGO_MANIFEST_DIR"), "/nets/pst.bin"),
            "--positions",
        ])
        .arg(&input)
        .arg("--king-features")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    let actual = fs::read(&output).unwrap();
    assert_eq!(actual.len(), 56 + 2 * 24);
    assert_eq!(&actual[..4], b"MNKF");
    for (start, expected) in [(4, 1_u32), (8, 1), (12, 24)] {
        assert_eq!(&actual[start..start + 4], &expected.to_le_bytes());
    }
    assert_eq!(&actual[16..24], &2_u64.to_le_bytes());
    assert_eq!(&actual[24..56], Sha256::digest(&bytes).as_slice());
    let mut expected = [0_u8; 48];
    expected[3..6].copy_from_slice(&[1, 1, 1]);
    expected[27..30].copy_from_slice(&[1, 2, 2]);
    assert_eq!(&actual[56..], &expected);
    // 入力と同じ出力パスは拒否し、入力の切り詰めを起こさない。
    let rejected = Command::new(env!("CARGO_BIN_EXE_pst_probe"))
        .args(["--pst", "unused", "--positions"])
        .arg(&input)
        .arg("--king-features")
        .arg(&input)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert_eq!(fs::read(&input).unwrap(), bytes);
}

#[test]
fn json_probe_reports_full_and_pst_scores_and_features_before_and_after_promotion() {
    // 段階9のJSON契約。酔象の成りで自王駒が2枚になり、該当側の特徴だけが消える。
    let directory = std::env::temp_dir().join(format!("minase-king-json-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let input = directory.join("input.bin");
    let weights = directory.join("weights.bin");
    let mut bytes = include_bytes!("../nets/pst-init.bin").to_vec();
    for column in 0..24 {
        for (endpoint, weight) in [(0, 10 + column as i16), (1, -30 - column as i16)] {
            let offset = 80 + (endpoint * 13704 + 13680 + column) * 2;
            bytes[offset..offset + 2].copy_from_slice(&weight.to_le_bytes());
        }
    }
    let checksum = Sha256::digest(&bytes[80..]);
    bytes[48..80].copy_from_slice(&checksum);
    fs::write(&weights, bytes).unwrap();
    let mut builder = PositionBuilder::new(Color::Black);
    for (file, rank, color, kind) in [
        (0, 0, Color::Black, PieceKind::King),
        (11, 11, Color::White, PieceKind::King),
        (5, 7, Color::Black, PieceKind::DrunkElephant),
        (0, 10, Color::White, PieceKind::Pawn),
    ] {
        builder
            .put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, kind).unwrap(),
            )
            .unwrap();
    }
    let position = builder.finish().unwrap();
    let header = Header::new("lishogi".into(), "0".repeat(40), [0; 32], 1, 1, 0).unwrap();
    let mut writer = Writer::new(Cursor::new(Vec::new()), header).unwrap();
    writer
        .write_record(&Record::from_position(&position, 0, Outcome::Draw, 1, 0))
        .unwrap();
    fs::write(&input, writer.finish().unwrap().into_inner()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_pst_probe"))
        .arg("--pst")
        .arg(weights)
        .arg("--positions")
        .arg(input)
        .arg("--promotions")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let row = &rows[0];
    let check = |row: &serde_json::Value, material: i64| {
        assert_eq!(row["pst_eval"], material);
        let features = row["king_features"].as_array().unwrap();
        assert_eq!(features.len(), 24);
        let mut mg = material * 8;
        let mut eg = material * 8;
        for (column, value) in features.iter().enumerate() {
            let value = value.as_i64().unwrap();
            mg += value * (10 + column as i64);
            eg += value * (-30 - column as i64);
        }
        // 駒は4枚。仕様の補間係数は2と88、除数は720。
        assert_eq!(row["eval"], (2 * mg + 88 * eg) / 720);
        assert_ne!(row["eval"], row["pst_eval"]);
    };
    check(row, 403);
    let promotions = row["promotions"].as_array().unwrap();
    assert!(!promotions.is_empty());
    for promotion in promotions {
        let after = &promotion["after"];
        check(after, -2500);
        assert_eq!(after["stm"], 1);
        for column in 12..24 {
            assert_eq!(after["king_features"][column], 0);
        }
        assert_eq!(
            promotion["delta"].as_i64().unwrap(),
            -after["eval"].as_i64().unwrap() - row["eval"].as_i64().unwrap()
        );
    }
}

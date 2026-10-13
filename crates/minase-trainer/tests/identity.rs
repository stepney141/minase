//! 入力の識別は形式のヘッダと長さに従い、ファイル全体の内容には依存しない。
use minase::training::records::{Header, Origin, Outcome, Record, Writer};
use minase_core::Position;
use minase_trainer::identity::file_identity;
use serde_json::json;
use std::fs;

#[test]
fn identities_use_paths_lengths_and_header_fields() {
    let directory = std::env::temp_dir().join(format!("minase-identities-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("data.bin");
    let header = Header::new("L0,P0,R1,E0".into(), "a".repeat(40), [7; 32], 200, 11, 0).unwrap();
    let mut writer = Writer::new(fs::File::create(&path).unwrap(), header).unwrap();
    writer
        .write_record(&Record::from_position(
            &Position::initial(),
            123,
            Outcome::Win,
            1,
            0,
            Origin::RandomSelfplay,
        ))
        .unwrap();
    writer.finish().unwrap();
    let identity = serde_json::to_value(file_identity(&path).unwrap()).unwrap();
    assert_eq!(
        identity,
        json!({
            "path": path.canonicalize().unwrap(), "byte_length": 296,
            "header": {"rule_set": "L0,P0,R1,E0", "generation_commit": "a".repeat(40),
                "network_checksum": "07".repeat(32), "teacher_nodes": 200, "seed": 11, "record_count": 1}
        })
    );
    let mut bytes = fs::read(&path).unwrap();
    bytes[136 + 147..136 + 149].copy_from_slice(&456_i16.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        serde_json::to_value(file_identity(&path).unwrap()).unwrap(),
        identity
    );
    // 重みの識別処理はヘッダに記録された検査和を読む。本体の検査は重みの読み手が行う。
    for (magic, length, offset) in [(b"MNPT", 80, 48), (b"MNUE", 192, 144)] {
        let mut bytes = vec![0; length];
        bytes[..4].copy_from_slice(magic);
        bytes[offset..offset + 32].fill(9);
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            serde_json::to_value(file_identity(&path).unwrap()).unwrap(),
            json!({"path": path.canonicalize().unwrap(), "checksum": "09".repeat(32)})
        );
    }
    let bytes = minase_trainer::pst::encode_pst_npz(&vec![[0.0; 2]; 13680]).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert_eq!(
        serde_json::to_value(file_identity(&path).unwrap()).unwrap(),
        json!({"path": path.canonicalize().unwrap(), "byte_length": bytes.len()})
    );
    fs::write(&path, b"MNSD").unwrap();
    assert!(file_identity(&path).is_err());
    fs::write(&path, b"????").unwrap();
    assert!(file_identity(&path).is_err());
    fs::remove_dir_all(directory).unwrap();
}

//! tatara形式は全バイト、他の入力はヘッダと長さで識別する。
#[path = "common/mnsd.rs"]
mod mnsd;
use minase::training::records::{Header, Origin};
use minase_core::Position;
use minase_trainer::data::{Outcome, Sample};
use minase_trainer::identity::file_identity;
use serde_json::json;
use std::{fs, path::PathBuf};

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "minase-identities-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn identities_use_paths_lengths_and_header_fields() {
    let directory = TempDirectory::new();
    let path = directory.0.join("data.bin");
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "a".repeat(40),
        [7; 32],
        minase::training::records::TeacherKind::AlphaBeta,
        200,
        11,
    )
    .unwrap();
    let record = Sample::from_position(
        &Position::initial(),
        123,
        Outcome::Win,
        1,
        0,
        Origin::RandomSelfplay,
    )
    .unwrap();
    fs::write(&path, mnsd::independent(header, &[record])).unwrap();
    let identity = serde_json::to_value(file_identity(&path).unwrap()).unwrap();
    assert_eq!(
        identity,
        json!({
            "path": path.canonicalize().unwrap(), "byte_length": 320,
            "header": {"rule_set": "L0,P0,R1,E0", "generation_commit": "a".repeat(40),
                "network_checksum": "07".repeat(32), "teacher_kind": 0, "teacher_search_budget": 200, "seed": 11, "game_count": 1, "entry_count": 1}
        })
    );
    let mut bytes = fs::read(&path).unwrap();
    bytes[152 + 161 + 2..152 + 161 + 4].copy_from_slice(&456_i16.to_le_bytes());
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        serde_json::to_value(file_identity(&path).unwrap()).unwrap(),
        identity
    );
    // 重みの識別処理はヘッダに記録された検査和を読む。本体の検査は重みの読み手が行う。
    let mut bytes = vec![0; 80];
    bytes[..4].copy_from_slice(b"MNPT");
    bytes[48..80].fill(9);
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        serde_json::to_value(file_identity(&path).unwrap()).unwrap(),
        json!({"path": path.canonicalize().unwrap(), "checksum": "09".repeat(32)})
    );
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
}

#[test]
fn tatara_identity_hashes_header_and_payload_for_both_versions() {
    use minase_trainer::nnue::layerstack::{Parameters, Scale, export};
    use sha2::{Digest, Sha256};
    let directory = TempDirectory::new();
    let path = directory.0.join("net.bin");
    let scale = Scale::new(679.03).unwrap();
    let original = export(&Parameters::initialize(1, scale), scale).unwrap();
    for version in [0x7AF32F21_u32, 0x7AF32F16] {
        let mut bytes = original.clone();
        bytes[..4].copy_from_slice(&version.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        let initial = serde_json::to_value(file_identity(&path).unwrap()).unwrap();
        assert_eq!(
            initial,
            json!({"path": path.canonicalize().unwrap(), "checksum": format!("{:x}", Sha256::digest(&bytes))})
        );
        // 識別は妥当性検査と分離し、ヘッダと本体のいずれの変更も検出する。
        for offset in [8, bytes.len() - 1] {
            bytes[offset] ^= 1;
            fs::write(&path, &bytes).unwrap();
            let changed = serde_json::to_value(file_identity(&path).unwrap()).unwrap();
            assert_ne!(changed, initial);
            assert_eq!(changed["checksum"], format!("{:x}", Sha256::digest(&bytes)));
            bytes[offset] ^= 1;
        }
    }
    fs::write(&path, b"invalid network").unwrap();
    assert_eq!(
        file_identity(&path).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
}

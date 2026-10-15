//! 学習器間で共有する合成記録と成果物の読み込み。
use super::{mnsd, position};
use minase::training::records::Header;
use minase_core::{Color, PieceKind};
use minase_trainer::data::{Outcome, Sample};
use minase_trainer::nn::npz;
use serde_json::Value;
use std::{fs, path::Path};

// PST単独は先獅子と検証専用の銀を持たず、FMを使う2学習器は両方を持つ。
pub fn records(large_evaluation: bool, fm_features: bool) -> Vec<u8> {
    let header = Header::new(
        "L0,P0,R1,E0".into(),
        "1".repeat(40),
        [2; 32],
        minase::training::records::TeacherKind::AlphaBeta,
        200,
        11,
    )
    .unwrap();
    let mut samples = Vec::new();
    for game in 0..40 {
        for ply in 0..3 {
            let side = if ply % 2 == 0 {
                Color::Black
            } else {
                Color::White
            };
            let mut pieces = vec![
                (4, 0, Color::Black, PieceKind::King),
                (7, 11, Color::White, PieceKind::King),
                ((game % 12) as u8, 3, Color::Black, PieceKind::Pawn),
                (((game + 6) % 12) as u8, 8, Color::White, PieceKind::Pawn),
                (((game + 3) % 12) as u8, 6, Color::Black, PieceKind::Rook),
            ];
            if large_evaluation {
                pieces.extend([
                    (0, 2, Color::Black, PieceKind::Pawn),
                    (1, 2, Color::Black, PieceKind::Pawn),
                    (2, 2, Color::Black, PieceKind::Pawn),
                ]);
            }
            if fm_features && minase_trainer::data::is_validation(11, game) {
                pieces.push((2, 5, Color::Black, PieceKind::SilverGeneral));
            }
            let p = position(
                &pieces,
                side,
                (fm_features && game % 2 == 0).then_some((5, 5)),
            );
            samples.push(
                Sample::from_position(
                    &p,
                    if side == Color::Black { 300 } else { -300 },
                    if game % 3 == 0 {
                        Outcome::Draw
                    } else {
                        Outcome::Win
                    },
                    game,
                    ply,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .unwrap(),
            );
        }
    }
    mnsd::independent(header, &samples)
}
pub fn logs(directory: &Path) -> Vec<Value> {
    fs::read_to_string(directory.join("training.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub fn document(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
pub fn compare_checkpoints(left: &Path, right: &Path) {
    let normalize = |path| {
        let mut arrays = npz::decode(&fs::read(path).unwrap()).unwrap();
        let meta = arrays.get_mut("meta").unwrap();
        let mut value: Value = serde_json::from_slice(&meta.data).unwrap();
        value["elapsed_seconds"] = serde_json::json!(0.0);
        meta.data = serde_json::to_vec(&value).unwrap();
        meta.shape = vec![meta.data.len()];
        arrays
    };
    assert_eq!(normalize(left), normalize(right));
}
pub fn assert_invalid_argument(mut error: &(dyn std::error::Error + 'static), flag: &str) {
    use minase_trainer::error::{Argument, ContractError};
    loop {
        if let Some(contract) = error.downcast_ref::<ContractError>() {
            let expected = match flag {
                "--epochs" => Argument::Epochs,
                "--patience" => Argument::Patience,
                "--batch" => Argument::Batch,
                "--threads" => Argument::Threads,
                "--lambda-res" => Argument::LambdaRes,
                "--removal-penalty" => Argument::RemovalPenalty,
                "--validation-sample" => Argument::ValidationSample,
                _ => panic!("unexpected argument error for {flag}: {contract}"),
            };
            assert_eq!(*contract, ContractError::InvalidArgument(expected));
            return;
        }
        if let Some(nn) = error.downcast_ref::<minase_trainer::nn::Error>() {
            assert!(
                [
                    "--lr",
                    "--weight-decay",
                    "--pst-lr",
                    "--fm-lr",
                    "--fm-weight-decay"
                ]
                .contains(&flag)
            );
            assert!(matches!(nn, minase_trainer::nn::Error::Invalid(_)));
            return;
        }
        error = error.source().expect("expected a typed validation error");
    }
}

// ZIP32中央ディレクトリを順に読む。名前をペイロードから検索しない。
pub fn npz_names(bytes: &[u8]) -> Vec<String> {
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    let count = u16::from_le_bytes(bytes[end + 10..end + 12].try_into().unwrap());
    let mut offset = u32::from_le_bytes(bytes[end + 16..end + 20].try_into().unwrap()) as usize;
    let mut names = Vec::new();
    for _ in 0..count {
        assert_eq!(&bytes[offset..offset + 4], b"PK\x01\x02");
        let length = |at| {
            usize::from(u16::from_le_bytes(
                bytes[offset + at..offset + at + 2].try_into().unwrap(),
            ))
        };
        let name = length(28);
        let extra = length(30);
        let comment = length(32);
        names.push(String::from_utf8(bytes[offset + 46..offset + 46 + name].to_vec()).unwrap());
        offset += 46 + name + extra + comment;
    }
    assert_eq!(offset, end);
    names
}

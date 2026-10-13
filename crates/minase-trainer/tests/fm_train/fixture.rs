use super::*;
use minase::training::records::Header;
use minase_trainer::pst::v0_piece_values;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) struct Fixture {
    pub(super) root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
impl Fixture {
    pub(super) fn new(large_evaluation: bool, with_fm: bool) -> (Self, Args) {
        let root = std::env::temp_dir().join(format!(
            "fm-training-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root };
        let mut args = Arguments::try_parse_from(argv()).unwrap().args;
        args.data = vec![fixture.root.join("records.bin")];
        args.init = fixture.root.join("init.bin");
        args.output_dir = fixture.root.join("continuous");
        args.batch = 16;
        args.validation_sample = 7;
        args.threads = Some(1);

        let weights = vec![
            if large_evaluation {
                [i16::MAX; 2]
            } else {
                [0; 2]
            };
            FEATURE_COUNT
        ];
        let mut initial = Pst::from_weights(
            &weights,
            v0_piece_values().map(|value| value * 2),
            317.25,
            "L0,P0,R1,E0",
        )
        .unwrap()
        .encode();
        if with_fm {
            initial = Pst::from_weights_with_fm(
                &weights,
                v0_piece_values().map(|value| value * 2),
                317.25,
                "L0,P0,R1,E0",
                minase::eval::pst::FmWeights {
                    embeddings: &vec![[771; FM_RANK]; FEATURE_COUNT],
                    signs: &[1; FM_RANK],
                    exponent: 7,
                },
            )
            .unwrap()
            .encode();
        }
        fs::write(&args.init, initial).unwrap();
        let header =
            Header::new("L0,P0,R1,E0".into(), "1".repeat(40), [2; 32], 200, 11, 120).unwrap();
        let mut bytes = header.encode().to_vec();
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
                if minase_trainer::data::is_validation(11, game) {
                    pieces.push((2, 5, Color::Black, PieceKind::SilverGeneral));
                }
                let p = position(&pieces, side, (game % 2 == 0).then_some((5, 5)));
                bytes.extend(
                    Record::from_position(
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
                    .encode(),
                );
            }
        }
        fs::write(&args.data[0], &bytes).unwrap();
        (fixture, args)
    }
}
pub(super) fn logs(directory: &Path) -> Vec<Value> {
    fs::read_to_string(directory.join("training.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
pub(super) fn document(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
pub(super) fn compare_checkpoints(left: &Path, right: &Path) {
    let mut left = Checkpoint::read(left).unwrap();
    let mut right = Checkpoint::read(right).unwrap();
    left.progress.elapsed_seconds = 0.0;
    right.progress.elapsed_seconds = 0.0;
    assert_eq!(left.encode().unwrap(), right.encode().unwrap());
}

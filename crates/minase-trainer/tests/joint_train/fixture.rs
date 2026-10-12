use super::*;
use minase::training::{
    provenance::{Provenance, ResultOrigin, SearchCondition, StartOrigin, Teacher},
    records::Header,
};
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
    pub(super) fn new(large_evaluation: bool) -> (Self, Args) {
        let root = std::env::temp_dir().join(format!(
            "joint-training-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root };
        let mut args = Arguments::try_parse_from(argv()).unwrap().args;
        args.data = vec![fixture.root.join("records.bin")];
        args.base = fixture.root.join("base.bin");
        args.pst_init = fixture.root.join("pst.npz");
        args.fm_init = fixture.root.join("fm.npz");
        args.fm_reference = fixture.root.join("reference.bin");
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
        let endpoints = weights
            .iter()
            .map(|r| r.map(|v| f32::from(v) / 8.0))
            .collect::<Vec<_>>();
        fs::write(
            &args.pst_init,
            minase_trainer::pst::encode_pst_npz(&endpoints).unwrap(),
        )
        .unwrap();
        let v = fm::Model::initialize(vec![true; FEATURE_COUNT], 317.25, 71)
            .unwrap()
            .v()
            .iter()
            .map(|r| r.map(|v| v * 0.1))
            .collect::<Vec<_>>();
        let a = std::array::from_fn(|f| (f as f32 - 15.0) * 0.02);
        let model = fm::Model::from_weights(&v, a, vec![true; FEATURE_COUNT], 317.25).unwrap();
        fs::write(&args.fm_init, fm_npz(&model)).unwrap();
        let quantized = fm::quantize(
            &v.iter().map(|r| r.map(f64::from)).collect::<Vec<_>>(),
            &a.map(f64::from),
            317.25,
        )
        .unwrap();
        let reference = Pst::from_weights_with_fm(
            &weights,
            v0_piece_values().map(|v| v * 2),
            317.25,
            "L0,P0,R1,E0",
            quantized.as_weights(),
        )
        .unwrap();
        fs::write(&args.fm_reference, reference.encode()).unwrap();
        let mut scaled = quantized;
        scaled.exponent += 1;
        let base = Pst::from_weights_with_fm(
            &weights,
            v0_piece_values().map(|v| v * 2),
            317.25,
            "L0,P0,R1,E0",
            scaled.as_weights(),
        )
        .unwrap();
        fs::write(&args.base, base.encode()).unwrap();
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
                    )
                    .encode(),
                );
            }
        }
        fs::write(&args.data[0], &bytes).unwrap();
        Provenance {
            format: "minase-provenance".into(),
            version: 1,
            mnsd_sha256: format!("{:x}", Sha256::digest(&bytes)),
            teacher: Teacher {
                generation_commit: "1".repeat(40),
                network_checksum: "02".repeat(32),
                nodes: 200,
                rule_set: "L0,P0,R1,E0".into(),
                search_condition: SearchCondition::InGame,
            },
            result_origin: ResultOrigin::Selfplay,
            start_origin: StartOrigin::Random,
            lambda: 0.75,
            games: None,
        }
        .write(fs::File::create(fixture.root.join("records.bin.provenance.json")).unwrap())
        .unwrap();
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

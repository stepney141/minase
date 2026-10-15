use super::*;
pub(super) use crate::common::mnsd;
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
        fs::write(
            &args.data[0],
            common::training::records(large_evaluation, true),
        )
        .unwrap();
        (fixture, args)
    }
}

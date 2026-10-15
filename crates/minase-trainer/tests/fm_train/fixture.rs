use super::*;
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
        fs::write(
            &args.data[0],
            common::training::records(large_evaluation, true),
        )
        .unwrap();
        (fixture, args)
    }
}

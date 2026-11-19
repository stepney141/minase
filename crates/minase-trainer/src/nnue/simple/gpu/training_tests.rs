//! CUDA training-loop contracts, exercised at width 16 to bound host memory.
use crate::nnue::simple::{Checkpoint, Parameter, trainer};
use crate::nnue::train::Args;
use clap::Parser;
use minase::training::records::{
    Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, TeacherKind, Value, Writer,
};
use minase_core::Color;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
#[derive(Parser)]
struct Parsed {
    #[command(flatten)]
    args: Args,
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "simple-gpu-loop-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let header = Header::new(
            "JCSA".into(),
            "1".repeat(40),
            [2; 32],
            TeacherKind::AlphaBeta,
            200,
            11,
        )
        .unwrap();
        let path = root.join("data.mnsd");
        let mut writer = Writer::new(std::fs::File::create(&path).unwrap(), header).unwrap();
        for game in 0..240 {
            let record = GameRecord::new(
                game,
                Origin::RandomSelfplay,
                Outcome::Game(minase_core::GameResult::Win {
                    winner: Color::Black,
                    reason: minase_core::WinReason::Resignation,
                }),
                0,
                minase_core::Position::initial(),
                vec![
                    Entry::new(
                        None,
                        Value::AlphaBeta(120),
                        Exclusion::None,
                        Policy::None,
                        None,
                    )
                    .unwrap(),
                ],
            )
            .unwrap();
            writer.write_game(&record).unwrap();
        }
        writer.finish().unwrap();
        Self(root)
    }
    fn args(&self, output: &str, superbatches: u64) -> Args {
        let mut args = Parsed::parse_from([
            "train",
            "--data",
            "unused",
            "--output-dir",
            "unused",
            "--rules",
            "JCSA",
            "--teacher-cp-per-logit",
            "1072.65",
            "--lambda",
            "0.75",
            "--superbatches",
            "2",
            "--batches-per-superbatch",
            "2",
            "--validation-superbatches",
            "1",
            "--lr-schedule",
            "cosine:3:0.027",
            "--device",
            "cuda",
            "--batch",
            "17",
            "--threads",
            "1",
            "--validation-sample",
            "3",
        ])
        .args;
        args.data = vec![self.0.join("data.mnsd")];
        args.output_dir = self.0.join(output);
        args.superbatches = superbatches.try_into().unwrap();
        args
    }
    fn checkpoint(&self, output: &str) -> Checkpoint {
        Checkpoint::read(self.0.join(output).join("latest.npz")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn bits(a: &Checkpoint, b: &Checkpoint) {
    assert_eq!(a.optimizer.step, b.optimizer.step);
    for (a, b) in [
        (&a.parameters, &b.parameters),
        (&a.optimizer.m, &b.optimizer.m),
        (&a.optimizer.v, &b.optimizer.v),
    ] {
        for p in Parameter::ALL {
            assert_eq!(a[p].len(), b[p].len());
            assert!(
                a[p].iter()
                    .zip(&b[p])
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{p:?}"
            );
        }
    }
}
#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_resume_inside_epoch_is_bit_identical() {
    let f = Fixture::new();
    trainer::run_width(f.args("full", 3), 16).unwrap();
    let mut args = f.args("resume", 1);
    trainer::run_width(args.clone(), 16).unwrap();
    let before = f.checkpoint("resume");
    assert_eq!(before.progress.completed_steps, 2);
    // 240 single-position games supply far more than 34 training positions.
    assert!(before.progress.completed_steps * (args.batch as u64) < 200);
    drop(before);
    args.superbatches = 3.try_into().unwrap();
    args.resume = Some(args.output_dir.clone());
    args.threads = Some(2);
    args.time_limit_hours = 23.0;
    trainer::run_width(args, 16).unwrap();
    bits(&f.checkpoint("full"), &f.checkpoint("resume"));
    assert_eq!(
        fs::read(f.0.join("full/nnue.bin")).unwrap(),
        fs::read(f.0.join("resume/nnue.bin")).unwrap()
    );
}
#[test]
#[ignore = "requires a CUDA GPU and NVRTC"]
fn gpu_validation_interval_does_not_change_training() {
    let f = Fixture::new();
    trainer::run_width(f.args("frequent", 3), 16).unwrap();
    let mut args = f.args("sparse", 3);
    args.validation_superbatches = 2.try_into().unwrap();
    trainer::run_width(args, 16).unwrap();
    let rows: Vec<serde_json::Value> = fs::read_to_string(f.0.join("sparse/training.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(rows[1]["validation_loss"].is_null());
    assert!(rows[2]["validation_loss"].is_number());
    assert!(rows[3]["validation_loss"].is_number());
    bits(&f.checkpoint("frequent"), &f.checkpoint("sparse"));
}

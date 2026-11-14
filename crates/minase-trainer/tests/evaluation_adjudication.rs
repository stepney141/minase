//! 指示書の理由13を、実ファイルから分割、較正、学習まで通す。
use clap::Parser;
use minase::training::records::{
    Entry, Exclusion, GameRecord, Header, Origin, Outcome, Policy, TeacherKind, Value, Writer,
};
use minase_core::{Color, Game, Position, Rules};
use minase_trainer::{
    data::{self, CalibrateArgs, Dataset},
    nnue::{handcrafted, train},
};
use std::{
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "minase-adjudication-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write(&self, rules: Rules) -> PathBuf {
        let path = self.0.join(format!("{}.mnsd", rules));
        let header = Header::new(
            rules.to_string(),
            "a".repeat(40),
            [0; 32],
            TeacherKind::AlphaBeta,
            100,
            7,
        )
        .unwrap();
        let mut writer = Writer::new(fs::File::create(&path).unwrap(), header).unwrap();
        let mut chosen = Vec::new();
        for validation in [false, true] {
            for (index, number) in (1..)
                .filter(|&number| data::is_validation(7, number) == validation)
                .take(4)
                .enumerate()
            {
                chosen.push((
                    number,
                    if index == 3 {
                        Color::White
                    } else {
                        Color::Black
                    },
                ));
            }
        }
        chosen.sort_unstable_by_key(|&(number, _)| number);
        for (number, winner) in chosen {
            let mut game = Game::new(rules);
            let mut entries = Vec::new();
            for (score, exclusion) in [
                (300, Exclusion::None),
                (-300, Exclusion::None),
                (20000, Exclusion::Check),
            ] {
                let mv = game.legal_moves()[0];
                entries.push(
                    Entry::new(
                        Some(mv),
                        Value::AlphaBeta(score),
                        exclusion,
                        Policy::PlayedMove,
                        None,
                    )
                    .unwrap(),
                );
                game.play(mv).unwrap();
            }
            entries
                .push(Entry::new(None, Value::None, Exclusion::None, Policy::None, None).unwrap());
            writer
                .write_game(
                    &GameRecord::new(
                        number,
                        Origin::RandomSelfplay,
                        Outcome::EvaluationAdjudication { winner },
                        0,
                        Position::initial(),
                        entries,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        writer.finish().unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn evaluation_adjudication_enters_dataset_calibration_and_validation_with_check_excluded() {
    let fixture = Fixture::new();
    let path = fixture.write(Rules::LISHOGI);
    let dataset = Dataset::open(
        std::slice::from_ref(&path),
        None,
        NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(1.0)
    .unwrap();
    assert_eq!(dataset.record_count(), 16);
    assert_eq!(dataset.training_indices().len(), 8);
    assert_eq!(dataset.validation_indices().len(), 8);
    for indices in [dataset.training_indices(), dataset.validation_indices()] {
        let mut black_wins = 0;
        for &index in indices {
            let sample = dataset.record(index).unwrap();
            assert!(sample.ply() < 2);
            assert_eq!(
                data::is_validation(7, sample.game_number()),
                indices == dataset.validation_indices()
            );
            if sample.ply() == 0 && sample.outcome() == data::Outcome::Win {
                black_wins += 1;
            }
            let next = dataset.record(index ^ 1).unwrap();
            assert_eq!(sample.game_number(), next.game_number());
            assert_ne!(sample.outcome(), next.outcome());
        }
        assert_eq!(black_wins, 3);
    }
    // 同じ絶対値300 cpで勝率3/4なので、尺度は300 / log(3)。
    let estimates = dataset.estimate_teacher_cp_per_logit().unwrap();
    let estimate = estimates[0].as_ref().unwrap();
    assert!((estimate.value - 300.0 / 3.0_f64.ln()).abs() < 0.01);
    assert!(!estimate.at_search_bound);
    let output = fixture.0.join("calibration.json");
    data::calibrate(CalibrateArgs {
        data: vec![path],
        lookahead_gamma: None,
        lookahead_plies: None,
        output: output.clone(),
        threads: Some(1),
    })
    .unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(output).unwrap()).unwrap();
    assert_eq!(saved["classes"][0]["training_positions"], 8);
}

#[derive(Parser)]
struct TrainCommand {
    #[command(flatten)]
    args: train::Args,
}

fn training_args(data: &Path, base: &Path, output: &Path) -> train::Args {
    TrainCommand::parse_from([
        "train",
        "--data",
        data.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--output-dir",
        output.to_str().unwrap(),
        "--device",
        "cpu",
        "--feature-set",
        "P",
        "--width",
        "4",
        "--lambda",
        "1",
        "--lr-schedule",
        "constant:1",
        "--superbatches",
        "1",
        "--batches-per-superbatch",
        "1",
        "--validation-superbatches",
        "1",
        "--batch",
        "8",
        "--validation-sample",
        "8",
        "--threads",
        "1",
    ])
    .args
}

#[test]
fn lishogi_handcrafted_base_trains_adjudicated_games_and_rejects_jcsa_books() {
    let fixture = Fixture::new();
    let base = fixture.0.join("v0.bin");
    handcrafted::run(handcrafted::Args {
        cp_per_logit: 679.03,
        output: base.clone(),
    })
    .unwrap();
    let net = minase::eval::Nnue::decode(&fs::read(&base).unwrap()).unwrap();
    assert_eq!(net.rule_set(), "L1,L2,P0,P3,R1,E1,E3");
    let jcsa = fixture.write(Rules::JCSA);
    let output = fixture.0.join("rejected");
    let error = train::run(training_args(&jcsa, &base, &output))
        .err()
        .unwrap();
    assert!(
        matches!(
            error.downcast_ref::<train::Error>(),
            Some(train::Error::Invalid(
                "MNSD rule set differs from baseline MNUE"
            ))
        ),
        "{error}"
    );
    let lishogi = fixture.write(Rules::LISHOGI);
    let output = fixture.0.join("trained");
    let summary = train::run(training_args(&lishogi, &base, &output)).unwrap();
    assert_eq!(summary.completed_steps, 1);
    let net = minase::eval::Nnue::decode(&fs::read(output.join("nnue.bin")).unwrap()).unwrap();
    assert_eq!(net.rule_set(), "L1,L2,P0,P3,R1,E1,E3");
}

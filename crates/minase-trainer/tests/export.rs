//! NNUEの書き出し契約を、合成MNSDと独立に計算できる線形評価で検証する。

use minase_trainer::nn::BestEpoch;
mod common;

use std::{
    fs,
    ops::Range,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use minase::{
    eval::{
        features::PIECE_STATE_COUNT,
        nnue::{
            self, Network,
            feature_set::{FeatureSet, HalfKpHmB8, P},
            head::DirectScrelu,
        },
    },
    training::records::{Header, Outcome, Record},
};
use minase_core::{Color, PieceKind, Position};
use minase_trainer::{
    data::Dataset,
    nnue::{
        data::InputFeatureSet,
        export::{self, QuantizationReport, write_best},
        model::{Checkpoint, Model, Parameter, Progress, Shape},
        reference::{AdamW, AdamWConfig, v0_piece_values},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(0);
const RULE_SET: &str = "L0,P0,R1,E0";
// hash64(seed=11, game) % 20 == 0となる0..80の対局番号である。
const VALIDATION_GAMES: [u32; 7] = [4, 15, 22, 41, 48, 59, 70];

struct Fixture {
    root: PathBuf,
    dataset: Dataset,
}

impl Fixture {
    fn new(games: Range<u32>, extra_pawn_game: Option<u32>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nnue-export-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("records.bin");
        let records: Vec<_> = games
            .map(|game| {
                let position = if VALIDATION_GAMES.contains(&game) {
                    let mut pieces = vec![
                        (4, 0, Color::Black, PieceKind::King),
                        (7, 11, Color::White, PieceKind::King),
                    ];
                    if extra_pawn_game == Some(game) {
                        pieces.push((2, 5, Color::Black, PieceKind::Pawn));
                    }
                    common::position(&pieces, Color::Black, None)
                } else {
                    Position::initial()
                };
                Record::from_position(
                    &position,
                    120,
                    Outcome::Win,
                    game,
                    0,
                    minase::training::records::Origin::RandomSelfplay,
                )
            })
            .collect();
        let header = Header::new(
            RULE_SET.into(),
            "1".repeat(40),
            [2; 32],
            200,
            11,
            records.len() as u64,
        )
        .unwrap();
        let mut bytes = header.encode().to_vec();
        for record in records {
            bytes.extend(record.encode());
        }
        fs::write(&path, &bytes).unwrap();
        Self {
            // SAFETY: この試験専用ファイルは、Datasetの生存中に変更も切り詰めもしない。
            dataset: unsafe { Dataset::open(&[path], None).unwrap().with_lambda(1.0) }.unwrap(),
            root,
        }
    }

    fn export(&self, checkpoint: &Checkpoint, sample: usize) -> QuantizationReport {
        write_best(
            &self.dataset,
            checkpoint,
            &self.root.join("output"),
            &v0_piece_values(),
            RULE_SET,
            sample,
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn checkpoint(set: InputFeatureSet, k: f32, linear_value: f32) -> Checkpoint {
    let shape = Shape::new(set, 8, k).unwrap();
    let mut model = Model::initialize(
        shape,
        &vec![[0.0; 2]; 13_680],
        common::all_observed(set),
        71,
        0.05,
    )
    .unwrap();
    model.parameter_mut(Parameter::Linear).fill(linear_value);
    model.constrain().unwrap();
    Checkpoint {
        removal_penalty: 1000.0,
        lambda: 1.0,
        model,
        optimizer: AdamW::new(shape, AdamWConfig::default()).unwrap(),
        progress: Progress {
            epoch: 2,
            best: Some(BestEpoch {
                epoch: 2,
                validation_loss: 0.5,
            }),
            elapsed_seconds: 1.5,
            seed: 71,
            config_hash: [3; 32],
            input_hash: [4; 32],
        },
    }
}

fn check_network<F: FeatureSet>(bytes: &[u8], values: &[i32; PIECE_STATE_COUNT]) {
    let network = Network::<F, DirectScrelu>::decode(bytes).unwrap();
    assert_eq!(network.width(), 8);
    assert_eq!(network.k(), 1000.0);
    assert_eq!(network.rule_set(), RULE_SET);
    for (state, &value) in values.iter().enumerate() {
        assert_eq!(network.piece_value_of_state(state), value);
    }
    assert_eq!(nnue::evaluate(&network, &Position::initial()), 0);
}

#[test]
fn both_feature_sets_export_mnue5_and_the_exact_best_checkpoint() {
    let fixture = Fixture::new(0..80, None);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let checkpoint = checkpoint(set, 1000.0, 0.0);
        let report = fixture.export(&checkpoint, 10_000);
        assert_eq!(report.samples, VALIDATION_GAMES.len());
        assert_eq!(report.mean_absolute_error_cp, 0.0);
        assert_eq!(report.max_absolute_error_cp, 0.0);
        assert_eq!(report.limit_cp, 25.0);
        assert!(report.passed);
        let output = fixture.root.join("output");
        assert_eq!(
            Checkpoint::read(output.join("nnue-float.bin")).unwrap(),
            checkpoint
        );
        let bytes = fs::read(output.join("nnue.bin")).unwrap();
        assert_eq!(&bytes[..8], b"MNUE\x05\0\0\0");
        match set {
            InputFeatureSet::P => check_network::<P>(&bytes, &v0_piece_values()),
            InputFeatureSet::HalfKpHmB8 => check_network::<HalfKpHmB8>(&bytes, &v0_piece_values()),
        }
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["samples"], 7);
        assert_eq!(json["seed"], 71);
        assert_eq!(json["split"], "validation");
        assert_eq!(json["passed"], true);
        assert_eq!(json["indices_sha256"].as_str().unwrap().len(), 64);
    }
}

#[test]
fn error_statistics_use_validation_only_and_include_the_largest_error() {
    let fixture = Fixture::new(0..80, Some(15));
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // 各線形値1/16は尺度8の偶数丸めで0になる。王2枚なら1/8 cp、
        // 歩兵が加わる1局面だけ3/16 cp。訓練集合の初期局面は含めない。
        let checkpoint = checkpoint(set, 8.0, 1.0 / 16.0);
        let report = fixture.export(&checkpoint, 10_000);
        assert_eq!(report.samples, 7);
        assert_eq!(
            report.mean_absolute_error_cp,
            (6.0 / 8.0 + 3.0 / 16.0) / 7.0
        );
        assert_eq!(report.max_absolute_error_cp, 3.0 / 16.0);
        assert!(report.passed);
    }
}

#[test]
fn integer_inference_and_the_float_reference_use_centipawns() {
    let fixture = Fixture::new(0..80, None);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // 王2枚の線形評価4 cpと、出力バイアス1×K=8 cpの合計は12 cp。
        // 全項が両形式で厳密に表せるため、整数評価との誤差は0になる。
        let mut checkpoint = checkpoint(set, 8.0, 2.0);
        checkpoint.model.parameter_mut(Parameter::OutputBias)[0] = 1.0;
        let report = fixture.export(&checkpoint, 7);
        assert_eq!(report.mean_absolute_error_cp, 0.0);
        assert_eq!(report.max_absolute_error_cp, 0.0);
        assert!(report.passed);
    }
}

#[test]
fn quantization_error_keeps_the_double_precision_float_evaluation() {
    let fixture = Fixture::new(0..80, None);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut checkpoint = checkpoint(set, 1000.0, 0.0);
        checkpoint.model.parameter_mut(Parameter::OutputBias)[0] = 0.04;
        // 保存値0.04_f32は0.04より小さい。K換算をf32で行うと40へ丸められるが、
        // 倍精度では整数推論の40 cpとの小さな差を検査結果に残す。
        let expected = (f64::from(0.04_f32) * 1000.0 - 40.0).abs();
        assert!(expected > 0.0);
        let report = fixture.export(&checkpoint, 7);
        assert_eq!(report.mean_absolute_error_cp, expected);
        assert_eq!(report.max_absolute_error_cp, expected);
    }
}

#[test]
fn a_fixed_seed_repeats_the_validation_sample_and_caps_its_size() {
    let fixture = Fixture::new(0..80, None);
    let mut checkpoint = checkpoint(InputFeatureSet::P, 1000.0, 0.0);
    let first = fixture.export(&checkpoint, 3);
    assert_eq!(first.samples, 3);
    assert_eq!(first, fixture.export(&checkpoint, 3));
    checkpoint.progress.seed += 1;
    let other = fixture.export(&checkpoint, 3);
    assert_ne!(first.indices_sha256, other.indices_sha256);
    assert_eq!(fixture.export(&checkpoint, 10_000).samples, 7);
}

#[test]
fn the_gate_accepts_k_over_40_exactly_and_reports_a_larger_mean() {
    let fixture = Fixture::new(0..80, None);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // 王2枚×1/16 cpの誤差は1/8 cp。K=5ならちょうどK/40になる。
        let boundary = fixture.export(&checkpoint(set, 5.0, 1.0 / 16.0), 7);
        assert_eq!(boundary.mean_absolute_error_cp, boundary.limit_cp);
        assert!(boundary.passed);
        let failed_checkpoint = checkpoint(set, 4.0, 1.0 / 16.0);
        let failed = fixture.export(&failed_checkpoint, 7);
        assert_eq!(failed.mean_absolute_error_cp, 1.0 / 8.0);
        assert_eq!(failed.limit_cp, 1.0 / 10.0);
        assert!(!failed.passed);
        assert_eq!(
            Checkpoint::read(fixture.root.join("output/nnue-float.bin")).unwrap(),
            failed_checkpoint
        );
        assert!(fixture.root.join("output/nnue.bin").is_file());
    }
}

#[test]
fn export_rejects_empty_samples_and_a_checkpoint_other_than_the_best_epoch() {
    let fixture = Fixture::new(0..80, None);
    let mut checkpoint = checkpoint(InputFeatureSet::P, 1000.0, 0.0);
    let output = fixture.root.join("output");
    let write = |checkpoint: &Checkpoint, sample| {
        write_best(
            &fixture.dataset,
            checkpoint,
            &output,
            &v0_piece_values(),
            RULE_SET,
            sample,
        )
    };
    assert!(matches!(
        write(&checkpoint, 0),
        Err(export::Error::Invalid(_))
    ));
    checkpoint.progress.epoch = 3;
    assert!(matches!(
        write(&checkpoint, 1),
        Err(export::Error::Invalid(_))
    ));
    checkpoint.progress.epoch = 0;
    checkpoint.progress.best = None;
    assert!(matches!(
        write(&checkpoint, 1),
        Err(export::Error::Invalid(_))
    ));
    assert!(!output.exists());
    let empty = Fixture::new(1..2, None);
    assert!(empty.dataset.validation_indices().is_empty());
    let checkpoint = self::checkpoint(InputFeatureSet::P, 1000.0, 0.0);
    assert!(matches!(
        write_best(
            &empty.dataset,
            &checkpoint,
            &empty.root.join("output"),
            &v0_piece_values(),
            RULE_SET,
            1,
        ),
        Err(export::Error::Invalid(_))
    ));
}

#[test]
fn invalid_weights_fail_explicitly_before_exporting() {
    let fixture = Fixture::new(0..80, None);
    let mut checkpoint = checkpoint(InputFeatureSet::P, 1000.0, 0.0);
    checkpoint.model.parameter_mut(Parameter::OutputBias)[0] = f32::NAN;
    let output = fixture.root.join("output");
    assert!(matches!(
        write_best(
            &fixture.dataset,
            &checkpoint,
            &output,
            &v0_piece_values(),
            RULE_SET,
            7,
        ),
        Err(export::Error::Model(_))
    ));
    assert!(!output.exists());
}

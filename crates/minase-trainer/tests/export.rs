//! NNUEの書き出し契約を、合成MNSDと独立に計算できる線形評価で検証する。

use common::mnsd;
use minase_trainer::data::{Outcome, Sample};
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
    training::records::Header,
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
                        (3, 5, Color::White, PieceKind::Pawn),
                    ];
                    if extra_pawn_game == Some(game) {
                        pieces.push((2, 5, Color::Black, PieceKind::Pawn));
                    }
                    common::position(&pieces, Color::Black, None)
                } else {
                    Position::initial()
                };
                Sample::from_position(
                    &position,
                    120,
                    Outcome::Win,
                    game,
                    0,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .unwrap()
            })
            .collect();
        let header = Header::new(
            RULE_SET.into(),
            "1".repeat(40),
            [2; 32],
            minase::training::records::TeacherKind::AlphaBeta,
            200,
            11,
        )
        .unwrap();
        let mut samples = Vec::new();
        for record in records {
            samples.push(record.clone());
        }
        fs::write(&path, mnsd::independent(header, &samples)).unwrap();
        Self {
            dataset: Dataset::open(&[path], None, std::num::NonZeroUsize::new(1).unwrap())
                .unwrap()
                .with_lambda(1.0)
                .unwrap(),
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
        .0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn checkpoint(set: InputFeatureSet, cp_per_logit: f32, linear_value: f32) -> Checkpoint {
    let shape = Shape::new(
        set,
        8,
        cp_per_logit,
        minase_trainer::nnue::model::HeadKind::DirectScrelu,
    )
    .unwrap();
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
            config: minase_trainer::nn::checkpoint::Configuration {
                settings: serde_json::json!({}),
                inputs: serde_json::json!({}),
                preprocessing: serde_json::json!({}),
            },
        },
    }
}

fn check_network<F: FeatureSet>(bytes: &[u8], values: &[i32; PIECE_STATE_COUNT]) {
    let network = Network::<F, DirectScrelu>::decode(bytes).unwrap();
    assert_eq!(network.width(), 8);
    assert_eq!(network.cp_per_logit(), 1000.0);
    assert_eq!(network.rule_set(), RULE_SET);
    for (state, &value) in values.iter().enumerate() {
        assert_eq!(network.piece_value_of_state(state), value);
    }
    assert_eq!(nnue::evaluate(&network, &Position::initial()), 0);
}

#[test]
fn both_feature_sets_export_mnue5_without_rewriting_the_best_checkpoint() {
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
        assert!(!output.join("best.npz").exists());
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
        use sha2::{Digest, Sha256};
        // export::write_bestの契約に従い、seed=71、epoch=0の
        // Fisher–Yatesを独立な整数演算で求めた検証集合の選択順。
        let indices: Vec<u8> = [41_u64, 59, 22, 4, 70, 15, 48]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect();
        assert_eq!(
            json["indices_sha256"],
            format!("{:x}", Sha256::digest(&indices))
        );
    }
}

#[test]
fn error_statistics_use_validation_only_and_include_the_largest_error() {
    let fixture = Fixture::new(0..80, Some(15));
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // 各線形値1/16は尺度8の偶数丸めで0になる。3枚なら3/16 cp、
        // 歩兵が加わる1局面だけ1/4 cp。訓練集合の初期局面は含めない。
        let checkpoint = checkpoint(set, 8.0, 1.0 / 16.0);
        let report = fixture.export(&checkpoint, 10_000);
        assert_eq!(report.samples, 7);
        assert_eq!(
            report.mean_absolute_error_cp,
            (6.0 * 3.0 / 16.0 + 1.0 / 4.0) / 7.0
        );
        assert_eq!(report.max_absolute_error_cp, 1.0 / 4.0);
        assert!(report.passed);
    }
}

#[test]
fn integer_inference_and_the_float_reference_use_centipawns() {
    let fixture = Fixture::new(0..80, None);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // 3枚の線形評価6 cpと、出力バイアス1×cp_per_logit=8 cpの合計は14 cp。
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
        // 保存値0.04_f32は0.04より小さい。cp_per_logit換算をf32で行うと40へ丸められるが、
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
fn the_gate_accepts_cp_per_logit_over_40_exactly_and_reports_a_larger_mean() {
    let fixture = Fixture::new(0..80, None);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // 3枚×1/16 cpの誤差は3/16 cp。cp_per_logit=7.5ならちょうどcp_per_logit / 40になる。
        let boundary = fixture.export(&checkpoint(set, 7.5, 1.0 / 16.0), 7);
        assert_eq!(boundary.mean_absolute_error_cp, boundary.limit_cp);
        assert!(boundary.passed);
        let failed_checkpoint = checkpoint(set, 4.0, 1.0 / 16.0);
        let failed = fixture.export(&failed_checkpoint, 7);
        assert_eq!(failed.mean_absolute_error_cp, 3.0 / 16.0);
        assert_eq!(failed.limit_cp, 1.0 / 10.0);
        assert!(!failed.passed);
        assert!(!fixture.root.join("output/best.npz").exists());
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

fn sign_fixture(positions: &[Position]) -> Fixture {
    let mut fixture = Fixture::new(0..80, None);
    let path = fixture.root.join("sign.mnsd");
    let samples: Vec<_> = positions
        .iter()
        .enumerate()
        .map(|(ply, position)| {
            Sample::from_position(
                position,
                0,
                Outcome::Draw,
                4,
                ply as u16,
                minase::training::records::Origin::RandomSelfplay,
            )
            .unwrap()
        })
        .collect();
    let header = Header::new(
        RULE_SET.into(),
        "1".repeat(40),
        [2; 32],
        minase::training::records::TeacherKind::AlphaBeta,
        200,
        11,
    )
    .unwrap();
    fs::write(&path, mnsd::independent(header, &samples)).unwrap();
    fixture.dataset = Dataset::open(&[path], None, std::num::NonZeroUsize::new(1).unwrap())
        .unwrap()
        .with_lambda(1.0)
        .unwrap();
    fixture
}

#[test]
fn removal_sign_uses_integer_deltas_recorded_perspective_and_shared_sampling() {
    use minase::eval::features::feature_index;
    use minase_core::{PieceCode, PositionBuilder, Square};
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let mut positions = Vec::new();
        for side in [Color::Black, Color::White] {
            let mut builder = PositionBuilder::new(side);
            for (index, color, kind, promoted) in [
                (0, Color::Black, PieceKind::King, false),
                (143, Color::White, PieceKind::King, false),
                (1, Color::Black, PieceKind::CrownPrince, true),
                (50, Color::Black, PieceKind::Pawn, false),
                (60, Color::White, PieceKind::Pawn, false),
                (70, side.opposite(), PieceKind::Lion, true),
            ] {
                let piece = if promoted {
                    PieceCode::new_promoted(color, kind)
                } else {
                    PieceCode::new(color, kind)
                }
                .unwrap();
                builder
                    .put(Square::from_dense(index).unwrap(), piece)
                    .unwrap();
            }
            let mut position = builder.finish().unwrap();
            position.set_lion_capture(Square::from_dense(70)).unwrap();
            positions.push(position);
        }
        let fixture = sign_fixture(&positions);
        let mut checkpoint = checkpoint(set, 1000.0, 0.0);
        // 自駒の歩を-10 cp、相手駒の歩を+4 cpにする。
        // 太子の負値は除外、先獅子の特徴の値は駒を除いても残る。
        for perspective in [Color::Black, Color::White] {
            for (index, value) in [(1, -30.0), (50, -10.0), (60, 4.0)] {
                let square = Square::from_dense(index).unwrap();
                let piece = positions[0].piece_at(square).unwrap();
                let feature = feature_index(perspective, piece, square);
                let row = feature / 144 * 72
                    + feature % 144 / 12 * 6
                    + (feature % 12).min(11 - feature % 12);
                let value = if perspective == Color::Black {
                    value
                } else {
                    -value
                };
                checkpoint.model.parameter_mut(Parameter::Linear)[row * 2..row * 2 + 2].fill(value);
            }
        }
        checkpoint.model.parameter_mut(Parameter::Linear)[6768 * 2..6840 * 2].fill(50.0);
        let (quantization, report) = write_best(
            &fixture.dataset,
            &checkpoint,
            &fixture.root.join("sign-output"),
            &v0_piece_values(),
            RULE_SET,
            2,
        )
        .unwrap();
        assert_eq!(report.positions, quantization.samples);
        let json = serde_json::to_value(&report).unwrap();
        // 2局面×3枚。歩2枚が反転し、値0の獅子は反転しない。
        assert_eq!(
            json,
            serde_json::json!({
                "positions": 2, "removals": 6,
                "reversals": { "count": 4, "total": 6, "fraction": 2.0 / 3.0,
                    "mean_absolute_cp": 7.0, "max_absolute_cp": 10.0 }
            })
        );
        assert!(quantization.passed);
        // 標本を1局面に縮めると、両検査とも同じ1局面だけを扱う。
        let (quantization, report) = write_best(
            &fixture.dataset,
            &checkpoint,
            &fixture.root.join("sign-output"),
            &v0_piece_values(),
            RULE_SET,
            1,
        )
        .unwrap();
        assert_eq!(report.positions, quantization.samples);
        assert_eq!(report.removals, 3);
    }
}

#[test]
fn removal_sign_rejects_royals_only_and_zero_reference_denominators() {
    let royals = common::position(
        &[
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
        ],
        Color::Black,
        None,
    );
    // v0では42枚の飛車が評価上限を超え、1枚除いても整数参照差分は0になる。
    let pieces: Vec<_> = (0..42)
        .map(|i| (i % 12, i / 12, Color::Black, PieceKind::Rook))
        .collect();
    let saturated = common::position(&pieces, Color::Black, None);
    for position in [royals, saturated] {
        let fixture = sign_fixture(&[position]);
        let checkpoint = checkpoint(InputFeatureSet::P, 1000.0, 0.0);
        assert!(matches!(
            write_best(
                &fixture.dataset,
                &checkpoint,
                &fixture.root.join("output"),
                &v0_piece_values(),
                RULE_SET,
                10
            ),
            Err(export::Error::Invalid(_))
        ));
    }
}

#[test]
fn removal_sign_counts_quantized_zero_as_no_reversal_and_excludes_zero_references() {
    let pawn = common::position(&[(1, 2, Color::Black, PieceKind::Pawn)], Color::Black, None);
    let pieces: Vec<_> = (0..42)
        .map(|i| (i % 12, i / 12, Color::Black, PieceKind::Rook))
        .collect();
    let saturated = common::position(&pieces, Color::Black, None);
    let royals = common::position(
        &[
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
        ],
        Color::Black,
        None,
    );
    let fixture = sign_fixture(&[pawn, saturated, royals]);
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        // -1/16 cpは尺度8の偶数丸めで0。実数では反転するが整数差分は0。
        let checkpoint = checkpoint(set, 1000.0, -1.0 / 16.0);
        let (_, report) = write_best(
            &fixture.dataset,
            &checkpoint,
            &fixture.root.join("output"),
            &v0_piece_values(),
            RULE_SET,
            3,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::json!({
                "positions": 3, "removals": 1,
                "reversals": { "count": 0, "total": 1, "fraction": 0.0,
                    "mean_absolute_cp": null, "max_absolute_cp": null }
            })
        );
    }
}

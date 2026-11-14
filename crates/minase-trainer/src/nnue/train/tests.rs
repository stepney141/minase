use crate::nn::checkpoint::MismatchField;
#[path = "tests_export.rs"]
mod export_tests;
#[path = "../../../tests/common/mnsd.rs"]
mod mnsd;
#[path = "tests_subsample.rs"]
mod subsample_tests;
use super::*;
use crate::data::{Outcome, Sample};

use crate::nn::sha256;
use crate::{
    data::Dataset,
    nnue::{
        model::{Checkpoint, Model},
        reference::AdamW,
    },
};
use minase::training::records::Header;
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
use std::{
    cell::Cell,
    collections::HashSet,
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, UNIX_EPOCH},
};

fn nz(value: u64) -> NonZeroU64 {
    NonZeroU64::new(value).unwrap()
}

fn args() -> Args {
    Args {
        buckets: 1,
        bucket_positions: None,
        scratch_dir: None,
        teacher_cp_per_logit: None,
        device: crate::nnue::options::DeviceKind::Cuda,
        training: crate::nnue::options::TrainingOptions::new(1.0),
        lr_schedule: "cosine:3:0.1".parse().unwrap(),
        feature_set: crate::nnue::data::InputFeatureSet::P,
        head: crate::nnue::model::HeadKind::PairwiseL16L32,
        width: 8,
        data: vec!["records.bin".into()],
        train_subsample: None,
        base: "base.bin".into(),
        output_dir: "output".into(),
        lookahead_gamma: None,
        lookahead_plies: None,
        batch: 33,
        superbatches: nz(3),
        batches_per_superbatch: nz(5),
        validation_superbatches: nz(1),
        seed: 1,
        time_limit_hours: 24.0,
        validation_sample: 24,
        threads: Some(2),
        resume: None,
    }
}

fn validation_statistics() -> Validation {
    Validation {
        loss_sum: 0.25,
        correct: 0,
        decisive: 0,
        nonlinear_sum: 0.0,
        nonlinear_square_sum: 0.0,
        lower: 8,
        upper: 0,
        positions: 1,
        activations: 16,
        hidden1_lower: 0,
        hidden1_upper: 0,
        hidden1_activations: 15,
        hidden2_lower: 0,
        hidden2_upper: 0,
        hidden2_activations: 32,
        pairwise_positive: 0,
        pairwise_products: 8,
    }
}

#[test]
fn validation_deadline_includes_the_final_batch_and_stops_further_evaluation() {
    // 各バッチの前後で期限を検査し、期限未満で全件を終えた場合だけ集計を返す。
    for (case, initial_time, completion_times, expected_evaluations, completed) in [
        ("final batch before deadline", 0, [3, 9], 2, true),
        ("final batch at deadline", 0, [3, 10], 2, false),
        ("final batch after deadline", 0, [3, 11], 2, false),
        ("first batch at deadline", 0, [10, 11], 1, false),
        ("deadline before first batch", 10, [10, 11], 0, false),
    ] {
        let now = Cell::new(initial_time);
        let evaluated = Cell::new(0);
        let result = validate_batches(
            completion_times,
            |completed_at| {
                evaluated.set(evaluated.get() + 1);
                now.set(completed_at);
                Ok(validation_statistics())
            },
            || now.get() >= 10,
        )
        .unwrap_or_else(|error| panic!("{case}: {error}"));
        assert_eq!(evaluated.get(), expected_evaluations, "{case}");
        if completed {
            let result = result.unwrap_or_else(|| panic!("{case}: missing validation"));
            assert_eq!(result.positions, 2, "{case}");
            assert_eq!(result.loss(), 0.25, "{case}");
        } else {
            assert!(result.is_none(), "{case}: unexpected validation");
        }
    }
}

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nnue-training-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn training_args(&self) -> Args {
        let mut args = args();
        args.data = vec![self.0.join("records.bin")];
        args.base = self.0.join("base.bin");
        args.output_dir = self.0.join("continuous");
        args.superbatches = nz(3);
        args.lookahead_gamma = Some(0.9);
        args.lookahead_plies = Some(40);
        fs::write(
            &args.base,
            include_bytes!("../../../../minase/nets/nnue-epoch0.bin"),
        )
        .unwrap();
        // seed=11では0..40のうち4、15、22が検証対局である。
        // 40対局×8局面のうち訓練296局面を33件ずつに分け、末尾を32件にする。
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
        for game in 0..40_u32 {
            for ply in 0..8_u16 {
                let side = if ply % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                };
                let mut builder = PositionBuilder::new(side);
                for (file, rank, color, kind) in [
                    (4, 0, Color::Black, PieceKind::King),
                    (7, 11, Color::White, PieceKind::King),
                    ((game % 12) as u8, 3, Color::Black, PieceKind::Pawn),
                    (((game + 6) % 12) as u8, 8, Color::White, PieceKind::Pawn),
                    (((game + 3) % 12) as u8, 6, Color::Black, PieceKind::Rook),
                ] {
                    builder
                        .put(
                            Square::new(file, rank).unwrap(),
                            PieceCode::new(color, kind).unwrap(),
                        )
                        .unwrap();
                }
                let score = if side == Color::Black { 300 } else { -300 };
                let outcome = match (game % 3, side) {
                    (0, _) => Outcome::Draw,
                    (1, Color::Black) | (2, Color::White) => Outcome::Win,
                    _ => Outcome::Loss,
                };
                samples.push(
                    Sample::from_position(
                        &builder.finish().unwrap(),
                        score,
                        outcome,
                        game,
                        ply,
                        minase::training::records::Origin::RandomSelfplay,
                    )
                    .unwrap(),
                );
            }
        }
        fs::write(&args.data[0], mnsd::independent(header, &samples)).unwrap();
        args
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check_training_initialization(feature_set: InputFeatureSet) {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.feature_set = feature_set;
    args.width = 32;
    let session = Session::new(args.clone()).unwrap();
    let model = &session.checkpoint.model;
    assert_eq!(session.checkpoint.optimizer.step(), 0);
    let (weights, mask) = match feature_set {
        InputFeatureSet::P => (
            &model.parameters()[Parameter::FirstLayer],
            model.masks().first_layer(),
        ),
        InputFeatureSet::HalfKpHmB8 => (
            &model.parameters()[Parameter::Virtual],
            model.masks().virtual_rows(),
        ),
    };
    let mut seen = HashSet::new();
    let mut samples = Vec::new();
    for (row, (values, &observed)) in weights.chunks_exact(args.width).zip(mask).enumerate() {
        if !observed {
            continue;
        }
        // HalfKPの鏡映対は同じ乱数を回転して共有するので、片側だけを標本にする。
        let canonical = match feature_set {
            InputFeatureSet::P => row,
            InputFeatureSet::HalfKpHmB8 => row.min(minase::eval::features::mirror_feature(row)),
        };
        if seen.insert(canonical) {
            samples.extend(values.iter().map(|&value| f64::from(value)));
        }
    }
    assert!(samples.len() > 1);
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let sample_std = (samples
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (n - 1.0))
        .sqrt();
    // 仕様のD = 13,680から期待値を求め、実装の初期化関数には依存しない。
    let expected_std = (1.0_f64 / (3.0 * 13_680.0)).sqrt();
    // 正規標本の標本標準偏差の標準誤差は近似的にσ/sqrt(2(n−1))。6倍を許容する。
    let tolerance = 6.0 * expected_std / (2.0 * (n - 1.0)).sqrt();
    assert!(
        (sample_std - expected_std).abs() <= tolerance,
        "{feature_set:?}: n={n}, sample_std={sample_std}, expected={expected_std}, tolerance={tolerance}"
    );
    let config: Value =
        serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap()).unwrap();
    let recorded_std = config["settings"]["initial_standard_deviation"]
        .as_f64()
        .unwrap();
    assert!(
        (recorded_std - expected_std).abs() <= expected_std * f64::from(f32::EPSILON),
        "{feature_set:?}: recorded_std={recorded_std}, expected={expected_std}"
    );
    eprintln!(
        "{feature_set:?}: n={n}, sample_std={sample_std}, tolerance={tolerance}, recorded_std={recorded_std}"
    );
}

#[test]
fn cpu_initialization_p_uses_base_feature_variance() {
    check_training_initialization(InputFeatureSet::P);
}

#[test]
fn cpu_initialization_halfkp_uses_base_feature_variance() {
    check_training_initialization(InputFeatureSet::HalfKpHmB8);
}

#[test]
fn cpu_training_inherits_mnue_metadata_and_initializes_only_from_search_values() {
    use minase::eval::{
        Nnue,
        features::{PIECE_STATE_COUNT, piece_state},
        nnue::{
            EncodeInput, encode,
            feature_set::P,
            head::{Head, PairwiseL16L32},
        },
    };

    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.superbatches = nz(1);
    args.batches_per_superbatch = nz(1);
    args.batch = 296;
    // 入力ネットの線形重みと探索用駒価値を別々に与え、重みの継続学習との混同を検出する。
    let mut values: [i32; PIECE_STATE_COUNT] = std::array::from_fn(|state| 40 + state as i32 * 3);
    let royal = values.iter().copied().max().unwrap()
        + values[piece_state(PieceCode::new(Color::Black, PieceKind::Pawn).unwrap())];
    values[PieceKind::King.index()] = royal;
    values[PieceKind::CrownPrince.index()] = royal;
    let scale = 317.25;
    let rules = minase_core::Rules::JCSA.to_string();
    let head = PairwiseL16L32::decode(&vec![0; PairwiseL16L32::byte_len(4)], 4, true).unwrap();
    fs::write(
        &args.base,
        encode::<P, PairwiseL16L32>(&EncodeInput {
            width: 4,
            cp_per_logit: scale,
            rule_set: &rules,
            piece_values: &values,
            linear: &vec![[777, -333]; FEATURE_COUNT],
            weights: &vec![0; FEATURE_COUNT * 4],
            biases: &[0; 4],
            head: &head,
        })
        .unwrap(),
    )
    .unwrap();
    let base = base::read(&args.base).unwrap();
    let mut initial_args = args.clone();
    initial_args.output_dir = fixture.0.join("initialization");
    let session = Session::new(initial_args).unwrap();
    assert_eq!(session.checkpoint.optimizer.step(), 0);
    assert_eq!(
        session.checkpoint.model.expanded_linear(),
        base.initial_linear()
    );
    let initial = session.checkpoint.model.initial_linear().to_vec();
    drop(session);

    let summary = run(args.clone()).unwrap();
    assert_eq!(summary.completed_superbatches, 1);
    assert_eq!(summary.completed_steps, 1);
    let net = Nnue::decode(&fs::read(args.output_dir.join("nnue.bin")).unwrap()).unwrap();
    assert_eq!(net.cp_per_logit(), scale);
    assert_eq!(net.rule_set(), rules);
    for (state, value) in values.into_iter().enumerate() {
        assert_eq!(net.piece_value_of_state(state), value);
    }
    let saved = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
    assert_eq!(saved.model.initial_linear(), initial);
}

#[test]
fn mnpt_base_is_rejected_before_reading_training_data_or_creating_output() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    fs::write(
        &args.base,
        include_bytes!("../../../../minase/nets/pst.bin"),
    )
    .unwrap();
    fs::remove_file(&args.data[0]).unwrap();
    let error = run(args.clone()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<minase::eval::nnue::Error>(),
        Some(minase::eval::nnue::Error::InvalidMagic { actual }) if actual == b"MNPT"
    ));
    assert!(!args.output_dir.exists());
}

fn logs(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn compare_checkpoint(left: &Path, right: &Path) {
    let mut left = Checkpoint::read(left).unwrap();
    let mut right = Checkpoint::read(right).unwrap();
    // 実時間だけを除き、重み、AdamWの全状態、進捗、マスク、固定値をビット比較する。
    left.progress.elapsed_seconds = 0.0;
    right.progress.elapsed_seconds = 0.0;
    assert!(
        left.encode().unwrap() == right.encode().unwrap(),
        "checkpoint differs after resume"
    );
}

fn parse_training_args(extra: &[&str]) -> std::result::Result<Args, clap::Error> {
    use clap::Parser;
    #[derive(Parser)]
    struct Command {
        #[command(flatten)]
        args: Args,
    }
    Command::try_parse_from(
        [
            "train",
            "--feature-set",
            "P",
            "--width",
            "8",
            "--superbatches",
            "3",
            "--validation-superbatches",
            "1",
            "--lr-schedule",
            "cosine:3:0.1",
            "--lambda",
            "1",
            "--data",
            "missing.bin",
            "--base",
            "missing.mnpt",
            "--output-dir",
            "missing-output",
        ]
        .into_iter()
        .chain(extra.iter().copied()),
    )
    .map(|command| command.args)
}

/// B=1のメモリ供給元を取り出す。
fn memory_dataset(store: &Store) -> &Dataset {
    match store {
        Store::Memory(dataset) => dataset,
        Store::Spill(_) => panic!("memory required"),
    }
}

#[test]
fn resume_requires_an_existing_array_without_changing_the_configuration() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    let path = fixture.0.join("config.json");
    for mut config in [json!({}), json!({"resumes": null}), json!({"resumes": {}})] {
        let original = config.clone();
        record::write_json(&path, &config).unwrap();
        let bytes = fs::read(&path).unwrap();
        let error = record::append_resume(&path, &mut config, &args, 1, 1, UNIX_EPOCH).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<Error>(),
                Some(Error::Invalid("configuration resumes must be an array"))
            ),
            "{error}"
        );
        assert_eq!(config, original);
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn cpu_resume_rejects_changed_checkpoint_meta_fields() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    let session = Session::new(args.clone()).unwrap();
    drop(session);
    args.resume = Some(args.output_dir.clone());
    let path = args.output_dir.join("latest.npz");
    let original = fs::read(&path).unwrap();
    let arrays = crate::nn::npz::decode(&original).unwrap();
    for (field, expected) in [
        ("settings", MismatchField::Settings),
        ("inputs", MismatchField::Inputs),
        ("preprocessing", MismatchField::Preprocessing),
    ] {
        let mut changed = arrays.clone();
        let meta = changed.get_mut("meta").unwrap();
        let mut value: Value = serde_json::from_slice(&meta.data).unwrap();
        value["config"][field] = serde_json::json!({"changed": true});
        meta.data = serde_json::to_vec(&value).unwrap();
        meta.shape = vec![meta.data.len()];
        let bytes = crate::nn::npz::encode(
            &changed
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        fs::write(&path, &bytes).unwrap();
        let error = match Session::new(args.clone()) {
            Ok(_) => panic!("accepted changed {field}"),
            Err(error) => error,
        };
        assert!(
            matches!(error.downcast_ref::<crate::nnue::model::Error>(), Some(crate::nnue::model::Error::CheckpointMismatch(field)) if *field == expected),
            "{field}: {error}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn bucket_arguments_validate_before_reading_inputs() {
    // 仕様1: Bの範囲、容量と作業ディレクトリの組合せを学習前に検査する。
    let fixture = Fixture::new();
    let scratch = fixture.0.to_str().unwrap();
    for extra in [
        vec!["--buckets", "0"],
        vec!["--buckets", "4097"],
        vec!["--bucket-positions", "1"],
        vec!["--scratch-dir", scratch],
        vec!["--buckets", "2"],
        vec!["--buckets", "2", "--bucket-positions", "10"],
        vec!["--buckets", "2", "--scratch-dir", scratch],
        vec![
            "--buckets",
            "2",
            "--bucket-positions",
            "0",
            "--scratch-dir",
            scratch,
        ],
        vec![
            "--buckets",
            "2",
            "--bucket-positions",
            "10",
            "--scratch-dir",
            "/missing-phase2-directory",
        ],
    ] {
        assert!(
            parse_training_args(&extra).unwrap().validate().is_err(),
            "{extra:?}"
        );
    }
    for buckets in ["2", "3", "4096"] {
        parse_training_args(&[
            "--buckets",
            buckets,
            "--bucket-positions",
            "10",
            "--scratch-dir",
            scratch,
        ])
        .unwrap()
        .validate()
        .unwrap();
    }
}

#[test]
fn spill_calibration_files_obey_the_same_contract_as_memory() {
    // 仕様2.3と3.1: 保存済み較正ファイルの照合も、B=1と同じ値とエラーになる。
    let fixture = Fixture::new();
    let args = fixture.training_args();
    let path = fixture.0.join("calibration.json");
    crate::data::calibrate(crate::data::CalibrateArgs {
        data: args.data.clone(),
        lookahead_gamma: args.lookahead_gamma,
        lookahead_plies: args.lookahead_plies,
        output: path.clone(),
        threads: Some(1),
    })
    .unwrap();
    let memory = Dataset::open(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap()
    .with_lambda(0.75)
    .unwrap();
    let spill = crate::data::SpillStore::create(
        &args.data,
        args.lookahead().unwrap(),
        std::num::NonZeroUsize::new(1).unwrap(),
        &fixture.0,
        3,
        200,
    )
    .unwrap()
    .with_lambda(0.75)
    .unwrap();
    let source = crate::data::TeacherCpPerLogitSource::File(path.clone());
    let expected = source.resolve(&memory, 1000.0).unwrap();
    let actual = spill_calibration::resolve(&spill, &source, 1000.0).unwrap();
    assert_eq!(actual.teacher_cp_per_logit, expected.teacher_cp_per_logit);
    assert_eq!(
        actual.teacher_score_multiplier,
        expected.teacher_score_multiplier
    );
    assert_eq!(actual.at_search_bound, expected.at_search_bound);
    assert_eq!(actual.source, expected.source);
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for invalid in 0..6 {
        let mut document = original.clone();
        match invalid {
            0 => document["classes"][0]["teacher_cp_per_logit"] = json!(0.0),
            1 => {
                let duplicate = document["classes"][0].clone();
                document["classes"].as_array_mut().unwrap().push(duplicate);
            }
            2 => document["classes"] = json!([]),
            3 => document["classes"][0]["search_budget"] = json!(u64::MAX),
            4 => document["format"] = json!("invalid"),
            5 => {
                document.as_object_mut().unwrap().remove("conditions");
            }
            _ => unreachable!(),
        }
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let error_text = |result: crate::data::Result<crate::data::TeacherCalibration>| match result
        {
            Err(error) => error.to_string(),
            Ok(_) => panic!("invalid calibration accepted"),
        };
        assert_eq!(
            error_text(source.resolve(&memory, 1000.0)),
            error_text(spill_calibration::resolve(&spill, &source, 1000.0))
        );
    }
}

fn cpu_args(fixture: &Fixture, buckets: u32) -> Args {
    let mut args = fixture.training_args();
    args.device = crate::nnue::options::DeviceKind::Cpu;
    args.width = 4;
    args.buckets = buckets;
    if buckets > 1 {
        args.bucket_positions = Some(400);
        args.scratch_dir = Some(fixture.0.clone());
    }
    args
}

/// 指示書の周の連結を直接たどり、各バッチをCPU参照計算で更新する。
fn expected_updates(args: &Args) -> (Model, AdamW, Vec<f64>) {
    let mut initial = args.clone();
    initial.output_dir = args.output_dir.with_extension("oracle");
    let session = Session::new(initial).unwrap();
    let mut model = session.checkpoint.model.clone();
    let mut optimizer = session.checkpoint.optimizer.clone();
    let total = args.planned_steps().unwrap();
    let mut losses = Vec::new();
    let mut sum = 0.0;
    let mut positions = 0;
    let mut update = |batch: BatchSlot| {
        let lambdas = match session.dataset.as_ref() {
            Store::Memory(d) => d.teacher_lambdas(),
            Store::Spill(d) => d.teacher_lambdas(),
        };
        let targets = build_targets(&batch, &session.teacher_cp_per_logit, lambdas).unwrap();
        let gradients =
            crate::nnue::reference::loss_and_gradients(&model, &batch, &targets).unwrap();
        sum += gradients.losses.loss * batch.len() as f64;
        positions += batch.len();
        // この検査ではconstant:1を指定し、並べ替えと短いバッチの規則だけを独立に検査する。
        optimizer.update(&mut model, &gradients.parameters).unwrap();
        if optimizer
            .step()
            .is_multiple_of(args.batches_per_superbatch.get())
        {
            losses.push(sum / positions as f64);
            sum = 0.0;
            positions = 0;
        }
        optimizer.step() == total
    };
    'epochs: for epoch in 1.. {
        match session.dataset.as_ref() {
            Store::Memory(dataset) => {
                let order = epoch_permutation(session.selection.indices(dataset), args.seed, epoch);
                for indices in order.chunks(args.batch) {
                    let batch = build_batches(dataset, args.feature_set, &[indices])
                        .unwrap()
                        .remove(0);
                    if update(batch) {
                        break 'epochs;
                    }
                }
            }
            Store::Spill(store) => {
                let mut reader = store
                    .epoch(args.seed, epoch, Instant::now() + Duration::from_secs(3600))
                    .unwrap()
                    .unwrap();
                while let Some(records) = reader.next_chunk(args.batch).unwrap() {
                    if update(BatchSlot::from_spill_records(args.feature_set, &records).unwrap()) {
                        break 'epochs;
                    }
                }
            }
        }
    }
    (model, optimizer, losses)
}

#[test]
fn cpu_updates_follow_epoch_batches_at_short_aligned_and_multiple_epoch_boundaries() {
    // M=296、m=33からU=9。B=5は周の途中、9は周境界、20は複数周を含む。
    // bucketは各約99件なので33件のバッチがbucketの境界をまたぐ。
    for buckets in [1, 3] {
        for batches in [5, 9, 20] {
            let fixture = Fixture::new();
            let mut args = cpu_args(&fixture, buckets);
            args.lr_schedule = "constant:1".parse().unwrap();
            args.batches_per_superbatch = nz(batches);
            args.superbatches = nz(2);
            let (model, optimizer, losses) = expected_updates(&args);
            let summary = run(args.clone()).unwrap();
            assert_eq!(summary.completed_steps, 2 * batches);
            assert_eq!(summary.epochs, (2 * batches) as f64 / 9.0);
            let checkpoint = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
            assert_eq!(checkpoint.model, model);
            assert_eq!(checkpoint.optimizer, optimizer);
            for (row, loss) in logs(&args.output_dir.join("training.jsonl"))[1..]
                .iter()
                .zip(losses)
            {
                assert!((row["train_loss"].as_f64().unwrap() - loss).abs() < 1e-12);
            }
            let mut resumed = args.clone();
            resumed.output_dir = fixture.0.join("resumed");
            resumed.superbatches = nz(1);
            run(resumed.clone()).unwrap();
            resumed.resume = Some(resumed.output_dir.clone());
            resumed.superbatches = nz(2);
            run(resumed.clone()).unwrap();
            compare_checkpoint(
                &args.output_dir.join("latest.npz"),
                &resumed.output_dir.join("latest.npz"),
            );
            assert_eq!(
                fs::read(args.output_dir.join("nnue.bin")).unwrap(),
                fs::read(resumed.output_dir.join("nnue.bin")).unwrap()
            );
        }
    }
}

fn mid_epoch_resume(device: crate::nnue::options::DeviceKind) {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        for buckets in [1, 3] {
            let fixture = Fixture::new();
            let mut args = cpu_args(&fixture, buckets);
            args.device = device;
            args.feature_set = set;
            // 1周9更新、superbatchは5更新なので、再開位置は第1周の165局面後である。
            args.lr_schedule = "cosine:4:0.1".parse().unwrap();
            run(args.clone()).unwrap();
            let mut resumed = args.clone();
            resumed.output_dir = fixture.0.join("resumed");
            resumed.superbatches = nz(1);
            run(resumed.clone()).unwrap();
            resumed.resume = Some(resumed.output_dir.clone());
            resumed.superbatches = args.superbatches;
            resumed.validation_superbatches = nz(2);
            resumed.threads = Some(1);
            if buckets > 1 {
                let scratch = fixture.0.join("scratch");
                fs::create_dir(&scratch).unwrap();
                resumed.scratch_dir = Some(scratch.clone());
                resumed.bucket_positions = Some(500);
            }
            run(resumed.clone()).unwrap();
            compare_checkpoint(
                &args.output_dir.join("latest.npz"),
                &resumed.output_dir.join("latest.npz"),
            );
            assert_eq!(
                fs::read(args.output_dir.join("nnue.bin")).unwrap(),
                fs::read(resumed.output_dir.join("nnue.bin")).unwrap()
            );
            let config: Value =
                serde_json::from_slice(&fs::read(resumed.output_dir.join("config.json")).unwrap())
                    .unwrap();
            for key in [
                "superbatches",
                "validation_superbatches",
                "time_limit_hours",
                "threads",
                "bucket_positions",
                "scratch_dir",
            ] {
                assert!(config["settings"].get(key).is_none(), "{key}");
            }
            assert_eq!(config["resumes"][0]["completed_superbatches"], 1);
            assert_eq!(config["resumes"][0]["superbatches"], 3);
        }
    }
}

#[test]
fn cpu_mid_epoch_resume_matches_weights_optimizer_and_network_for_both_feature_sets() {
    mid_epoch_resume(crate::nnue::options::DeviceKind::Cpu);
}

#[test]
#[ignore = "CUDAが必要。学習環境で明示的に実行する"]
fn gpu_mid_epoch_resume_matches_weights_optimizer_and_network_for_both_feature_sets() {
    mid_epoch_resume(crate::nnue::options::DeviceKind::Cuda);
}

fn validation_frequency(device: crate::nnue::options::DeviceKind) {
    let fixture = Fixture::new();
    let mut args = cpu_args(&fixture, 1);
    args.device = device;
    args.superbatches = nz(4);
    run(args.clone()).unwrap();
    for frequency in [2, 3, 5] {
        let mut sparse = args.clone();
        sparse.validation_superbatches = nz(frequency);
        sparse.output_dir = fixture.0.join(format!("frequency-{frequency}"));
        let summary = run(sparse.clone()).unwrap();
        compare_checkpoint(
            &args.output_dir.join("latest.npz"),
            &sparse.output_dir.join("latest.npz"),
        );
        let rows = logs(&sparse.output_dir.join("training.jsonl"));
        assert_eq!(rows.len(), 5);
        for (k, row) in rows.iter().enumerate() {
            assert_eq!(row["superbatch"], k);
            let validated = k == 0 || k == 4 || (k as u64).is_multiple_of(frequency);
            for field in [
                "validation_loss",
                "samples",
                "decisive_samples",
                "nonlinear_std_cp",
                "saturation",
                "hidden1_saturation",
                "hidden2_saturation",
                "pairwise_both_positive",
                "validation_seconds",
            ] {
                assert_eq!(row[field].is_null(), !validated, "k={k}: {field}");
            }
            if !validated {
                assert!(row["sign_accuracy"].is_null());
            }
            assert!(row["first_layer_at_limit"].is_number());
            assert!(row["linear_movement"].is_object());
        }
        assert_eq!(
            summary.final_validation_loss,
            rows[4]["validation_loss"].as_f64()
        );
    }
}

#[test]
fn cpu_validation_superbatches_only_change_validation_records() {
    validation_frequency(crate::nnue::options::DeviceKind::Cpu);
}

#[test]
#[ignore = "CUDAが必要。学習環境で明示的に実行する"]
fn gpu_validation_superbatches_do_not_change_the_trajectory() {
    validation_frequency(crate::nnue::options::DeviceKind::Cuda);
}

#[test]
fn data_size_does_not_change_update_budget_or_learning_rates() {
    let fixture = Fixture::new();
    let mut args = cpu_args(&fixture, 1);
    args.lr_schedule = "exponential:1:0.1".parse().unwrap();
    let mut expected_rates = None;
    for copies in [1, 2] {
        let mut data = args.data.clone();
        if copies == 2 {
            let copy = fixture.0.join("copy.mnsd");
            let dataset =
                Dataset::open(&data, None, std::num::NonZeroUsize::new(1).unwrap()).unwrap();
            let samples = (0..dataset.record_count())
                .map(|i| dataset.record(i).unwrap())
                .collect::<Vec<_>>();
            let header = Header::new(
                "L0,P0,R1,E0".into(),
                "1".repeat(40),
                [2; 32],
                minase::training::records::TeacherKind::AlphaBeta,
                200,
                12,
            )
            .unwrap();
            fs::write(&copy, mnsd::independent(header, &samples)).unwrap();
            data.push(copy);
        }
        args.data = data;
        args.output_dir = fixture.0.join(format!("size-{copies}"));
        let summary = run(args.clone()).unwrap();
        assert_eq!(summary.completed_steps, 15);
        let rates: Vec<_> = logs(&args.output_dir.join("training.jsonl"))[1..]
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let first = row["lr_multiplier_first"].as_f64().unwrap();
                let last = row["lr_multiplier_last"].as_f64().unwrap();
                assert_eq!(first, 2.0_f64.powf(-(i as f64)));
                assert_eq!(last, 2.0_f64.powf(-((i * 5 + 4) as f64) / 5.0));
                (first, last)
            })
            .collect();
        if let Some(expected) = &expected_rates {
            assert_eq!(&rates, expected);
        }
        expected_rates = Some(rates);
    }
}

#[test]
fn time_limit_leaves_only_committed_updates_and_resume_matches_uninterrupted_training() {
    for buckets in [1, 3] {
        let fixture = Fixture::new();
        let args = cpu_args(&fixture, buckets);
        run(args.clone()).unwrap();
        let mut resumed = args.clone();
        resumed.output_dir = fixture.0.join("resumed");
        resumed.superbatches = nz(1);
        run(resumed.clone()).unwrap();
        let original = Checkpoint::read(resumed.output_dir.join("latest.npz")).unwrap();
        resumed.resume = Some(resumed.output_dir.clone());
        resumed.superbatches = args.superbatches;
        resumed.time_limit_hours = f64::MIN_POSITIVE;
        let error = run(resumed.clone()).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Error>(),
            Some(Error::Incomplete)
        ));
        assert!(!resumed.output_dir.join("nnue.bin").exists());
        let summary: Value =
            serde_json::from_slice(&fs::read(resumed.output_dir.join("summary.json")).unwrap())
                .unwrap();
        assert_eq!(summary["status"], "incomplete");
        assert_eq!(summary["stopped_by"], "time_limit");
        assert_eq!(summary["completed_steps"], 5);
        assert_eq!(summary["planned_steps"], 15);
        assert_eq!(summary["completed_superbatches"], 1);
        let saved = Checkpoint::read(resumed.output_dir.join("latest.npz")).unwrap();
        assert_eq!(saved.model, original.model);
        assert_eq!(saved.optimizer, original.optimizer);
        resumed.time_limit_hours = 24.0;
        run(resumed.clone()).unwrap();
        compare_checkpoint(
            &args.output_dir.join("latest.npz"),
            &resumed.output_dir.join("latest.npz"),
        );
        assert_eq!(
            fs::read(args.output_dir.join("nnue.bin")).unwrap(),
            fs::read(resumed.output_dir.join("nnue.bin")).unwrap()
        );
    }
}

#[test]
fn resume_rejects_changed_schedule_batch_budget_and_inconsistent_progress() {
    let fixture = Fixture::new();
    let mut args = cpu_args(&fixture, 1);
    args.superbatches = nz(2);
    run(args.clone()).unwrap();
    args.resume = Some(args.output_dir.clone());
    let original = fs::read(args.output_dir.join("nnue.bin")).unwrap();
    for case in 0..4 {
        let mut changed = args.clone();
        match case {
            0 => changed.batches_per_superbatch = nz(6),
            1 => changed.lr_schedule = "constant:1".parse().unwrap(),
            2 => changed.superbatches = nz(1),
            3 => changed.buckets = 2,
            _ => unreachable!(),
        }
        assert!(run(changed).is_err());
        assert_eq!(
            fs::read(args.output_dir.join("nnue.bin")).unwrap(),
            original
        );
    }
    let before = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
    // 保存済みkと同じ予算なら、重みと最適化器を更新しない。
    run(args.clone()).unwrap();
    let after = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
    assert_eq!(before.model, after.model);
    assert_eq!(before.optimizer, after.optimizer);
    let mut invalid = after;
    invalid.progress.completed_superbatches = 1;
    invalid.write(args.output_dir.join("latest.npz")).unwrap();
    assert!(run(args).is_err());
}

#[test]
fn resume_truncates_uncommitted_rows_and_requires_the_checkpoint_row() {
    let fixture = Fixture::new();
    let mut args = cpu_args(&fixture, 1);
    run(args.clone()).unwrap();
    args.resume = Some(args.output_dir.clone());
    let path = args.output_dir.join("training.jsonl");
    let original = fs::read(&path).unwrap();
    let mut tail = original.clone();
    tail.extend_from_slice(b"{\"superbatch\":4}\n{partial");
    fs::write(&path, tail).unwrap();
    run(args.clone()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    let rows = logs(&path);
    let missing = rows[..rows.len() - 1]
        .iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>();
    for tail in ["", "{partial"] {
        fs::write(&path, format!("{missing}{tail}")).unwrap();
        assert!(run(args.clone()).is_err());
    }
}

#[test]
fn old_nnue_progress_is_rejected_with_an_explicit_format_error() {
    let fixture = Fixture::new();
    let mut args = cpu_args(&fixture, 1);
    let session = Session::new(args.clone()).unwrap();
    drop(session);
    let path = args.output_dir.join("latest.npz");
    let mut arrays = crate::nn::npz::decode(&fs::read(&path).unwrap()).unwrap();
    let meta = arrays.get_mut("meta").unwrap();
    let mut value: Value = serde_json::from_slice(&meta.data).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .remove("completed_superbatches");
    value["epoch"] = json!(0);
    value["best"] = Value::Null;
    meta.data = serde_json::to_vec(&value).unwrap();
    meta.shape = vec![meta.data.len()];
    let bytes = crate::nn::npz::encode(
        &arrays
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    fs::write(&path, bytes).unwrap();
    args.resume = Some(args.output_dir.clone());
    assert!(
        run(args)
            .unwrap_err()
            .to_string()
            .contains("old NNUE checkpoint format")
    );
}

#[test]
fn omitted_teacher_scale_uses_base_and_zero_decay_while_explicit_sources_keep_targets() {
    for buckets in [1, 3] {
        for lambda in [0.0, 0.75] {
            let fixture = Fixture::new();
            let mut args = cpu_args(&fixture, buckets);
            args.training.lambda = lambda;
            args.lookahead_gamma = None;
            args.lookahead_plies = None;
            let session = Session::new(args.clone()).unwrap();
            let config: Value =
                serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap())
                    .unwrap();
            let scale = f64::from(session.base.cp_per_logit);
            assert_eq!(
                config["preprocessing"]["teacher_cp_per_logit_source"],
                "base"
            );
            assert_eq!(
                config["preprocessing"]["teacher_cp_per_logit"],
                if lambda == 0.0 {
                    json!([null])
                } else {
                    json!([scale])
                }
            );
            assert_eq!(config["settings"]["optimizer"]["ft"]["weight_decay"], 0.0);
            assert_eq!(config["settings"]["optimizer"]["head"]["weight_decay"], 0.0);
            let dataset = Dataset::open(&args.data, None, std::num::NonZeroUsize::new(1).unwrap())
                .unwrap()
                .with_lambda(lambda)
                .unwrap();
            let batch = build_batches(
                &dataset,
                args.feature_set,
                &[&dataset.training_indices()[..3]],
            )
            .unwrap()
            .remove(0);
            let actual = build_targets(
                &batch,
                &session.teacher_cp_per_logit,
                dataset.teacher_lambdas(),
            )
            .unwrap();
            for (&index, target) in dataset.training_indices()[..3].iter().zip(actual) {
                let sample = dataset.record(index).unwrap();
                let outcome = match sample.outcome() {
                    Outcome::Win => 1.0,
                    Outcome::Draw => 0.5,
                    Outcome::Loss => 0.0,
                };
                let expected = lambda / (1.0 + (-f64::from(sample.score()) / scale).exp())
                    + (1.0 - lambda) * outcome;
                assert!((f64::from(target) - expected).abs() < 1e-7);
            }
            let calibration = fixture.0.join("calibration.json");
            crate::data::calibrate(crate::data::CalibrateArgs {
                data: args.data.clone(),
                lookahead_gamma: None,
                lookahead_plies: None,
                output: calibration.clone(),
                threads: Some(1),
            })
            .unwrap();
            for source in [
                crate::data::TeacherCpPerLogitSource::Calibrate,
                crate::data::TeacherCpPerLogitSource::File(calibration),
            ] {
                let expected = source.resolve(&dataset, session.base.cp_per_logit).unwrap();
                let actual = session
                    .dataset
                    .calibration(Some(&source), session.base.cp_per_logit)
                    .unwrap();
                assert_eq!(actual.teacher_cp_per_logit, expected.teacher_cp_per_logit);
                assert_eq!(actual.source, expected.source);
                assert_eq!(
                    build_targets(&batch, &actual.target_values(), dataset.teacher_lambdas())
                        .unwrap(),
                    build_targets(&batch, &expected.target_values(), dataset.teacher_lambdas())
                        .unwrap()
                );
            }
        }
    }
}

#[test]
fn constant_multiplier_matches_training_with_scaled_base_learning_rates() {
    let fixture = Fixture::new();
    let mut scheduled = fixture.training_args();
    scheduled.device = crate::nnue::options::DeviceKind::Cpu;
    scheduled.lr_schedule = "constant:0.25".parse().unwrap();
    scheduled.training.weight_decay_linear = 0.001;
    scheduled.training.weight_decay_ft = 0.004;
    scheduled.training.weight_decay_head = 0.009;
    run(scheduled.clone()).unwrap();

    let mut scaled = scheduled.clone();
    scaled.output_dir = fixture.0.join("scaled");
    scaled.lr_schedule = "constant:1".parse().unwrap();
    scaled.training.lr_linear *= 0.25;
    scaled.training.lr_ft *= 0.25;
    scaled.training.lr_head *= 0.25;
    run(scaled.clone()).unwrap();

    let left = Checkpoint::read(scheduled.output_dir.join("latest.npz")).unwrap();
    let right = Checkpoint::read(scaled.output_dir.join("latest.npz")).unwrap();
    assert_eq!(left.optimizer.config(), scheduled.training.optimizer());
    assert_eq!(right.optimizer.config(), scaled.training.optimizer());
    assert_eq!(left.optimizer.step(), right.optimizer.step());
    for p in Parameter::ALL {
        for (a, b) in [
            (left.model.parameters(), right.model.parameters()),
            (left.optimizer.m(), right.optimizer.m()),
            (left.optimizer.v(), right.optimizer.v()),
        ] {
            assert!(
                a[p].iter()
                    .zip(&b[p])
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{p:?}"
            );
        }
    }
    assert_eq!(
        fs::read(scheduled.output_dir.join("nnue.bin")).unwrap(),
        fs::read(scaled.output_dir.join("nnue.bin")).unwrap()
    );
}

#[test]
fn resume_rejects_changed_trajectory_settings_and_input_paths() {
    let fixture = Fixture::new();
    let args = cpu_args(&fixture, 1);
    drop(Session::new(args.clone()).unwrap());
    let settings = [
        (
            "seed",
            (|a: &mut Args| a.seed += 1) as fn(&mut Args),
            MismatchField::Seed,
        ),
        (
            "device",
            |a| a.device = crate::nnue::options::DeviceKind::Cuda,
            MismatchField::Device,
        ),
        (
            "lr_linear",
            |a| a.training.lr_linear *= 2.0,
            MismatchField::LrLinear,
        ),
        ("lr_ft", |a| a.training.lr_ft *= 2.0, MismatchField::LrFt),
        (
            "lr_head",
            |a| a.training.lr_head *= 2.0,
            MismatchField::LrHead,
        ),
        (
            "decay_linear",
            |a| a.training.weight_decay_linear = 0.01,
            MismatchField::WeightDecayLinear,
        ),
        (
            "decay_ft",
            |a| a.training.weight_decay_ft = 0.01,
            MismatchField::WeightDecayFt,
        ),
        (
            "decay_head",
            |a| a.training.weight_decay_head = 0.01,
            MismatchField::WeightDecayHead,
        ),
        (
            "lambda",
            |a| a.training.lambda = 0.75,
            MismatchField::Lambda,
        ),
    ];
    for (field, change, expected) in settings {
        let mut changed = args.clone();
        changed.resume = Some(changed.output_dir.clone());
        change(&mut changed);
        let error = run(changed).unwrap_err();
        assert!(
            matches!(error.downcast_ref::<crate::nn::Error>(), Some(crate::nn::Error::CheckpointMismatch(actual)) if *actual == expected),
            "{field}: {error}"
        );
    }
    for input in ["base", "data"] {
        let mut changed = args.clone();
        changed.resume = Some(changed.output_dir.clone());
        let destination = fixture.0.join(format!("copied-{input}"));
        if input == "base" {
            fs::copy(&changed.base, &destination).unwrap();
            changed.base = destination;
        } else {
            fs::copy(&changed.data[0], &destination).unwrap();
            changed.data = vec![destination];
        }
        let error = run(changed).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<crate::nn::Error>(),
                Some(crate::nn::Error::CheckpointMismatch(MismatchField::Inputs))
            ),
            "{input}: {error}"
        );
    }
}

#[test]
fn completed_resume_fills_a_missing_final_validation_without_updating_weights() {
    let fixture = Fixture::new();
    let mut args = cpu_args(&fixture, 1);
    args.superbatches = nz(2);
    args.validation_superbatches = nz(8);
    run(args.clone()).unwrap();
    let original = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
    let path = args.output_dir.join("training.jsonl");
    let mut rows = logs(&path);
    let final_loss = rows[2]["validation_loss"].clone();
    // 予算の大きい実行が検証の番でないkで中断した保存物を表す。
    for key in [
        "validation_loss",
        "sign_accuracy",
        "samples",
        "decisive_samples",
        "nonlinear_std_cp",
        "saturation",
        "hidden1_saturation",
        "hidden2_saturation",
        "pairwise_both_positive",
        "validation_seconds",
    ] {
        rows[2][key] = Value::Null;
    }
    fs::write(
        &path,
        rows.iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>(),
    )
    .unwrap();
    args.resume = Some(args.output_dir.clone());
    args.time_limit_hours = f64::MIN_POSITIVE;
    assert!(run(args.clone()).is_err());
    assert!(!args.output_dir.join("nnue.bin").exists());
    args.time_limit_hours = 24.0;
    let summary = run(args.clone()).unwrap();
    assert_eq!(json!(summary.final_validation_loss), final_loss);
    let after = logs(&path);
    assert_eq!(after.len(), 3);
    assert_eq!(after[2]["validation_loss"], final_loss);
    for key in [
        "train_loss",
        "steps",
        "total_steps",
        "training_seconds",
        "lr_multiplier_first",
        "lr_multiplier_last",
    ] {
        assert_eq!(after[2][key], rows[2][key]);
    }
    let saved = Checkpoint::read(args.output_dir.join("latest.npz")).unwrap();
    assert_eq!(saved.model, original.model);
    assert_eq!(saved.optimizer, original.optimizer);
}

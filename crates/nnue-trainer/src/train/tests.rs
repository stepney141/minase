use super::*;
use crate::{
    data::{Dataset, Observations, build_batches, epoch_permutation},
    model::{BestEpoch, Checkpoint, Masks, Model, Progress, Shape},
    reference::{AdamW, AdamWConfig},
};
use minase::training::{
    provenance::{Provenance, ResultOrigin, SearchCondition, StartOrigin, Teacher},
    records::{HEADER_LEN, Header, Outcome, RECORD_LEN, Record},
};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

fn args() -> Args {
    Args {
        device: crate::options::DeviceKind::Cuda,
        training: crate::options::TrainingOptions::default(),
        feature_set: crate::data::InputFeatureSet::P,
        width: 8,
        data: vec!["records.bin".into()],
        init_pst: "pst-float.npz".into(),
        base: "pst.bin".into(),
        output_dir: "output".into(),
        lookahead_gamma: None,
        lookahead_plies: None,
        batch: 33,
        epochs: 50,
        patience: 5,
        seed: 1,
        time_limit_hours: 24.0,
        validation_sample: 24,
        threads: Some(2),
        resume: None,
    }
}

fn initial_progress() -> Progress {
    Progress {
        epoch: 0,
        best: None,
        elapsed_seconds: 0.0,
        seed: 1,
        config_hash: [1; 32],
        input_hash: [2; 32],
    }
}

#[test]
fn best_epoch_excludes_epoch_zero_and_requires_strict_improvement() {
    let mut progress = initial_progress();
    assert!(!update_best(&mut progress, 0, 0.1).unwrap());
    assert_eq!(progress.best, None);
    assert!(update_best(&mut progress, 1, 0.5).unwrap());
    assert_eq!(
        progress.best,
        Some(BestEpoch {
            epoch: 1,
            validation_loss: 0.5
        })
    );
    assert!(!update_best(&mut progress, 2, 0.5).unwrap());
    assert!(!update_best(&mut progress, 3, 0.6).unwrap());
    assert_eq!(progress.best.unwrap().epoch, 1);
    assert!(update_best(&mut progress, 4, 0.4).unwrap());
    assert_eq!(
        progress.best,
        Some(BestEpoch {
            epoch: 4,
            validation_loss: 0.4
        })
    );
}

#[test]
fn non_finite_validation_cannot_replace_a_completed_best_epoch() {
    for loss in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut progress = initial_progress();
        update_best(&mut progress, 1, 0.5).unwrap();
        let saved = progress.clone();
        assert!(update_best(&mut progress, 2, loss).is_err());
        assert_eq!(progress, saved);
    }
}

#[test]
fn early_stopping_waits_for_five_unimproved_epochs_and_resets_on_improvement() {
    let args = args();
    let mut progress = initial_progress();
    update_best(&mut progress, 1, 0.5).unwrap();
    for epoch in 2..=5 {
        update_best(&mut progress, epoch, 0.5).unwrap();
        assert_eq!(stop_reason(&progress, &args), None, "epoch={epoch}");
    }
    update_best(&mut progress, 6, 0.6).unwrap();
    assert_eq!(
        stop_reason(&progress, &args),
        Some(StopReason::EarlyStopping)
    );

    let mut progress = initial_progress();
    update_best(&mut progress, 1, 0.5).unwrap();
    update_best(&mut progress, 5, 0.4).unwrap();
    for epoch in 6..=9 {
        update_best(&mut progress, epoch, 0.5).unwrap();
        assert_eq!(stop_reason(&progress, &args), None, "epoch={epoch}");
    }
    update_best(&mut progress, 10, 0.4).unwrap();
    assert_eq!(
        stop_reason(&progress, &args),
        Some(StopReason::EarlyStopping)
    );
}

#[test]
fn budgets_stop_at_the_boundary_without_selecting_epoch_zero() {
    let args = args();
    let mut progress = initial_progress();
    progress.elapsed_seconds = 24.0 * 3600.0 - 1.0;
    assert_eq!(stop_reason(&progress, &args), None);
    progress.elapsed_seconds += 1.0;
    assert_eq!(stop_reason(&progress, &args), Some(StopReason::TimeLimit));
    assert_eq!(progress.best, None);

    let mut progress = initial_progress();
    update_best(&mut progress, 49, 0.5).unwrap();
    assert_eq!(stop_reason(&progress, &args), None);
    update_best(&mut progress, 50, 0.6).unwrap();
    assert_eq!(stop_reason(&progress, &args), Some(StopReason::MaxEpochs));
    progress.elapsed_seconds = 24.0 * 3600.0;
    assert_eq!(stop_reason(&progress, &args), Some(StopReason::TimeLimit));
    assert_eq!(progress.best.unwrap().epoch, 49);
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
    }
}

#[test]
fn validation_deadline_includes_the_final_batch() {
    // Pythonのvalidation_lossと同じく、最終バッチの完了時刻も期限未満でなければならない。
    for final_time in [9, 10, 11] {
        let now = Cell::new(0);
        let evaluated = Cell::new(0);
        let result = validate_batches(
            [3, final_time],
            |completed_at| {
                evaluated.set(evaluated.get() + 1);
                now.set(completed_at);
                Ok(validation_statistics())
            },
            || now.get() >= 10,
        )
        .unwrap();
        assert_eq!(evaluated.get(), 2);
        if final_time < 10 {
            let result = result.unwrap();
            assert_eq!(result.positions, 2);
            assert_eq!(result.loss(), 0.25);
        } else {
            assert!(result.is_none(), "final batch completed at {final_time}");
        }
    }
}

#[test]
fn validation_timeout_stops_before_evaluating_more_batches() {
    for (initial_time, expected_evaluations) in [(0, 1), (10, 0)] {
        let now = Cell::new(initial_time);
        let evaluated = Cell::new(0);
        let result = validate_batches(
            [10, 11],
            |completed_at| {
                evaluated.set(evaluated.get() + 1);
                now.set(completed_at);
                Ok(validation_statistics())
            },
            || now.get() >= 10,
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(evaluated.get(), expected_evaluations);
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
        args.base = self.0.join("pst.bin");
        args.init_pst = self.0.join("pst-float.npz");
        args.output_dir = self.0.join("continuous");
        args.epochs = 2;
        args.lookahead_gamma = Some(0.9);
        args.lookahead_plies = Some(40);
        fs::write(&args.base, include_bytes!("../../../minase/nets/pst.bin")).unwrap();
        let initial: Vec<_> = base::read(&args.base)
            .unwrap()
            .linear
            .iter()
            .map(|pair| pair.map(|weight| f32::from(weight) / 8.0))
            .collect();
        fs::write(&args.init_pst, stored_npz(&initial)).unwrap();

        // seed=11では0..40のうち4、15、22が検証対局である。
        // 40対局×8局面のうち訓練296局面を33件ずつに分け、末尾を32件にする。
        let header =
            Header::new("L0,P0,R1,E0".into(), "1".repeat(40), [2; 32], 200, 11, 320).unwrap();
        let mut bytes = header.encode().to_vec();
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
                bytes.extend(
                    Record::from_position(&builder.finish().unwrap(), score, outcome, game, ply)
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
        .write(File::create(self.0.join("records.bin.provenance.json")).unwrap())
        .unwrap();
        args
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// NPY 1.0の2配列を、ZIP仕様の無圧縮メンバーとして格納する。
fn stored_npz(initial: &[[f32; 2]]) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (endpoint, name) in ["middlegame.npy", "endgame.npy"].into_iter().enumerate() {
        let mut header =
            "{'descr': '<f4', 'fortran_order': False, 'shape': (13680,), }".to_string();
        while !(10 + header.len() + 1).is_multiple_of(64) {
            header.push(' ');
        }
        header.push('\n');
        let mut array = b"\x93NUMPY\x01\x00".to_vec();
        array.extend((header.len() as u16).to_le_bytes());
        array.extend(header.as_bytes());
        for pair in initial {
            array.extend(pair[endpoint].to_le_bytes());
        }
        let mut crc = u32::MAX;
        for &byte in &array {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
            }
        }
        let offset = zip.len() as u32;
        let size = array.len() as u32;
        zip.extend(b"PK\x03\x04");
        for value in [20_u16, 0, 0, 0, 0] {
            zip.extend(value.to_le_bytes());
        }
        for value in [!crc, size, size] {
            zip.extend(value.to_le_bytes());
        }
        for value in [name.len() as u16, 0] {
            zip.extend(value.to_le_bytes());
        }
        zip.extend(name.as_bytes());
        zip.extend(array);
        central.extend(b"PK\x01\x02");
        for value in [20_u16, 20, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        for value in [!crc, size, size] {
            central.extend(value.to_le_bytes());
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        central.extend(0_u32.to_le_bytes());
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let offset = zip.len() as u32;
    let size = central.len() as u32;
    zip.extend(central);
    zip.extend(b"PK\x05\x06");
    for value in [0_u16, 0, 2, 2] {
        zip.extend(value.to_le_bytes());
    }
    zip.extend(size.to_le_bytes());
    zip.extend(offset.to_le_bytes());
    zip.extend(0_u16.to_le_bytes());
    zip
}

fn initialized(args: &Args, dataset: &Dataset) -> (Model, Observations) {
    let base = base::read(&args.base).unwrap();
    let observations = Observations::compute(dataset, args.feature_set).unwrap();
    let model = Model::initialize(
        Shape::new(args.feature_set, args.width, base.k).unwrap(),
        &crate::model::load_initial(&args.init_pst).unwrap(),
        Masks::from_observations(args.feature_set, &observations).unwrap(),
        args.seed,
        0.05,
    )
    .unwrap();
    (model, observations)
}

#[test]
fn epoch_batches_preserve_the_permutation_and_include_the_partial_tail() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    assert_eq!(dataset.record_count(), 320);
    assert_eq!(dataset.training_indices().len(), 296);
    assert_eq!(dataset.validation_indices().len(), 24);
    let permutation = epoch_permutation(dataset.training_indices(), args.seed, 1);
    assert_eq!(
        permutation,
        epoch_permutation(dataset.training_indices(), args.seed, 1)
    );
    assert_ne!(
        permutation,
        epoch_permutation(dataset.training_indices(), args.seed, 2)
    );
    let ranges: Vec<_> = permutation.chunks(args.batch).collect();
    let batches = build_batches(&dataset, args.feature_set, &ranges).unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.len()).collect::<Vec<_>>(),
        [33, 33, 33, 33, 33, 33, 33, 33, 32]
    );
    let joined: Vec<_> = batches
        .iter()
        .flat_map(|batch| batch.record_indices().iter().copied())
        .collect();
    assert_eq!(joined, permutation);
    let mut sorted = joined;
    sorted.sort_unstable();
    assert_eq!(sorted, dataset.training_indices());
}

#[test]
fn resume_hashes_bind_training_settings_and_inputs_but_allow_budget_and_worker_changes() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    let (model, observations) = initialized(&args, &dataset);
    let teacher_ks = dataset.estimate_teacher_ks().unwrap();
    let configuration =
        record::configuration(&args, 2, &dataset, &observations, &teacher_ks, &model).unwrap();
    assert_eq!(
        configuration.document["preprocessing"]["training_positions"],
        296
    );
    assert_eq!(
        configuration.document["preprocessing"]["validation_positions"],
        24
    );
    assert_eq!(
        configuration.document["inputs"]["files"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for key in ["base", "init_pst"] {
        assert_eq!(
            configuration.document["inputs"][key]["sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
    }
    let checkpoint = Checkpoint {
        removal_penalty: args.training.removal_penalty,
        lambda_override: args.training.lambda_override,
        optimizer: AdamW::new(model.shape(), AdamWConfig::default()).unwrap(),
        model: model.clone(),
        progress: Progress {
            config_hash: configuration.config_hash,
            input_hash: configuration.input_hash,
            ..initial_progress()
        },
    };
    let restored = Checkpoint::decode(&checkpoint.encode().unwrap()).unwrap();
    restored
        .check_hashes(&configuration.config_hash, &configuration.input_hash)
        .unwrap();
    let mut resume = args.clone();
    resume.output_dir = fixture.0.join("resumed");
    resume.resume = Some(resume.output_dir.clone());
    resume.epochs = 10;
    resume.patience = 2;
    resume.time_limit_hours = 12.0;
    resume.threads = Some(3);
    let same =
        record::configuration(&resume, 3, &dataset, &observations, &teacher_ks, &model).unwrap();
    restored
        .check_hashes(&same.config_hash, &same.input_hash)
        .unwrap();
    resume.seed += 1;
    let changed =
        record::configuration(&resume, 2, &dataset, &observations, &teacher_ks, &model).unwrap();
    assert_ne!(changed.config_hash, configuration.config_hash);
    assert!(
        restored
            .check_hashes(&changed.config_hash, &changed.input_hash)
            .is_err()
    );

    // FMだけを変えた有効なMNPTでも、--baseの検査和は変わる。
    let mut bytes = fs::read(&args.base).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let checksum = Sha256::digest(&bytes[80..]);
    bytes[48..80].copy_from_slice(&checksum);
    fs::write(&args.base, bytes).unwrap();
    base::read(&args.base).unwrap();
    let changed =
        record::configuration(&args, 2, &dataset, &observations, &teacher_ks, &model).unwrap();
    assert_eq!(changed.config_hash, configuration.config_hash);
    assert_ne!(changed.input_hash, configuration.input_hash);
    assert!(
        restored
            .check_hashes(&changed.config_hash, &changed.input_hash)
            .is_err()
    );
}

#[test]
fn each_resume_persists_its_time_budget_workers_and_starting_epoch() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    let (model, observations) = initialized(&args, &dataset);
    let teacher_ks = dataset.estimate_teacher_ks().unwrap();
    let configuration =
        record::configuration(&args, 2, &dataset, &observations, &teacher_ks, &model).unwrap();
    let mut original = configuration.document;
    assert_eq!(original["resumes"], json!([]));
    // 初回の再開で配列を新設し、2回目以降も既存の履歴を保持する。
    original.as_object_mut().unwrap().remove("resumes");
    let path = fixture.0.join("config.json");
    record::write_json(&path, &original).unwrap();
    let earliest = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    for (epoch, epochs, patience, hours, workers) in [(1, 4, 2, 6.0, 3), (3, 8, 4, 12.0, 1)] {
        args.epochs = epochs;
        args.patience = patience;
        args.time_limit_hours = hours;
        args.threads = Some(workers);
        let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record::append_resume(&path, &mut saved, &args, workers, epoch).unwrap();
    }
    let latest = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let resumes = saved["resumes"].as_array().unwrap();
    assert_eq!(resumes.len(), 2);
    for (entry, epoch, epochs, patience, hours, workers) in [
        (&resumes[0], 1, 4, 2, 6.0, 3),
        (&resumes[1], 3, 8, 4, 12.0, 1),
    ] {
        let timestamp = entry["resumed_at_unix_seconds"].as_u64().unwrap();
        assert!((earliest..=latest).contains(&timestamp));
        assert_eq!(entry["epoch"], epoch);
        assert_eq!(entry["epochs"], epochs);
        assert_eq!(entry["patience"], patience);
        assert_eq!(entry["time_limit_hours"], hours);
        assert_eq!(entry["workers"], workers);
    }
    saved.as_object_mut().unwrap().remove("resumes");
    assert_eq!(saved, original);
}

#[test]
fn resume_rejects_each_changed_input_or_setting_before_loading_cuda() {
    for change in [
        "settings",
        "device",
        "lr_linear",
        "weight_decay_linear",
        "lr_ft",
        "weight_decay_ft",
        "lr_head",
        "weight_decay_head",
        "removal_penalty",
        "lambda_override",
        "npz",
        "base",
        "data",
        "provenance",
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.training_args();
        {
            // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
            let dataset = unsafe {
                Dataset::open(
                    &args.data,
                    args.lookahead().unwrap(),
                    Some(args.training.lambda_override),
                )
                .unwrap()
            };
            let (model, observations) = initialized(&args, &dataset);
            let teacher_ks = dataset.estimate_teacher_ks().unwrap();
            let configuration =
                record::configuration(&args, 2, &dataset, &observations, &teacher_ks, &model)
                    .unwrap();
            fs::create_dir(&args.output_dir).unwrap();
            record::write_json(
                &args.output_dir.join("config.json"),
                &configuration.document,
            )
            .unwrap();
            record::append_epoch(
                &args.output_dir.join("training.jsonl"),
                &epoch_record(&model, 0),
            )
            .unwrap();
            Checkpoint {
                removal_penalty: args.training.removal_penalty,
                lambda_override: args.training.lambda_override,
                optimizer: AdamW::new(model.shape(), AdamWConfig::default()).unwrap(),
                model,
                progress: Progress {
                    config_hash: configuration.config_hash,
                    input_hash: configuration.input_hash,
                    ..initial_progress()
                },
            }
            .write(args.output_dir.join("latest.bin"))
            .unwrap();
        }
        args.resume = Some(args.output_dir.clone());
        match change {
            "settings" => args.seed += 1,
            "device" => args.device = crate::options::DeviceKind::Cpu,
            "lr_linear" => args.training.lr_linear *= 2.0,
            "weight_decay_linear" => args.training.weight_decay_linear = 0.001,
            "lr_ft" => args.training.lr_ft *= 2.0,
            "weight_decay_ft" => args.training.weight_decay_ft *= 2.0,
            "lr_head" => args.training.lr_head *= 2.0,
            "weight_decay_head" => args.training.weight_decay_head *= 2.0,
            "removal_penalty" => args.training.removal_penalty *= 2.0,
            "lambda_override" => args.training.lambda_override = 0.75,
            "npz" => {
                // 1/64cpは再量子化を変えず、鏡映する2行を同じだけ動かす。
                let mut initial = crate::model::load_initial(&args.init_pst).unwrap();
                initial[0][0] += 1.0 / 64.0;
                initial[11][0] += 1.0 / 64.0;
                fs::write(&args.init_pst, stored_npz(&initial)).unwrap();
            }
            "base" => {
                let mut bytes = fs::read(&args.base).unwrap();
                let last = bytes.len() - 1;
                bytes[last] ^= 1;
                let checksum = Sha256::digest(&bytes[80..]);
                bytes[48..80].copy_from_slice(&checksum);
                fs::write(&args.base, bytes).unwrap();
            }
            "data" => {
                // ヘッダを保ち、先読みで参照されない対局先頭の探索値だけを変える。
                let mut bytes = fs::read(&args.data[0]).unwrap();
                let first = Record::decode(&bytes[HEADER_LEN..HEADER_LEN + RECORD_LEN]).unwrap();
                let changed = Record::from_position(
                    &first.to_position().unwrap(),
                    first.score() + 1,
                    first.outcome(),
                    first.game_number(),
                    first.ply(),
                );
                bytes[HEADER_LEN..HEADER_LEN + RECORD_LEN].copy_from_slice(&changed.encode());
                fs::write(&args.data[0], &bytes).unwrap();
                let path = fixture.0.join("records.bin.provenance.json");
                let mut provenance = Provenance::read(File::open(&path).unwrap()).unwrap();
                provenance.mnsd_sha256 = format!("{:x}", Sha256::digest(&bytes));
                provenance.write(File::create(path).unwrap()).unwrap();
            }
            "provenance" => {
                let path = fixture.0.join("records.bin.provenance.json");
                let mut provenance = Provenance::read(File::open(&path).unwrap()).unwrap();
                provenance.teacher.search_condition = SearchCondition::Standalone;
                provenance.write(File::create(path).unwrap()).unwrap();
            }
            _ => unreachable!(),
        }
        let error = match Session::new(args) {
            Ok(_) => panic!("resume accepted changed {change}"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.downcast_ref::<Error>(),
                Some(Error::Invalid(
                    "resume configuration or input hash mismatch"
                ))
            ),
            "{change} reached an unexpected error: {error}"
        );
    }
}

fn epoch_record(model: &Model, epoch: u64) -> EpochRecord {
    EpochRecord {
        device: crate::options::DeviceKind::Cuda,
        epoch,
        train_loss: (epoch > 0).then_some(0.5),
        removal_loss: (epoch > 0).then_some(0.1),
        total_loss: (epoch > 0).then_some(0.6),
        validation_loss: 0.4,
        sign_accuracy: Some(0.75),
        samples: 24,
        decisive_samples: 16,
        nonlinear_std_cp: 3.5,
        saturation: Saturation {
            acc_le_zero: 0.25,
            acc_ge_one: 0.125,
        },
        linear_movement: record::linear_movement(model),
        training_seconds: (epoch > 0).then_some(3.0),
        validation_seconds: 0.5,
        training_positions_per_second: (epoch > 0).then_some(296.0 / 3.0),
        update_seconds: (epoch > 0).then_some(0.1),
        gpu_memory_used_bytes: Some(1024 * 1024),
        gpu_memory_total_bytes: Some(8 * 1024 * 1024),
        elapsed_seconds: 3.5 * epoch as f64 + 0.5,
        steps: if epoch == 0 { 0 } else { 9 },
        total_steps: 9 * epoch,
    }
}

#[test]
fn epoch_log_uses_one_json_line_per_epoch_and_preserves_metrics() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    let (model, _) = initialized(&args, &dataset);
    let path = fixture.0.join("training.jsonl");
    for epoch in 0..=1 {
        record::append_epoch(&path, &epoch_record(&model, epoch)).unwrap();
    }
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 2);
    assert_eq!(bytes.last(), Some(&b'\n'));
    let rows = logs(&path);
    assert_eq!(rows[0]["epoch"], 0);
    assert!(rows[0]["train_loss"].is_null());
    assert!(rows[0]["removal_loss"].is_null());
    assert_eq!(rows[1]["epoch"], 1);
    for (key, expected) in [
        ("train_loss", 0.5),
        ("removal_loss", 0.1),
        ("total_loss", 0.6),
        ("validation_loss", 0.4),
        ("sign_accuracy", 0.75),
        ("nonlinear_std_cp", 3.5),
        ("training_seconds", 3.0),
        ("validation_seconds", 0.5),
    ] {
        assert_eq!(rows[1][key].as_f64(), Some(expected), "{key}");
    }
    assert_eq!(rows[1]["saturation"]["acc_le_zero"], 0.25);
    assert_eq!(rows[1]["saturation"]["acc_ge_one"], 0.125);
    assert_eq!(
        rows[1]["linear_movement"]["middlegame"]["mean_absolute_cp"],
        0.0
    );
    assert_eq!(
        rows[1]["linear_movement"]["endgame"]["max_absolute_cp"],
        0.0
    );
    assert_eq!(rows[1]["gpu_memory_used_bytes"], 1024 * 1024);
    assert_eq!(rows[1]["steps"], 9);
    assert_eq!(rows[1]["total_steps"], 9);
}

#[test]
fn resume_keeps_completed_log_epochs_and_discards_an_uncommitted_tail() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    let (model, _) = initialized(&args, &dataset);
    let path = fixture.0.join("training.jsonl");
    for epoch in 0..=2 {
        record::append_epoch(&path, &epoch_record(&model, epoch)).unwrap();
    }
    let mut bytes = fs::read(&path).unwrap();
    bytes.extend(b"{\"epoch\":3");
    fs::write(&path, bytes).unwrap();
    assert_eq!(record::reconcile_log(&path, 1).unwrap(), 0.4);
    assert_eq!(
        logs(&path)
            .iter()
            .map(|row| row["epoch"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(record::reconcile_log(&path, 2).is_err());
}

fn completed_checkpoint(model: &Model, epoch: u64, best: BestEpoch) -> Checkpoint {
    let mut model = model.clone();
    model.parameter_mut(Parameter::OutputBias)[0] = epoch as f32 / 8.0;
    let mut optimizer = AdamW::new(model.shape(), AdamWConfig::default()).unwrap();
    optimizer.step = 9 * epoch;
    optimizer.m[Parameter::OutputBias][0] = epoch as f32 / 32.0;
    optimizer.v[Parameter::OutputBias][0] = epoch as f32 / 64.0;
    Checkpoint {
        removal_penalty: 1000.0,
        lambda_override: 1.0,
        model,
        optimizer,
        progress: Progress {
            epoch,
            best: Some(best),
            elapsed_seconds: 10.0 * epoch as f64,
            ..initial_progress()
        },
    }
}

#[test]
fn committed_latest_recovers_an_interrupted_best_checkpoint_save() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    let (model, _) = initialized(&args, &dataset);
    let old = completed_checkpoint(
        &model,
        1,
        BestEpoch {
            epoch: 1,
            validation_loss: 0.5,
        },
    );
    let latest = completed_checkpoint(
        &model,
        2,
        BestEpoch {
            epoch: 2,
            validation_loss: 0.4,
        },
    );
    let path = fixture.0.join("best.bin");
    old.write(&path).unwrap();
    recover_best_checkpoint(&fixture.0, &latest).unwrap();
    assert!(fs::read(&path).unwrap() == latest.encode().unwrap());

    fs::remove_file(&path).unwrap();
    recover_best_checkpoint(&fixture.0, &latest).unwrap();
    assert!(fs::read(&path).unwrap() == latest.encode().unwrap());
}

#[test]
fn resume_preserves_an_older_best_and_rejects_its_loss_or_corruption() {
    let fixture = Fixture::new();
    let args = fixture.training_args();
    // SAFETY: 専用の一時ファイルはDatasetの破棄まで変更・切り詰めない。
    let dataset = unsafe {
        Dataset::open(
            &args.data,
            args.lookahead().unwrap(),
            Some(args.training.lambda_override),
        )
        .unwrap()
    };
    let (model, _) = initialized(&args, &dataset);
    let best_epoch = BestEpoch {
        epoch: 1,
        validation_loss: 0.5,
    };
    let best = completed_checkpoint(&model, 1, best_epoch);
    let latest = completed_checkpoint(&model, 2, best_epoch);
    let path = fixture.0.join("best.bin");
    best.write(&path).unwrap();
    let original = fs::read(&path).unwrap();
    recover_best_checkpoint(&fixture.0, &latest).unwrap();
    assert!(fs::read(&path).unwrap() == original);

    fs::remove_file(&path).unwrap();
    assert!(recover_best_checkpoint(&fixture.0, &latest).is_err());
    fs::write(&path, b"truncated checkpoint").unwrap();
    assert!(recover_best_checkpoint(&fixture.0, &latest).is_err());
    let wrong = completed_checkpoint(
        &model,
        1,
        BestEpoch {
            epoch: 1,
            validation_loss: 0.6,
        },
    );
    wrong.write(&path).unwrap();
    assert!(recover_best_checkpoint(&fixture.0, &latest).is_err());
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

#[test]
#[ignore = "CUDAが必要。学習環境で明示的に実行する"]
fn gpu_training_two_epochs_match_an_epoch_boundary_resume_for_both_feature_sets() {
    training_two_epochs_match_resume(crate::options::DeviceKind::Cuda);
}

#[test]
fn cpu_training_two_epochs_match_an_epoch_boundary_resume_for_both_feature_sets() {
    training_two_epochs_match_resume(crate::options::DeviceKind::Cpu);
}

fn training_two_epochs_match_resume(device: crate::options::DeviceKind) {
    for feature_set in [
        crate::data::InputFeatureSet::P,
        crate::data::InputFeatureSet::HalfKpHmB8,
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.training_args();
        args.feature_set = feature_set;
        args.device = device;
        args.training.lambda_override = 0.75;
        args.training.removal_penalty = 250.0;
        args.training.lr_linear = 0.012;
        args.training.weight_decay_linear = 1e-5;
        args.training.lr_ft = 0.002;
        args.training.weight_decay_ft = 0.0003;
        args.training.lr_head = 0.0015;
        args.training.weight_decay_head = 0.0002;
        run(args.clone()).unwrap();

        let mut split = args.clone();
        split.output_dir = fixture.0.join("resumed");
        let mut session = Session::new(split.clone()).unwrap();
        assert!(session.advance().unwrap());
        drop(session);
        let first = Checkpoint::read(split.output_dir.join("latest.bin")).unwrap();
        assert_eq!(first.progress.epoch, 1);
        assert_eq!(first.optimizer.step(), 9);
        split.resume = Some(split.output_dir.clone());
        split.threads = Some(5);
        run(split.clone()).unwrap();
        let configuration: Value =
            serde_json::from_slice(&fs::read(split.output_dir.join("config.json")).unwrap())
                .unwrap();
        assert_eq!(configuration["resumes"].as_array().unwrap().len(), 1);
        assert_eq!(configuration["resumes"][0]["epoch"], 1);
        assert_eq!(configuration["resumes"][0]["epochs"], split.epochs);
        assert_eq!(configuration["resumes"][0]["workers"], 5);
        assert_eq!(
            configuration["device"],
            serde_json::to_value(device).unwrap()
        );
        assert_eq!(configuration["settings"]["lambda_override"], 0.75);
        assert_eq!(configuration["settings"]["removal_penalty"], 250.0);

        for file in ["latest.bin", "best.bin", "nnue-float.bin"] {
            compare_checkpoint(&args.output_dir.join(file), &split.output_dir.join(file));
        }
        assert_eq!(
            fs::read(args.output_dir.join("nnue.bin")).unwrap(),
            fs::read(split.output_dir.join("nnue.bin")).unwrap()
        );
        let latest = Checkpoint::read(split.output_dir.join("latest.bin")).unwrap();
        assert_eq!(latest.progress.epoch, 2);
        assert_eq!(latest.optimizer.step(), 18);
        assert!(latest.progress.best.unwrap().epoch >= 1);
        let continuous = logs(&args.output_dir.join("training.jsonl"));
        let resumed = logs(&split.output_dir.join("training.jsonl"));
        assert_eq!(
            continuous
                .iter()
                .map(|row| row["epoch"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(resumed.len(), continuous.len());
        for (left, right) in continuous.iter().zip(&resumed) {
            for row in [left, right] {
                assert_eq!(row["device"], serde_json::to_value(device).unwrap());
                if device == crate::options::DeviceKind::Cpu {
                    assert!(row["gpu_memory_used_bytes"].is_null());
                    assert!(row["gpu_memory_total_bytes"].is_null());
                }
            }
            for key in [
                "epoch",
                "train_loss",
                "removal_loss",
                "total_loss",
                "validation_loss",
                "steps",
                "total_steps",
            ] {
                assert_eq!(left[key], right[key], "{feature_set:?}: {key}");
            }
            assert!(left["validation_loss"].as_f64().unwrap().is_finite());
            if left["epoch"] != 0 {
                for key in ["train_loss", "removal_loss", "total_loss"] {
                    assert!(
                        left[key].as_f64().unwrap().is_finite(),
                        "{feature_set:?}: {key}"
                    );
                }
            }
        }
    }
}

#[test]
fn cpu_result_only_training_records_null_teacher_k_and_resumes() {
    let fixture = Fixture::new();
    let mut args = fixture.training_args();
    args.device = crate::options::DeviceKind::Cpu;
    args.width = 2;
    args.batch = 128;
    args.epochs = 1;
    args.training.lambda_override = 0.0;
    run(args.clone()).unwrap();
    let config: Value =
        serde_json::from_slice(&fs::read(args.output_dir.join("config.json")).unwrap()).unwrap();
    assert_eq!(config["preprocessing"]["teacher_ks"], json!([null]));
    assert_eq!(config["preprocessing"]["teacher_lambdas"], json!([0.0]));
    args.resume = Some(args.output_dir.clone());
    args.epochs = 2;
    run(args.clone()).unwrap();
    let checkpoint = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
    assert_eq!(checkpoint.progress.epoch, 2);
    assert_eq!(checkpoint.lambda_override, 0.0);
}

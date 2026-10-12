//! 指示書の再生形式を手で作り、CLIの拒否条件と更新状態を検査する。
use super::*;
use minase::eval::features::mirror_feature;
use minase_trainer::{
    data::{is_validation, replay::Replay},
    nn::npz::{self, Array},
};
use serde_json::json;
use std::collections::BTreeMap;

fn fixture() -> (Fixture, Args, Vec<(u32, u32)>) {
    let (fixture, mut args) = Fixture::new(false, false);
    // 2番目のMNSDはseedだけ変える。対局の分割も異なるのでファイル番号の誤変換を検出できる。
    let mut bytes = Header::new("L0,P0,R1,E0".into(), "1".repeat(40), [2; 32], 200, 22, 120)
        .unwrap()
        .encode()
        .to_vec();
    bytes.extend(&fs::read(&args.data[0]).unwrap()[136..]);
    let second = fixture.root.join("second.bin");
    fs::write(&second, &bytes).unwrap();
    let mut provenance = document(fixture.root.join("records.bin.provenance.json"));
    provenance["mnsd_sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    fs::write(
        fixture.root.join("second.bin.provenance.json"),
        serde_json::to_vec(&provenance).unwrap(),
    )
    .unwrap();
    args.data.push(second);
    let directory = fixture.root.join("replay");
    fs::create_dir(&directory).unwrap();
    args.replay = Some(directory.clone());
    let inputs: Vec<_> = args
        .data
        .iter()
        .map(|path| {
            json!({
                "path": path.canonicalize().unwrap(),
                "sha256": format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
            })
        })
        .collect();
    fs::write(
        directory.join("replay.json"),
        serde_json::to_vec(&json!({
            "format": "minase-replay-1", "trainer": "pst", "files": inputs,
            "batch": args.batch, "epochs": args.epochs
        }))
        .unwrap(),
    )
    .unwrap();
    let mut positions = Vec::new();
    for (file, seed) in [11, 22].into_iter().enumerate() {
        for game in 0..40 {
            if !is_validation(seed, game) {
                for ply in 0..3 {
                    positions.push((file as u32, game * 3 + ply));
                }
            }
        }
    }
    // ファイル順を逆転し、2エポック目では端数の位置も変える。
    positions.reverse();
    write_order(&directory, 1, &positions);
    let mut second_order = positions.clone();
    second_order.rotate_left(13);
    write_order(&directory, 2, &second_order);
    fs::write(
        directory.join("initial.npz"),
        pst::encode_pst_npz(&vec![[0.0; 2]; FEATURE_COUNT]).unwrap(),
    )
    .unwrap();
    (fixture, args, positions)
}

fn write_order(directory: &Path, epoch: u64, positions: &[(u32, u32)]) {
    let bytes: Vec<_> = positions
        .iter()
        .flat_map(|(file, local)| file.to_le_bytes().into_iter().chain(local.to_le_bytes()))
        .collect();
    fs::write(directory.join(format!("epoch-{epoch:03}.positions")), bytes).unwrap();
    fs::write(
        directory.join(format!("epoch-{epoch:03}.mirror")),
        vec![0; positions.len()],
    )
    .unwrap();
}

fn rejects(args: &Args, reason: &str) {
    let error = pst::run(args.clone()).unwrap_err().to_string();
    assert!(error.contains(reason), "{reason}: {error}");
    assert!(
        !args.output_dir.exists(),
        "invalid replay must fail before creating output"
    );
}

#[test]
fn replay_steps_require_replay_positive_count_and_fresh_run() {
    for extra in [
        vec!["--replay-steps", "1"],
        vec!["--replay", "replay", "--replay-steps", "0"],
        vec![
            "--replay",
            "replay",
            "--replay-steps",
            "1",
            "--resume",
            "output",
        ],
        vec![
            "--replay",
            "replay",
            "--replay-steps",
            "9223372036854775808",
        ],
    ] {
        let mut values = argv();
        values.extend(extra);
        let result = Arguments::try_parse_from(values);
        assert!(result.is_err() || result.unwrap().args.validate().is_err());
    }
    let (_fixture, mut args, positions) = fixture();
    args.replay_steps = Some((positions.len().div_ceil(args.batch) * 2 + 1) as u64);
    rejects(&args, "exceeds available updates");
}

#[test]
fn replay_rejects_manifest_identity_batch_and_epoch_mismatches() {
    let (_fixture, args, _) = fixture();
    let path = args.replay.as_ref().unwrap().join("replay.json");
    let original = document(&path);
    type ManifestChange = fn(&mut Value);
    let cases: Vec<(ManifestChange, &str)> = vec![
        (|v| v["format"] = json!("unknown"), "format or trainer"),
        (|v| v["trainer"] = json!("fm"), "format or trainer"),
        (
            |v| v["files"].as_array_mut().unwrap().swap(0, 1),
            "file order or path",
        ),
        (
            |v| v["files"][0]["sha256"] = json!("00".repeat(32)),
            "SHA-256",
        ),
        (
            |v| v["files"][0]["path"] = json!("records.bin"),
            "file order or path",
        ),
        (
            |v| {
                v["files"].as_array_mut().unwrap().pop();
            },
            "file count",
        ),
        (|v| v["batch"] = json!(17), "batch mismatch"),
        (|v| v["epochs"] = json!(1), "insufficient epochs"),
    ];
    for (change, reason) in cases {
        let mut value = original.clone();
        change(&mut value);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        rejects(&args, reason);
    }
    fs::write(&path, b"{").unwrap();
    rejects(&args, "invalid replay.json");
}

#[test]
fn replay_rejects_missing_duplicate_outside_and_malformed_positions() {
    let (_fixture, args, positions) = fixture();
    let directory = args.replay.as_ref().unwrap();
    let cases: Vec<(Vec<(u32, u32)>, &str)> = vec![
        (positions[..positions.len() - 1].to_vec(), "position count"),
        (
            {
                let mut v = positions.clone();
                v.push(v[0]);
                v
            },
            "position count",
        ),
        (
            {
                let mut v = positions.clone();
                v[1] = v[0];
                v
            },
            "duplicate",
        ),
        (
            {
                let mut v = positions.clone();
                v[0] = (0, 12);
                v
            },
            "outside training set",
        ), // seed11のgame4は検証用。
        (
            {
                let mut v = positions.clone();
                v[0] = (0, 120);
                v
            },
            "out of range",
        ),
        (
            {
                let mut v = positions.clone();
                v[0] = (2, 0);
                v
            },
            "out of range",
        ),
    ];
    for (changed, reason) in cases {
        write_order(directory, 1, &changed);
        rejects(&args, reason);
    }
    write_order(directory, 1, &positions);
    let path = directory.join("epoch-001.positions");
    let mut bytes = fs::read(&path).unwrap();
    bytes.pop();
    fs::write(&path, bytes).unwrap();
    rejects(&args, "record length");
    write_order(directory, 1, &positions);
    for name in ["epoch-002.positions", "epoch-002.mirror"] {
        let path = directory.join(name);
        let bytes = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        // 最初の更新だけを要求しても、--epochs分の列の不足を拒否する。
        let mut steps = args.clone();
        steps.replay_steps = Some(1);
        assert!(pst::run(steps).is_err());
        assert!(!args.output_dir.exists());
        fs::write(&path, bytes).unwrap();
    }
}

#[test]
fn replay_checks_mirror_bits_and_pst_requires_zero() {
    let (_fixture, args, positions) = fixture();
    let directory = args.replay.as_ref().unwrap();
    for (count, bit, reason) in [
        (positions.len() - 1, 0, "mirror length"),
        (positions.len(), 2, "bit value"),
        (positions.len(), 1, "must all be zero"),
    ] {
        let mut bits = vec![0; count];
        bits[0] = bit;
        fs::write(directory.join("epoch-001.mirror"), bits).unwrap();
        rejects(&args, reason);
    }
    // データ層は鏡映1を保持する。PST固有の制約は学習器が課す。
    // SAFETY: fixtureのMNSDはDatasetの生存中に変更しない。
    let dataset = unsafe { Dataset::open(&args.data, None, None).unwrap() };
    let replay = Replay::open(directory, "pst", &dataset, args.batch, args.epochs).unwrap();
    let epoch = replay.read_epoch(&dataset, 1).unwrap();
    assert_eq!(epoch.mirror[0], 1);
    assert_eq!(
        epoch.indices,
        positions
            .iter()
            .map(|&(file, local)| u64::from(file) * 120 + u64::from(local))
            .collect::<Vec<_>>()
    );
}

fn write_arrays(path: &Path, arrays: &BTreeMap<String, Array>) {
    let values: Vec<_> = arrays
        .iter()
        .map(|(name, array)| (name.as_str(), array.clone()))
        .collect();
    fs::write(path, npz::encode(&values).unwrap()).unwrap();
}

#[test]
fn replay_initial_requires_f32_shape_mirror_sharing_and_bitwise_init_match() {
    let (_fixture, args, _) = fixture();
    let path = args.replay.as_ref().unwrap().join("initial.npz");
    for (values, reason) in [
        (vec![[0.125; 2]; FEATURE_COUNT], "differs from --init"),
        (vec![[-0.0; 2]; FEATURE_COUNT], "differs from --init"),
        (
            {
                let mut v = vec![[0.0; 2]; FEATURE_COUNT];
                v[0][0] = 0.125;
                v
            },
            "mirror sharing",
        ),
        (vec![[f32::NAN; 2]; FEATURE_COUNT], "finite"),
    ] {
        fs::write(&path, common::npz_bytes(&values, true)).unwrap();
        rejects(&args, reason);
    }
    let valid = npz::decode(&pst::encode_pst_npz(&vec![[0.0; 2]; FEATURE_COUNT]).unwrap()).unwrap();
    for array in [
        Array {
            dtype: "<f8".into(),
            shape: vec![FEATURE_COUNT],
            data: vec![0; FEATURE_COUNT * 8],
        },
        Array {
            dtype: "<f4".into(),
            shape: vec![FEATURE_COUNT, 1],
            data: vec![0; FEATURE_COUNT * 4],
        },
        Array {
            dtype: "<f4".into(),
            shape: vec![FEATURE_COUNT - 1],
            data: vec![0; (FEATURE_COUNT - 1) * 4],
        },
    ] {
        let mut arrays = valid.clone();
        arrays.insert("middlegame".into(), array);
        write_arrays(&path, &arrays);
        rejects(&args, "dtype or shape");
    }
}

fn f32s(array: &Array) -> Vec<f32> {
    array
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&v| f32::from_le_bytes(v))
        .collect()
}
fn f64s(array: &Array) -> Vec<f64> {
    array
        .data
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&v| f64::from_le_bytes(v))
        .collect()
}

#[test]
fn replay_steps_exports_exact_keys_types_shapes_and_losses_before_updates() {
    let (fixture, args, positions) = fixture();
    for limit in [1, 3, positions.len().div_ceil(args.batch) as u64 + 1] {
        let mut steps = args.clone();
        steps.output_dir = fixture.root.join(format!("steps-{limit}"));
        steps.replay_steps = Some(limit);
        steps.removal_reference = RemovalReferenceKind::V0;
        steps.removal_penalty = 7.0;
        let summary = pst::run(steps.clone()).unwrap();
        assert_eq!(summary.stopped_by, pst::StopReason::ReplaySteps);
        assert_eq!(summary.total_steps, limit);
        let arrays =
            npz::decode(&fs::read(steps.output_dir.join("rust-steps.npz")).unwrap()).unwrap();
        assert_eq!(arrays.len(), 10);
        for key in ["loss_bce", "loss_removal", "loss_total"] {
            assert_eq!(arrays[key].dtype, "<f8");
            assert_eq!(arrays[key].shape, [limit as usize]);
        }
        let bce = f64s(&arrays["loss_bce"]);
        let removal = f64s(&arrays["loss_removal"]);
        let total = f64s(&arrays["loss_total"]);
        // 初期重み0ならlogit=0、v0除去の不足は各対象で0.25 cp。
        assert!((bce[0] - 2_f64.ln()).abs() < 1e-14);
        assert!((removal[0] - 0.25 / 317.25).abs() < 1e-14);
        for i in 0..limit as usize {
            assert!(bce[i].is_finite() && removal[i].is_finite());
            assert_eq!(total[i], bce[i] + 7.0 * removal[i]);
        }
        for key in [
            "middlegame",
            "endgame",
            "exp_avg_middlegame",
            "exp_avg_endgame",
            "exp_avg_sq_middlegame",
            "exp_avg_sq_endgame",
        ] {
            assert_eq!(arrays[key].dtype, "<f4");
            assert_eq!(arrays[key].shape, [FEATURE_COUNT]);
            let values = f32s(&arrays[key]);
            assert!(values.iter().all(|v| v.is_finite()));
            for i in 0..FEATURE_COUNT {
                assert_eq!(values[i].to_bits(), values[mirror_feature(i)].to_bits());
            }
        }
        assert!(f32s(&arrays["endgame"]).iter().any(|&v| v != 0.0));
        assert!(f32s(&arrays["exp_avg_sq_endgame"]).iter().any(|&v| v > 0.0));
        assert_eq!(arrays["step"].dtype, "<i8");
        assert!(arrays["step"].shape.is_empty());
        assert_eq!(
            i64::from_le_bytes(arrays["step"].data.clone().try_into().unwrap()),
            limit as i64
        );
        assert!(!steps.output_dir.join("pst.bin").exists());
        // エポック途中の状態をlatest.binに偽装しない。
        let checkpoint = Checkpoint::read(steps.output_dir.join("latest.bin")).unwrap();
        assert_eq!(
            checkpoint.optimizer.step(),
            checkpoint.progress.epoch * positions.len().div_ceil(args.batch) as u64
        );
    }
}

#[test]
fn replay_order_controls_updates_and_resume_checks_replay_identity() {
    let (fixture, args, positions) = fixture();
    pst::run(args.clone()).unwrap();
    let config = document(args.output_dir.join("config.json"));
    let directory = args.replay.as_ref().unwrap();
    let manifest = directory.join("replay.json");
    assert_eq!(
        config["settings"]["replay"]["path"],
        json!(directory.canonicalize().unwrap())
    );
    assert_eq!(
        config["settings"]["replay"]["replay_json_sha256"],
        json!(format!(
            "{:x}",
            Sha256::digest(fs::read(&manifest).unwrap())
        ))
    );
    let mut split = args.clone();
    split.output_dir = fixture.root.join("split");
    split.epochs = 1;
    pst::run(split.clone()).unwrap();
    split.epochs = 2;
    split.resume = Some(split.output_dir.clone());
    split.threads = Some(3);
    pst::run(split.clone()).unwrap();
    compare_checkpoints(
        &args.output_dir.join("latest.bin"),
        &split.output_dir.join("latest.bin"),
    );
    // 全エポック分のN更新と通常の再生学習は、同じ重み・Adam状態に到達する。
    let mut steps = args.clone();
    steps.output_dir = fixture.root.join("all-steps");
    steps.replay_steps = Some((2 * positions.len().div_ceil(args.batch)) as u64);
    pst::run(steps.clone()).unwrap();
    let arrays = npz::decode(&fs::read(steps.output_dir.join("rust-steps.npz")).unwrap()).unwrap();
    let checkpoint = Checkpoint::read(args.output_dir.join("latest.bin")).unwrap();
    for (column, name) in ["middlegame", "endgame"].into_iter().enumerate() {
        assert_eq!(
            f32s(&arrays[name]),
            checkpoint
                .model
                .expanded()
                .iter()
                .map(|row| row[column])
                .collect::<Vec<_>>()
        );
        // 正準行の展開はModelの既存公開インターフェースで確認する。
        for (prefix, moments) in [
            ("exp_avg_", checkpoint.optimizer.m()),
            ("exp_avg_sq_", checkpoint.optimizer.v()),
        ] {
            let mut expanded = checkpoint.model.clone();
            expanded
                .weights_mut()
                .copy_from_slice(&moments[..FEATURE_COUNT]);
            assert_eq!(
                f32s(&arrays[&format!("{prefix}{name}")]),
                expanded
                    .expanded()
                    .iter()
                    .map(|row| row[column])
                    .collect::<Vec<_>>()
            );
        }
    }
    let before = fs::read(split.output_dir.join("latest.bin")).unwrap();
    split.replay = None;
    assert!(
        pst::run(split.clone())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
    split.replay = args.replay.clone();
    // 内容が同じJSONでもバイト列が違えば、そのSHA-256を再開時に拒否する。
    let mut bytes = fs::read(&manifest).unwrap();
    bytes.push(b'\n');
    fs::write(&manifest, bytes).unwrap();
    assert!(
        pst::run(split.clone())
            .unwrap_err()
            .to_string()
            .contains("hash mismatch")
    );
    assert_eq!(
        before,
        fs::read(split.output_dir.join("latest.bin")).unwrap()
    );
}

#[test]
fn replay_steps_cli_exits_successfully_without_best_epoch() {
    let (_fixture, args, _) = fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["pst", "train", "--data"])
        .args(&args.data)
        .arg("--init")
        .arg(&args.init)
        .arg("--output-dir")
        .arg(&args.output_dir)
        .arg("--replay")
        .arg(args.replay.as_ref().unwrap())
        .args([
            "--batch",
            "16",
            "--epochs",
            "2",
            "--patience",
            "5",
            "--removal-reference",
            "init",
            "--threads",
            "1",
            "--replay-steps",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(args.output_dir.join("rust-steps.npz").exists());
}

#[test]
fn replay_uses_recorded_order_instead_of_seed_and_stops_before_later_batches() {
    let (fixture, mut args, positions) = fixture();
    args.replay_steps = Some(3);
    pst::run(args.clone()).unwrap();
    let original = fs::read(args.output_dir.join("rust-steps.npz")).unwrap();
    let mut changed = args.clone();
    changed.output_dir = fixture.root.join("other-seed");
    changed.seed = 999;
    pst::run(changed).unwrap();
    assert_eq!(
        original,
        fs::read(fixture.root.join("other-seed/rust-steps.npz")).unwrap()
    );

    let directory = args.replay.as_ref().unwrap();
    let mut tail = positions.clone();
    tail[args.batch * 3..].reverse();
    write_order(directory, 1, &tail);
    let mut changed = args.clone();
    changed.output_dir = fixture.root.join("other-tail");
    pst::run(changed).unwrap();
    assert_eq!(
        original,
        fs::read(fixture.root.join("other-tail/rust-steps.npz")).unwrap()
    );

    let mut reversed = positions;
    reversed.reverse();
    write_order(directory, 1, &reversed);
    args.output_dir = fixture.root.join("other-prefix");
    pst::run(args.clone()).unwrap();
    let changed = fs::read(args.output_dir.join("rust-steps.npz")).unwrap();
    assert_ne!(
        npz::decode(&original).unwrap()["endgame"],
        npz::decode(&changed).unwrap()["endgame"]
    );
}

#[test]
fn replay_accepts_mirror_averaged_initialization_and_exports_after_clamping() {
    let (_fixture, mut args, _) = fixture();
    args.removal_penalty = 0.0;
    args.replay_steps = Some(1);
    args.lr = 1e6;
    let mut weights = vec![[0; 2]; FEATURE_COUNT];
    weights[0] = [4, 12];
    weights[mirror_feature(0)] = [12, 20];
    let base = Pst::from_weights(&weights, v0_piece_values(), 317.25, "L0,P0,R1,E0").unwrap();
    fs::write(&args.init, base.encode()).unwrap();
    let mut expanded = vec![[0.0; 2]; FEATURE_COUNT];
    expanded[0] = [1.0, 2.0];
    expanded[mirror_feature(0)] = [1.0, 2.0];
    fs::write(
        args.replay.as_ref().unwrap().join("initial.npz"),
        pst::encode_pst_npz(&expanded).unwrap(),
    )
    .unwrap();
    pst::run(args.clone()).unwrap();
    let arrays = npz::decode(&fs::read(args.output_dir.join("rust-steps.npz")).unwrap()).unwrap();
    let values = f32s(&arrays["endgame"]);
    assert!(values.iter().all(|v| (-4096.0..=4095.875).contains(v)));
    assert!(values.iter().any(|&v| v == -4096.0 || v == 4095.875));
}

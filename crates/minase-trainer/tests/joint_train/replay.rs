//! 指示書の再生形式を手で作り、CLIの拒否条件と更新状態を検査する。
use super::*;
use minase::training::records::Header;
use minase_trainer::{
    data::is_validation,
    nn::npz::{self, Array},
};
use serde_json::json;
use std::collections::BTreeMap;

fn fixture() -> (Fixture, Args, Vec<(u32, u32)>) {
    let (fixture, mut args) = Fixture::new(false);
    args.epochs = 2;
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
            "format": "minase-replay-1", "trainer": "joint", "files": inputs,
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
    let (model, _) = initialize(&args);
    write_arrays(&directory.join("initial.npz"), &initial_arrays(&model));
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
        (0..positions.len())
            .map(|i| (i % 2) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
}

fn rejects(args: &Args, reason: &str) {
    let error = joint::train(args.clone()).unwrap_err().to_string();
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
        (|v| v["trainer"] = json!("pst"), "format or trainer"),
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
        // 通常の再生学習では、要求した全エポックを検査する。
        assert!(joint::train(args.clone()).is_err());
        assert!(!args.output_dir.exists());
        fs::write(&path, bytes).unwrap();
    }
}

fn floats(values: impl IntoIterator<Item = f32>, shape: Vec<usize>) -> Array {
    Array {
        dtype: "<f4".into(),
        shape,
        data: values.into_iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}
fn initial_arrays(model: &Model) -> BTreeMap<String, Array> {
    let endpoints = model.pst.expanded();
    BTreeMap::from([
        (
            "middlegame".into(),
            floats(endpoints.iter().map(|r| r[0]), vec![FEATURE_COUNT]),
        ),
        (
            "endgame".into(),
            floats(endpoints.iter().map(|r| r[1]), vec![FEATURE_COUNT]),
        ),
        (
            "V".into(),
            floats(
                model.fm.v().iter().flatten().copied(),
                vec![FEATURE_COUNT, FM_RANK],
            ),
        ),
        (
            "a".into(),
            floats(model.fm.a().iter().copied(), vec![FM_RANK]),
        ),
        (
            "mask".into(),
            Array {
                dtype: "|b1".into(),
                shape: vec![FEATURE_COUNT],
                data: model.fm.mask().iter().map(|&v| u8::from(v)).collect(),
            },
        ),
    ])
}
fn write_arrays(path: &Path, arrays: &BTreeMap<String, Array>) {
    fs::write(
        path,
        npz::encode(
            &arrays
                .iter()
                .map(|(k, v)| (k.as_str(), v.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap(),
    )
    .unwrap();
}
fn f32s(array: &Array) -> Vec<f32> {
    array
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| f32::from_le_bytes(b))
        .collect()
}
fn f64s(array: &Array) -> Vec<f64> {
    array
        .data
        .as_chunks::<8>()
        .0
        .iter()
        .map(|&b| f64::from_le_bytes(b))
        .collect()
}

#[test]
fn initial_rejects_wrong_shapes_dtypes_masks_and_nonzero_unobserved_rows() {
    let (_fixture, args, _) = fixture();
    let path = args.replay.as_ref().unwrap().join("initial.npz");
    let original = npz::decode(&fs::read(&path).unwrap()).unwrap();
    for key in ["middlegame", "endgame", "V", "a", "mask"] {
        let mut arrays = original.clone();
        arrays.remove(key);
        write_arrays(&path, &arrays);
        rejects(&args, "keys");
        let mut arrays = original.clone();
        arrays.get_mut(key).unwrap().shape.push(1);
        write_arrays(&path, &arrays);
        rejects(&args, "differs");
    }
    let mut arrays = original.clone();
    arrays.get_mut("V").unwrap().dtype = "<i4".into();
    write_arrays(&path, &arrays);
    rejects(&args, "differs");
    let mut arrays = original.clone();
    arrays.get_mut("mask").unwrap().data[0] ^= 1;
    write_arrays(&path, &arrays);
    rejects(&args, "mask differs");
    let mut arrays = original.clone();
    arrays.get_mut("mask").unwrap().data[0] = 2;
    write_arrays(&path, &arrays);
    rejects(&args, "differs");
    let f = original["mask"].data.iter().position(|&v| v == 0).unwrap();
    let mut arrays = original.clone();
    arrays.get_mut("V").unwrap().data[f * FM_RANK * 4..f * FM_RANK * 4 + 4]
        .copy_from_slice(&1.0_f32.to_le_bytes());
    write_arrays(&path, &arrays);
    rejects(&args, "differs");
    let mut arrays = original;
    arrays.get_mut("a").unwrap().data[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    write_arrays(&path, &arrays);
    rejects(&args, "differs");
}

#[test]
fn replay_requires_enough_mirrors_for_the_requested_step_prefix() {
    let (fixture, mut args, positions) = fixture();
    let directory = args.replay.as_ref().unwrap();
    let path = directory.join("epoch-001.mirror");
    for bits in [vec![0; positions.len() - 1], vec![2; positions.len()]] {
        fs::write(&path, bits).unwrap();
        rejects(&args, "mirror length or bit");
    }
    args.replay_steps = Some(3);
    fs::write(&path, vec![0; 3 * args.batch - 1]).unwrap();
    rejects(&args, "mirror length");
    fs::write(
        &path,
        (0..3 * args.batch)
            .map(|i| (i % 2) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    joint::train(args.clone()).unwrap();
    assert!(args.output_dir.join("rust-steps.npz").exists());
    args.output_dir = fixture.root.join("shorter-prefix");
    args.replay_steps = Some(1);
    joint::train(args.clone()).unwrap();
    assert!(args.output_dir.join("rust-steps.npz").exists());
}

#[test]
fn replay_steps_have_exact_keys_dtypes_shapes_and_losses_before_updates() {
    let (fixture, args, positions) = fixture();
    for limit in [1, 3, positions.len().div_ceil(args.batch) as u64 + 1] {
        let mut run = args.clone();
        run.replay_steps = Some(limit);
        run.output_dir = fixture.root.join(format!("steps-{limit}"));
        let summary = joint::train(run.clone()).unwrap();
        assert_eq!(summary.total_steps, limit);
        assert_eq!(summary.status, "replay_steps");
        let arrays =
            npz::decode(&fs::read(run.output_dir.join("rust-steps.npz")).unwrap()).unwrap();
        assert_eq!(arrays.len(), 17);
        for key in ["loss_bce", "loss_removal", "loss_reg", "loss_total"] {
            assert_eq!(arrays[key].dtype, "<f8");
            assert_eq!(arrays[key].shape, [limit as usize]);
        }
        let bce = f64s(&arrays["loss_bce"]);
        let reg = f64s(&arrays["loss_reg"]);
        let total = f64s(&arrays["loss_total"]);
        let removal = f64s(&arrays["loss_removal"]);
        for i in 0..limit as usize {
            assert_eq!(
                total[i],
                bce[i] + reg[i] + args.removal_penalty * removal[i]
            );
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
        }
        for key in ["V", "exp_avg_V", "exp_avg_sq_V"] {
            assert_eq!(arrays[key].dtype, "<f4");
            assert_eq!(arrays[key].shape, [FEATURE_COUNT, FM_RANK]);
            assert!(f32s(&arrays[key]).iter().all(|v| v.is_finite()));
        }
        for key in ["a", "exp_avg_a", "exp_avg_sq_a"] {
            assert_eq!(arrays[key].dtype, "<f4");
            assert_eq!(arrays[key].shape, [FM_RANK]);
        }
        assert!(f32s(&arrays["a"]).iter().any(|&v| v != 0.0));
        assert_eq!(arrays["step"].dtype, "<i8");
        assert_eq!(arrays["step"].shape, Vec::<usize>::new());
        assert_eq!(
            i64::from_le_bytes(arrays["step"].data.clone().try_into().unwrap()),
            limit as i64
        );
        assert!(!run.output_dir.join("pst.bin").exists());
        let checkpoint = Checkpoint::read(run.output_dir.join("latest.bin")).unwrap();
        assert_eq!(
            checkpoint.optimizer.step(),
            checkpoint.progress.epoch * positions.len().div_ceil(args.batch) as u64
        );
    }
}

#[test]
fn replay_compares_all_initial_weight_bits_and_uses_recorded_mirrors() {
    let (fixture, mut args, _) = fixture();
    let path = args.replay.as_ref().unwrap().join("initial.npz");
    let original = npz::decode(&fs::read(&path).unwrap()).unwrap();
    for key in ["middlegame", "endgame", "V", "a"] {
        let mut arrays = original.clone();
        // A one-bit difference must fail, even when it does not affect quantization.
        arrays.get_mut(key).unwrap().data[0] ^= 1;
        write_arrays(&path, &arrays);
        rejects(&args, "differs");
    }
    write_arrays(&path, &original);
    args.replay_steps = Some(3);
    joint::train(args.clone()).unwrap();
    let expected = fs::read(args.output_dir.join("rust-steps.npz")).unwrap();
    args.seed = 999;
    args.threads = Some(3);
    args.output_dir = fixture.root.join("other-seed");
    joint::train(args.clone()).unwrap();
    assert_eq!(
        expected,
        fs::read(args.output_dir.join("rust-steps.npz")).unwrap()
    );
    let path = args.replay.as_ref().unwrap().join("epoch-001.mirror");
    let mirrors = fs::read(&path)
        .unwrap()
        .iter()
        .map(|b| 1 - b)
        .collect::<Vec<_>>();
    fs::write(path, mirrors).unwrap();
    args.output_dir = fixture.root.join("other-mirrors");
    joint::train(args.clone()).unwrap();
    assert_ne!(
        expected,
        fs::read(args.output_dir.join("rust-steps.npz")).unwrap()
    );
}

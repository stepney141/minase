//! 数値診断の出力契約を、手計算できる合成入力と実コマンドで検査する。

mod common;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use minase::{
    eval::{
        features::{FEATURE_COUNT, PIECE_STATE_COUNT},
        pst::{FM_RANK, Pst},
    },
    training::records::{Header, Outcome, Record},
};
use minase_core::{Color, Game, PieceCode, PieceKind, Position, PositionBuilder, Rules, Square};
use minase_trainer::nnue::{
    data::InputFeatureSet,
    model::{Checkpoint, Model, Parameter, Progress, Shape, quantize},
    reference::{AdamW, AdamWConfig, v0_piece_values},
};
use serde_json::Value;
use sha2::{Digest, Sha256};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "minase-diagnostics-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn data(
        &self,
        name: &str,
        positions: &[Position],
        seed: u64,
        games: &[u32],
        rules: &str,
    ) -> PathBuf {
        let records: Vec<_> = positions
            .iter()
            .zip(games)
            .enumerate()
            .map(|(i, (position, &game))| {
                Record::from_position(
                    position,
                    0,
                    Outcome::Draw,
                    game,
                    i as u16,
                    minase::training::records::Origin::RandomSelfplay,
                )
            })
            .collect();
        self.records(name, &records, seed, rules)
    }

    fn records(&self, name: &str, records: &[Record], seed: u64, rules: &str) -> PathBuf {
        let path = self.path(name);
        let mut bytes = Header::new(
            rules.into(),
            "1".repeat(40),
            [2; 32],
            200,
            seed,
            records.len() as u64,
        )
        .unwrap()
        .encode()
        .to_vec();
        for record in records {
            bytes.extend(record.encode());
        }
        fs::write(&path, &bytes).unwrap();
        path
    }

    fn baseline(&self, name: &str, linear: &[[f32; 2]], fm: i16) -> PathBuf {
        // FMの第1次元だけ全特徴をfmとし、補正はfm²*n*(n-1)/2。
        let weights: Vec<_> = linear
            .iter()
            .map(|row| row.map(|value| (value * 8.0) as i16))
            .collect();
        let mut embedding = [0; FM_RANK];
        embedding[0] = fm;
        let bytes = Pst::from_weights_with_fm(
            &weights,
            v0_piece_values(),
            1016.0,
            "engine-default",
            minase::eval::pst::FmWeights {
                embeddings: &vec![embedding; FEATURE_COUNT],
                signs: &[1; FM_RANK],
                exponent: 0,
            },
        )
        .unwrap()
        .encode();
        let path = self.path(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn network(&self, name: &str, model: &Model) -> PathBuf {
        let path = self.path(name);
        fs::write(
            &path,
            quantize(model)
                .unwrap()
                .encode(&v0_piece_values(), "engine-default")
                .unwrap(),
        )
        .unwrap();
        path
    }

    fn checkpoint(&self, model: Model) -> PathBuf {
        let path = self.path("nnue-float.bin");
        Checkpoint {
            optimizer: AdamW::new(model.shape(), AdamWConfig::default()).unwrap(),
            model,
            removal_penalty: 0.0,
            lambda: 1.0,
            progress: Progress {
                epoch: 0,
                best: None,
                elapsed_seconds: 0.0,
                seed: 1,
                config_hash: [0; 32],
                input_hash: [0; 32],
            },
        }
        .write(&path)
        .unwrap();
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
            .current_dir(&self.root)
            .arg("diag")
            .args(args)
            .output()
            .unwrap()
    }

    fn success(&self, args: &[&str], name: &str) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&fs::read(self.path(name)).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn model(set: InputFeatureSet, linear: &[[f32; 2]], k: f32) -> Model {
    Model::initialize(
        Shape::new(set, 4, k).unwrap(),
        linear,
        common::all_observed(set),
        1,
        0.0,
    )
    .unwrap()
}

fn pawn_linear(cp: f32) -> Vec<[f32; 2]> {
    let mut linear = vec![[0.0; 2]; FEATURE_COUNT];
    for relative in 0..2 {
        for square in 0..144 {
            linear[(relative * PIECE_STATE_COUNT + 29) * 144 + square] =
                [if relative == 0 { cp } else { -cp }; 2];
        }
    }
    linear
}

fn assert_close(actual: &Value, expected: f64, tolerance: f64) {
    let actual = actual.as_f64().unwrap();
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

fn root_keys(fixture: &Fixture, name: &str) -> Vec<String> {
    fs::read_to_string(fixture.path(name))
        .unwrap()
        .lines()
        .filter(|line| line.starts_with("  \""))
        .map(|line| line.split('"').nth(1).unwrap().to_owned())
        .collect()
}

fn distribution_positions() -> Vec<Position> {
    let mut positions = Vec::new();
    // 各位相帯・絶対値帯に異なる2値以上を置く。基準は50 cp×歩の枚数。
    for count in [27, 42, 72] {
        for pawns in [0, 1, 2, 3, 6, 7, 20, 21] {
            let mut builder = PositionBuilder::new(Color::Black);
            for i in 0..count - 2 {
                let kind = if i < pawns {
                    PieceKind::Pawn
                } else {
                    PieceKind::GoldGeneral
                };
                builder
                    .put(
                        Square::from_dense(i).unwrap(),
                        PieceCode::new(Color::Black, kind).unwrap(),
                    )
                    .unwrap();
            }
            for (i, side) in [(142, Color::Black), (143, Color::White)] {
                builder
                    .put(
                        Square::from_dense(i).unwrap(),
                        PieceCode::new(side, PieceKind::King).unwrap(),
                    )
                    .unwrap();
            }
            positions.push(builder.finish().unwrap());
        }
    }
    positions
}

#[test]
fn distribution_preserves_python_schema_order_statistics_and_identities() {
    let f = Fixture::new();
    let positions = distribution_positions();
    // 数値診断は評価用の復元を使い、先獅子の升に手番側の駒がある記録も受理する。
    for (name, seed, game) in [("a.mnsd", 11, 4), ("b.mnsd", 22, 6)] {
        let records: Vec<_> = positions
            .iter()
            .map(|position| {
                let mut bytes = Record::from_position(
                    position,
                    0,
                    Outcome::Draw,
                    game,
                    0,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .encode();
                bytes[145] = 0;
                let record = Record::decode(&bytes).unwrap();
                assert!(record.to_position().is_err());
                record
            })
            .collect();
        f.records(name, &records, seed, "engine-default");
    }
    f.baseline("base.bin", &pawn_linear(50.0), 0);
    for (set, name) in [
        (InputFeatureSet::P, "p.bin"),
        (InputFeatureSet::HalfKpHmB8, "half.bin"),
    ] {
        f.network(name, &model(set, &pawn_linear(100.0), 1016.0));
    }
    // 全局面を標本にして、各位相帯と評価帯の既知の分布を検査する。
    let args = [
        "nnue-distribution",
        "--data",
        "a.mnsd",
        "b.mnsd",
        "--sample",
        "48",
        "--seed",
        "7",
        "--pair",
        "half",
        "base.bin",
        "half.bin",
        "--pair",
        "p",
        "base.bin",
        "p.bin",
        "--output",
        "report.json",
    ];
    let report = f.success(&args, "report.json");
    assert_eq!(
        root_keys(&f, "report.json"),
        [
            "sample_count",
            "seed",
            "sample_indices_sha256_le_i64",
            "input_files",
            "population_count",
            "sampling",
            "evaluation_perspective",
            "standard_deviation_ddof",
            "percentile_method",
            "spearman_tie_method",
            "regression_equation",
            "unchanged_scale_interval_inclusive",
            "pairs"
        ]
    );
    assert_eq!(report["sample_count"], 48);
    assert_eq!(report["population_count"], 48);
    assert!(report["sampling"].as_str().unwrap().contains("SplitMix64"));
    assert_eq!(
        report["sample_indices_sha256_le_i64"],
        format!(
            "{:x}",
            Sha256::digest((0_i64..48).flat_map(i64::to_le_bytes).collect::<Vec<_>>())
        )
    );
    for (i, name) in ["a.mnsd", "b.mnsd"].iter().enumerate() {
        assert_eq!(
            report["input_files"][i]["path"],
            f.path(name).canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(report["input_files"][i]["record_count"], 24);
        assert_eq!(
            report["input_files"][i]["byte_length"],
            fs::metadata(f.path(name)).unwrap().len()
        );
    }
    assert_eq!(report["pairs"][0]["name"], "half");
    assert_eq!(report["pairs"][1]["name"], "p");
    for pair in report["pairs"].as_array().unwrap() {
        assert_close(&pair["baseline_statistics"]["mean_cp"], 375.0, 1e-12);
        assert_close(&pair["candidate_statistics"]["mean_cp"], 750.0, 1e-12);
        assert_close(&pair["std_ratio_candidate_over_baseline"], 2.0, 1e-12);
        assert_close(&pair["regression_slope"], 2.0, 1e-12);
        assert_close(&pair["regression_intercept_cp"], 0.0, 1e-12);
        assert_close(&pair["spearman_correlation"], 1.0, 1e-12);
        assert_close(&pair["regression_residual_std_cp"], 0.0, 1e-12);
        assert_eq!(pair["verdict"], "散らばりの尺度を変えた");
        for interval in pair["by_phase_q"].as_array().unwrap() {
            assert_eq!(interval["sample_count"], 16);
        }
        // 基準値は各絶対値帯に2種類ずつ、2ファイルで計12件。
        for interval in pair["by_baseline_absolute_cp"].as_array().unwrap() {
            assert_eq!(interval["sample_count"], 12);
        }
    }
    let saved = fs::read(f.path("report.json")).unwrap();
    assert_eq!(f.run(&args).status.code(), Some(1));
    assert_eq!(fs::read(f.path("report.json")).unwrap(), saved);
}

#[test]
fn distribution_sampling_is_deterministic_and_invalid_inputs_create_no_report() {
    let f = Fixture::new();
    let positions = distribution_positions();
    f.data("data.mnsd", &positions, 11, &[4; 24], "engine-default");
    f.baseline("base.bin", &pawn_linear(50.0), 0);
    f.network(
        "net.bin",
        &model(InputFeatureSet::P, &pawn_linear(50.0), 1016.0),
    );
    let args = [
        "nnue-distribution",
        "--data",
        "data.mnsd",
        "--sample",
        "24",
        "--seed",
        "9",
        "--pair",
        "same",
        "base.bin",
        "net.bin",
        "--output",
        "report.json",
    ];
    let first = f.success(&args, "report.json");
    fs::remove_file(f.path("report.json")).unwrap();
    let second = f.success(&args, "report.json");
    assert_eq!(first, second);
    assert!(first["sampling"].as_str().unwrap().contains("SplitMix64"));
    assert_eq!(
        first["pairs"][0]["verdict"],
        "全体の尺度はほとんど変えていない"
    );
    fs::remove_file(f.path("report.json")).unwrap();
    let mut duplicate = args.to_vec();
    duplicate.extend(["--pair", "same", "base.bin", "net.bin"]);
    assert_eq!(f.run(&duplicate).status.code(), Some(1));
    assert!(!f.path("report.json").exists());
    // 空の帯や分散0は報告を作成せずに拒否する。
    f.network(
        "constant.bin",
        &model(InputFeatureSet::P, &vec![[0.0; 2]; FEATURE_COUNT], 1016.0),
    );
    let mut constant = args;
    constant[10] = "constant.bin";
    assert_eq!(f.run(&constant).status.code(), Some(1));
    assert!(!f.path("report.json").exists());
}

fn removal_args(sample: &str) -> Vec<&str> {
    vec![
        "nnue-removal",
        "--float",
        "nnue-float.bin",
        "--net",
        "net.bin",
        "--data",
        "data.mnsd",
        "--sample",
        sample,
        "--seed",
        "8",
        "--output",
        "report.json",
    ]
}

#[test]
fn removal_reports_known_reversals_and_keeps_the_lion_square_when_its_piece_is_removed() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let f = Fixture::new();
        let mut builder = PositionBuilder::new(Color::Black);
        for (index, side, kind, promoted) in [
            (0, Color::Black, PieceKind::King, false),
            (143, Color::White, PieceKind::King, false),
            (1, Color::Black, PieceKind::CrownPrince, true),
            (50, Color::Black, PieceKind::Pawn, false),
            (60, Color::White, PieceKind::Pawn, false),
            (70, Color::White, PieceKind::Lion, true),
        ] {
            let piece = if promoted {
                PieceCode::new_promoted(side, kind)
            } else {
                PieceCode::new(side, kind)
            }
            .unwrap();
            builder
                .put(Square::from_dense(index).unwrap(), piece)
                .unwrap();
        }
        let mut position = builder.finish().unwrap();
        position.set_lion_capture(Square::from_dense(70)).unwrap();
        let mut record = Record::from_position(
            &position,
            0,
            Outcome::Draw,
            4,
            0,
            minase::training::records::Origin::RandomSelfplay,
        )
        .encode();
        assert_eq!(record[146], 1);
        // 数値診断の特徴に含めない麒麟成りフラグを、入力で意図的に不一致にする。
        record[146] = 0;
        let record = Record::decode(&record).unwrap();
        assert!(record.to_position().is_err());
        assert!(record.to_position_for_evaluation().is_ok());
        f.records("data.mnsd", &[record], 11, "engine-default");
        let values = v0_piece_values();
        let linear: Vec<_> = (0..FEATURE_COUNT)
            .map(|feature| {
                if feature >= 2 * PIECE_STATE_COUNT * 144 {
                    [37.0; 2]
                } else {
                    [if feature / (PIECE_STATE_COUNT * 144) == 0 {
                        -(values[feature / 144 % PIECE_STATE_COUNT] as f32)
                    } else {
                        values[feature / 144 % PIECE_STATE_COUNT] as f32
                    }; 2]
                }
            })
            .collect();
        let m = model(set, &linear, 1016.0);
        f.network("net.bin", &m);
        f.checkpoint(m);
        let args = removal_args("1");
        let report = f.success(&args, "report.json");
        assert_eq!(
            root_keys(&f, "report.json"),
            [
                "sample_count",
                "seed",
                "sample_indices_sha256_le_i64",
                "input_files",
                "split",
                "population_count",
                "float",
                "net",
                "removal_count",
                "reference",
                "float_batch_vs_recomputed",
                "float_vs_integer_recomputed",
                "reversals",
                "by_piece_state"
            ]
        );
        assert_eq!(report["removal_count"], 3);
        assert_eq!(
            report["reversals"],
            serde_json::json!({ "count": 3, "total": 3, "fraction": 1.0, "mean_absolute_cp": 900.0, "max_absolute_cp": 2500.0 })
        );
        assert_eq!(report["by_piece_state"]["20"]["count"], 1);
        assert_eq!(report["by_piece_state"]["29"]["count"], 2);
        assert!(report["by_piece_state"].get("11").is_none());
        assert!(report["by_piece_state"].get("21").is_none());
        assert_eq!(report["float_batch_vs_recomputed"]["passed"], true);
        assert_close(
            &report["float_vs_integer_recomputed"]["max_absolute_cp"],
            0.0,
            1e-9,
        );
        let text = fs::read_to_string(f.path("report.json")).unwrap();
        assert!(text.find("\"20\":").unwrap() < text.find("\"29\":").unwrap());
    }
}

#[test]
fn removal_uses_f64_recomputation_and_the_integer_network_k() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let f = Fixture::new();
        let position =
            common::position(&[(2, 4, Color::Black, PieceKind::Pawn)], Color::Black, None);
        f.data("data.mnsd", &[position], 11, &[4], "engine-default");
        let mut linear = pawn_linear(80.0);
        for row in &mut linear {
            row[0] /= 2.0;
        }
        let mut m = model(set, &linear, 500.0);
        let h = m.shape().width();
        let first = m.parameter_mut(Parameter::FirstLayer);
        let end = first.len() - h;
        for row in first[..end].chunks_mut(h) {
            row.copy_from_slice(&[0.1, 0.2, 0.1, 0.2]);
        }
        m.parameter_mut(Parameter::Biases).fill(0.3);
        let units = m.shape().units();
        m.parameter_mut(Parameter::OutputWeights)[..units].fill(0.125);
        m.parameter_mut(Parameter::OutputBias)[0] = 0.1;
        f.checkpoint(m);
        // 整数側は0評価、K=1000。チェックポイントのK=500を使うとこの既知差分に合わない。
        f.network(
            "net.bin",
            &model(set, &vec![[0.0; 2]; FEATURE_COUNT], 1000.0),
        );
        let report = f.success(&removal_args("1"), "report.json");
        // 実数の差分は -80 +1000*0.25*(2*0.3²-0.4²-0.5²) = -137.5 cp。
        assert_close(
            &report["float_vs_integer_recomputed"]["mean_absolute_cp"],
            137.5,
            1e-4,
        );
        assert_close(
            &report["float_vs_integer_recomputed"]["max_absolute_cp"],
            137.5,
            1e-4,
        );
        assert_eq!(report["float_batch_vs_recomputed"]["passed"], true);
        assert_eq!(report["reversals"]["count"], 0);
        assert_eq!(report["reversals"]["total"], 1);
        assert!(report["reversals"]["mean_absolute_cp"].is_null());

        let position =
            common::position(&[(2, 4, Color::Black, PieceKind::Pawn)], Color::Black, None);
        let mut bytes = Record::from_position(
            &position,
            0,
            Outcome::Draw,
            4,
            0,
            minase::training::records::Origin::RandomSelfplay,
        )
        .encode();
        bytes[145] = 50; // 手番側の歩と同じ升に先獅子を置く。
        let record = Record::decode(&bytes).unwrap();
        assert!(record.to_position().is_err());
        let (placement, perspective) = record.to_position_for_evaluation().unwrap();
        assert_eq!(perspective, Color::Black);
        assert_eq!(placement.side_to_move(), Color::White);
        assert_eq!(placement.lion_capture_square(), Square::from_dense(50));
        placement.validate().unwrap();
        f.records("data.mnsd", &[record], 11, "engine-default");
        fs::remove_file(f.path("report.json")).unwrap();
        let report = f.success(&removal_args("1"), "report.json");
        // 先獅子の特徴は除去後も残り、-80+1000*0.25*(0.4²+0.5²-0.5²-0.7²)=-162.5。
        assert_close(
            &report["float_vs_integer_recomputed"]["mean_absolute_cp"],
            162.5,
            1e-4,
        );
        assert_eq!(report["float_batch_vs_recomputed"]["passed"], true);
    }
}

#[test]
fn removal_samples_without_replacement_using_the_existing_splitmix64_sequence() {
    let f = Fixture::new();
    let pawn = common::position(&[(2, 4, Color::Black, PieceKind::Pawn)], Color::Black, None);
    f.data("data.mnsd", &vec![pawn; 12], 11, &[4; 12], "engine-default");
    let m = model(InputFeatureSet::P, &pawn_linear(100.0), 1016.0);
    f.network("net.bin", &m);
    f.checkpoint(m);
    let mut args = removal_args("4");
    let seed = args.iter().position(|&arg| arg == "--seed").unwrap();
    args[seed + 1] = "7";
    let report = f.success(&args, "report.json");
    // data/shuffle.rsの独立した既知列の先頭4件は3,11,1,4。昇順にすると1,3,4,11。
    let bytes: Vec<_> = [1_i64, 3, 4, 11]
        .into_iter()
        .flat_map(i64::to_le_bytes)
        .collect();
    assert_eq!(
        report["sample_indices_sha256_le_i64"],
        format!("{:x}", Sha256::digest(bytes))
    );
    assert_eq!(report["sample_count"], 4);
    assert_eq!(report["population_count"], 12);
}

#[test]
fn failed_removal_is_saved_before_the_command_returns_exit_code_one() {
    let f = Fixture::new();
    let pawn = common::position(&[(2, 4, Color::Black, PieceKind::Pawn)], Color::Black, None);
    f.data("data.mnsd", &[pawn], 11, &[4], "engine-default");
    let mut m = model(InputFeatureSet::P, &pawn_linear(100.0), 1000.0);
    // 有限だが極端な定数項により、独立再計算では100 cpが桁落ちする。一括差分は-100 cp。
    m.parameter_mut(Parameter::OutputBias)[0] = 1e20;
    f.checkpoint(m);
    f.network(
        "net.bin",
        &model(InputFeatureSet::P, &vec![[0.0; 2]; FEATURE_COUNT], 1000.0),
    );
    let mut args = removal_args("1");
    let last = args.len() - 1;
    args[last] = "nested/report.json";
    let output = f.run(&args);
    assert_eq!(output.status.code(), Some(1));
    let saved = fs::read(f.path("nested/report.json")).unwrap();
    let report: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(report["float_batch_vs_recomputed"]["passed"], false);
    assert_close(
        &report["float_batch_vs_recomputed"]["max_absolute_cp"],
        100.0,
        1e-12,
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains(report["error"].as_str().unwrap()));
    assert_eq!(f.run(&args).status.code(), Some(1));
    assert_eq!(fs::read(f.path("nested/report.json")).unwrap(), saved);
}

#[test]
fn removal_checks_phase_boundaries_and_both_sides_with_nonlinear_weights() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let f = Fixture::new();
        let mut positions = Vec::new();
        for side in Color::ALL {
            for count in [1, 2, 3, 91, 92, 93] {
                let mut builder = PositionBuilder::new(side);
                for i in 0..count {
                    let color = if i % 2 == 0 {
                        Color::Black
                    } else {
                        Color::White
                    };
                    let pawn = PieceCode::new(color, PieceKind::Pawn).unwrap();
                    let piece = if i % 4 >= 2 {
                        pawn.promote().unwrap()
                    } else {
                        pawn
                    };
                    builder.put(Square::from_dense(i).unwrap(), piece).unwrap();
                }
                let mut position = builder.finish().unwrap();
                position.set_lion_capture(Square::from_dense(140)).unwrap();
                positions.push(position);
            }
        }
        f.data("data.mnsd", &positions, 11, &[4; 12], "engine-default");
        let mut linear = pawn_linear(40.0);
        for row in &mut linear {
            row[1] *= 0.9;
        }
        let mut m = model(set, &linear, 1000.0);
        let h = m.shape().width();
        let rows = m.parameter_mut(Parameter::FirstLayer);
        let end = rows.len() - h;
        for (i, value) in rows[..end].iter_mut().enumerate() {
            *value = (i as i32 % 13 - 6) as f32 * 0.001;
        }
        m.parameter_mut(Parameter::Biases).fill(0.13);
        for (i, value) in m
            .parameter_mut(Parameter::OutputWeights)
            .iter_mut()
            .enumerate()
        {
            *value = (i as f32 - 2.0) * 0.1;
        }
        m.parameter_mut(Parameter::OutputBias)[0] = 0.125;
        f.network("net.bin", &m);
        f.checkpoint(m);
        let report = f.success(&removal_args("12"), "report.json");
        assert_eq!(report["removal_count"], 564);
        assert_eq!(report["float_batch_vs_recomputed"]["passed"], true);
        assert!(
            report["float_vs_integer_recomputed"]["max_absolute_cp"]
                .as_f64()
                .unwrap()
                > 0.0
        );
        assert!(report["error"].is_null());
    }
}

fn moves_args() -> [&'static str; 9] {
    [
        "nnue-moves",
        "--net",
        "net.bin",
        "--base",
        "base.bin",
        "--positions",
        "data.mnsd",
        "--output",
        "report.json",
    ]
}

fn tempo_model(set: InputFeatureSet) -> Model {
    let mut m = model(set, &vec![[0.0; 2]; FEATURE_COUNT], 1016.0);
    m.parameter_mut(Parameter::OutputBias)[0] = 0.125;
    m
}

#[test]
fn moves_use_negamax_and_include_fm_while_empty_positions_have_null_statistics() {
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let f = Fixture::new();
        let kings = [
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
        ];
        let positions = [
            common::position(&kings, Color::Black, None),
            common::position(&kings, Color::White, None),
            Position::empty(Color::Black),
        ];
        f.data("data.mnsd", &positions, 11, &[1; 3], "L0,P0,R1,E0");
        f.baseline("base.bin", &vec![[0.0; 2]; FEATURE_COUNT], 2);
        f.network("net.bin", &tempo_model(set));
        let report = f.success(&moves_args(), "report.json");
        assert_eq!(
            root_keys(&f, "report.json"),
            [
                "net",
                "base",
                "positions",
                "position_count",
                "evaluation_perspective",
                "standard_deviation_ddof",
                "candidate_only_count",
                "baseline_only_count",
                "by_position"
            ]
        );
        assert_eq!(report["candidate_only_count"], 0);
        assert_eq!(report["baseline_only_count"], 0);
        for i in 0..2 {
            let row = &report["by_position"][i];
            assert_eq!(row["matched_count"], 3);
            // NNUEは両視点で127 cp、FMは両視点で4 cpなので、それぞれ-254と-8。
            for (key, expected) in [
                ("candidate", -254.0),
                ("baseline", -8.0),
                ("candidate_minus_baseline", -246.0),
                ("candidate_minus_baseline_pst", -254.0),
            ] {
                assert_eq!(row[key]["count"], 3);
                assert_close(&row[key]["mean_cp"], expected, 0.0);
                assert_close(&row[key]["std_cp"], 0.0, 0.0);
            }
        }
        assert_eq!(report["by_position"][2]["matched_count"], 0);
        for key in [
            "candidate",
            "baseline",
            "candidate_minus_baseline",
            "candidate_minus_baseline_pst",
        ] {
            let stats = &report["by_position"][2][key];
            assert_eq!(stats["count"], 0);
            for field in ["mean_cp", "std_cp", "mean_absolute_cp", "max_absolute_cp"] {
                assert!(stats[field].is_null());
            }
        }
        assert!(
            !fs::read_to_string(f.path("report.json"))
                .unwrap()
                .contains("probe_")
        );
    }
}

#[test]
fn moves_cover_captures_promotions_and_terminal_moves_using_library_games() {
    let f = Fixture::new();
    let positions = [
        common::position(
            &[
                (0, 0, Color::Black, PieceKind::King),
                (11, 11, Color::White, PieceKind::King),
                (2, 7, Color::Black, PieceKind::Pawn),
            ],
            Color::Black,
            None,
        ),
        common::position(
            &[
                (0, 0, Color::Black, PieceKind::King),
                (11, 11, Color::White, PieceKind::King),
                (11, 8, Color::Black, PieceKind::Rook),
            ],
            Color::Black,
            None,
        ),
    ];
    f.data("data.mnsd", &positions, 11, &[1; 2], "engine-default");
    f.baseline("base.bin", &pawn_linear(50.0), 2);
    f.network("net.bin", &tempo_model(InputFeatureSet::P));
    let report = f.success(&moves_args(), "report.json");
    let mut capture = false;
    let mut promotion = false;
    let mut terminal = false;
    for (i, position) in positions.iter().enumerate() {
        let game = Game::from_position(Rules::ENGINE_DEFAULT, position.clone());
        let mut deltas = Vec::new();
        let mut pst_deltas = Vec::new();
        let before = if i == 0 { 50.0 } else { 0.0 };
        for mv in game.legal_moves() {
            let mut after = game.clone();
            after.play(mv).unwrap();
            capture |= after.position().occupied().popcount() < 3;
            promotion |= mv.promote;
            terminal |= after.result().is_some();
            let n = f64::from(after.position().occupied().popcount());
            // 唯一の歩が成るとPSTは50から0へ変わる。FMは3駒で12、n駒で2n(n-1)。
            let placement = if i == 0 && mv.promote { -before } else { 0.0 };
            deltas.push(placement - 12.0 - 2.0 * n * (n - 1.0));
            pst_deltas.push(placement);
        }
        let row = &report["by_position"][i];
        assert_eq!(row["matched_count"], deltas.len());
        let mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let std =
            (deltas.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / deltas.len() as f64).sqrt();
        assert_close(&row["baseline"]["mean_cp"], mean, 1e-12);
        assert_close(&row["baseline"]["std_cp"], std, 1e-12);
        assert_close(
            &row["candidate_minus_baseline"]["mean_cp"],
            -254.0 - mean,
            1e-12,
        );
        assert_close(
            &row["candidate_minus_baseline_pst"]["mean_cp"],
            -254.0 - pst_deltas.iter().sum::<f64>() / pst_deltas.len() as f64,
            1e-12,
        );
    }
    assert!(capture && promotion && terminal);
}

#[test]
fn moves_reject_nondefault_rules_and_empty_input() {
    let f = Fixture::new();
    f.baseline("base.bin", &vec![[0.0; 2]; FEATURE_COUNT], 0);
    f.network("net.bin", &tempo_model(InputFeatureSet::P));
    f.data("data.mnsd", &[Position::initial()], 11, &[1], "L1,P0,R1,E0");
    assert!(String::from_utf8_lossy(&f.run(&moves_args()).stderr).contains("engine-default"));
    assert!(!f.path("report.json").exists());
    f.data("data.mnsd", &[], 11, &[], "engine-default");
    assert!(String::from_utf8_lossy(&f.run(&moves_args()).stderr).contains("nonempty"));
    assert!(!f.path("report.json").exists());
}

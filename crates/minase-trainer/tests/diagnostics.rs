//! 数値診断の出力契約を手計算できる合成入力で検査し、代表事例で終了コードと標準エラーも確かめる。

use common::mnsd;
use minase_trainer::data::{Outcome, Sample};
mod common;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use clap::Parser;
use minase::{
    eval::{
        features::{FEATURE_COUNT, PIECE_STATE_COUNT},
        pst::{FM_RANK, Pst},
    },
    training::records::Header,
};
use minase_core::{Color, Game, PieceCode, PieceKind, Position, PositionBuilder, Rules, Square};
use minase_trainer::diagnostics;
use minase_trainer::error::ContractError;
use minase_trainer::nnue::layerstack::{
    self, AdamW, Checkpoint, H, OptimizerConfig, Parameter, Parameters, ROWS, Scale, Settings,
};
use minase_trainer::pst::v0_piece_values;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    command: diagnostics::Command,
}

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
                Sample::from_position(
                    position,
                    0,
                    Outcome::Draw,
                    game,
                    i as u16,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .unwrap()
            })
            .collect();
        self.records(name, &records, seed, rules)
    }

    fn records(&self, name: &str, records: &[Sample], seed: u64, rules: &str) -> PathBuf {
        let path = self.path(name);
        let header = Header::new(
            rules.into(),
            "1".repeat(40),
            [2; 32],
            minase::training::records::TeacherKind::AlphaBeta,
            200,
            seed,
        )
        .unwrap();
        let mut samples = Vec::new();
        for record in records {
            samples.push(record.clone());
        }
        fs::write(&path, mnsd::independent(header, &samples)).unwrap();
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
            "JCSA",
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

    fn network(&self, name: &str, model: &TestModel) -> PathBuf {
        let path = self.path(name);
        fs::write(
            &path,
            layerstack::export(&model.parameters, model.scale).unwrap(),
        )
        .unwrap();
        path
    }

    fn checkpoint(&self, model: TestModel) -> PathBuf {
        let path = self.path("best.npz");
        let config = OptimizerConfig::default();
        Checkpoint {
            progress: progress(),
            optimizer: AdamW::new(config).unwrap(),
            parameters: model.parameters,
            settings: Settings {
                scale: model.scale,
                rules: Rules::JCSA,
                lambda: 1.0,
                seed: 1,
                optimizer: config,
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

    fn diagnose(&self, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        // 相対パスをこのfixtureへ解決する。並列テストの作業ディレクトリは変更しない。
        let mut values: Vec<_> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let mut index = 1;
        while index < values.len() {
            match values[index].as_str() {
                "--data" => {
                    index += 1;
                    while index < values.len() && !values[index].starts_with("--") {
                        values[index] = self.path(&values[index]).to_str().unwrap().to_owned();
                        index += 1;
                    }
                }
                "--pair" => {
                    for offset in [2, 3] {
                        values[index + offset] = self
                            .path(&values[index + offset])
                            .to_str()
                            .unwrap()
                            .to_owned();
                    }
                    index += 4;
                }
                "--float" | "--net" | "--base" | "--positions" | "--output" => {
                    values[index + 1] = self.path(&values[index + 1]).to_str().unwrap().to_owned();
                    index += 2;
                }
                "--sample" | "--seed" => index += 2,
                flag => panic!("unexpected diagnostic argument: {flag}"),
            }
        }
        let parsed = Arguments::try_parse_from(std::iter::once("diag".to_owned()).chain(values))
            .unwrap_or_else(|error| panic!("{args:?}: {error}"));
        diagnostics::run(parsed.command)
    }

    fn success(&self, args: &[&str], name: &str) -> Value {
        self.diagnose(args).unwrap();
        serde_json::from_slice(&fs::read(self.path(name)).unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

struct TestModel {
    parameters: Parameters,
    scale: Scale,
}
fn model(linear: &[[f32; 2]], teacher: f64) -> TestModel {
    let scale = Scale::new(teacher).unwrap();
    let mut parameters = Parameters::initialize(1, scale);
    for p in Parameter::ALL {
        parameters[p].fill(0.0);
    }
    // 試験のPSQTは両端点が等しい駒升評価をK_oで割って構築する。
    for (f, values) in linear.iter().enumerate() {
        assert_eq!(values[0], values[1]);
        let row = f / 144 * 72 + f % 144 / 12 * 6 + (f % 12).min(11 - f % 12);
        parameters[Parameter::Psqt][row] = (f64::from(values[0]) / scale.output()) as f32;
    }
    TestModel { parameters, scale }
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
    json_root_keys(&fs::read_to_string(fixture.path(name)).unwrap())
}

fn json_root_keys(json: &str) -> Vec<String> {
    use serde::Deserializer;
    use serde::de::{IgnoredAny, MapAccess, Visitor};
    struct Keys;
    impl<'de> Visitor<'de> for Keys {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut keys = Vec::new();
            while let Some(key) = map.next_key::<String>()? {
                keys.push(key);
                map.next_value::<IgnoredAny>()?;
            }
            Ok(keys)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let keys = deserializer.deserialize_map(Keys).unwrap();
    deserializer.end().unwrap();
    keys
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
    // 先獅子を空升に置き、通常の局面復元で同じ数値診断を行う。
    for (name, seed, game) in [("a.mnsd", 11, 4), ("b.mnsd", 22, 6)] {
        let records: Vec<_> = positions
            .iter()
            .map(|position| {
                let mut position = position.clone();
                position.set_lion_capture(Square::from_dense(100)).unwrap();
                Sample::from_position(
                    &position,
                    0,
                    Outcome::Draw,
                    game,
                    0,
                    minase::training::records::Origin::RandomSelfplay,
                )
                .unwrap()
            })
            .collect();
        f.records(name, &records, seed, "JCSA");
    }
    f.baseline("base.bin", &pawn_linear(50.0), 0);
    for name in ["p.bin", "second.bin"] {
        f.network(name, &model(&pawn_linear(100.0), 1016.0));
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
        "second",
        "base.bin",
        "second.bin",
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
    assert_eq!(report["pairs"][0]["name"], "second");
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
    assert!(f.diagnose(&args).is_err());
    assert_eq!(fs::read(f.path("report.json")).unwrap(), saved);
}

#[test]
fn distribution_sampling_is_deterministic_and_invalid_inputs_create_no_report() {
    let f = Fixture::new();
    let positions = distribution_positions();
    f.data("data.mnsd", &positions, 11, &[4; 24], "JCSA");
    f.baseline("base.bin", &pawn_linear(50.0), 0);
    f.network("net.bin", &model(&pawn_linear(50.0), 1016.0));
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
    assert_eq!(
        first["pairs"][0]["verdict"],
        "全体の尺度はほとんど変えていない"
    );
    fs::remove_file(f.path("report.json")).unwrap();
    let mut duplicate = args.to_vec();
    duplicate.extend(["--pair", "same", "base.bin", "net.bin"]);
    assert!(f.diagnose(&duplicate).is_err());
    assert!(!f.path("report.json").exists());
    // 空の帯や分散0は報告を作成せずに拒否する。
    f.network(
        "constant.bin",
        &model(&vec![[0.0; 2]; FEATURE_COUNT], 1016.0),
    );
    let mut constant = args;
    constant[10] = "constant.bin";
    assert!(f.diagnose(&constant).is_err());
    assert!(!f.path("report.json").exists());
}

fn removal_args(sample: &str) -> Vec<&str> {
    vec![
        "nnue-removal",
        "--float",
        "best.npz",
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
    let record = Sample::from_position(
        &position,
        0,
        Outcome::Draw,
        4,
        0,
        minase::training::records::Origin::RandomSelfplay,
    )
    .unwrap();
    assert!(record.lion_by_kirin_promotion());
    assert_eq!(record.to_position().unwrap(), position);
    f.records("data.mnsd", &[record], 11, "JCSA");
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
    let m = model(&linear, 1016.0);
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
        // PSQTのf32表現による誤差は最大駒価値のf32相対精度以内。
        2500.0 * f64::from(f32::EPSILON),
    );
    let text = fs::read_to_string(f.path("report.json")).unwrap();
    assert_eq!(json_nested_keys(&text, "by_piece_state"), ["20", "29"]);
}

#[test]
fn removal_samples_without_replacement_using_the_existing_splitmix64_sequence() {
    let f = Fixture::new();
    let pawn = common::position(&[(2, 4, Color::Black, PieceKind::Pawn)], Color::Black, None);
    f.data("data.mnsd", &vec![pawn; 12], 11, &[4; 12], "JCSA");
    let m = model(&pawn_linear(100.0), 1016.0);
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

fn tempo_model() -> TestModel {
    let mut m = model(&vec![[0.0; 2]; FEATURE_COUNT], 1016.0);
    m.parameters[Parameter::L3Bias][0] = 0.125;
    m
}

#[test]
fn moves_use_negamax_and_include_fm_while_empty_positions_have_null_statistics() {
    let f = Fixture::new();
    let kings = [
        (0, 0, Color::Black, PieceKind::King),
        (11, 11, Color::White, PieceKind::King),
        (2, 2, Color::Black, PieceKind::Pawn),
        (9, 9, Color::White, PieceKind::Pawn),
    ];
    let positions = [
        common::position(&kings, Color::Black, None),
        common::position(&kings, Color::White, None),
        Position::empty(Color::Black),
    ];
    f.data("data.mnsd", &positions, 11, &[1; 3], "L1,L2,P0,P3,R1,E1,E3");
    f.baseline("base.bin", &vec![[0.0; 2]; FEATURE_COUNT], 2);
    f.network("net.bin", &tempo_model());
    let report = f.success(&moves_args(), "report.json");
    assert_eq!(
        root_keys(&f, "report.json"),
        [
            "net",
            "base",
            "positions",
            "position_count",
            "terminal_count",
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
        assert_eq!(row["matched_count"], 4);
        // NNUEは両視点で127 cp、FMは両視点で24 cpなので、それぞれ-254と-48。
        for (key, expected) in [
            ("candidate", -254.0),
            ("baseline", -48.0),
            ("candidate_minus_baseline", -206.0),
            ("candidate_minus_baseline_pst", -254.0),
        ] {
            assert_eq!(row[key]["count"], 4);
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
    fn assert_no_probe_keys(value: &Value) {
        match value {
            Value::Object(fields) => {
                for (key, value) in fields {
                    assert!(!key.starts_with("probe_"), "unexpected key: {key}");
                    assert_no_probe_keys(value);
                }
            }
            Value::Array(values) => values.iter().for_each(assert_no_probe_keys),
            _ => {}
        }
    }
    assert_no_probe_keys(&report);
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
    f.data("data.mnsd", &positions, 11, &[1; 2], "lishogi");
    f.baseline("base.bin", &pawn_linear(50.0), 2);
    f.network("net.bin", &tempo_model());
    let report = f.success(&moves_args(), "report.json");
    let mut capture = false;
    let mut promotion = false;
    let mut terminal = false;
    let mut total_terminal = 0;
    for (i, position) in positions.iter().enumerate() {
        let game = Game::from_position(Rules::LISHOGI, position.clone());
        let mut deltas = Vec::new();
        let mut pst_deltas = Vec::new();
        let before = if i == 0 { 50.0 } else { 0.0 };
        let mut terminal_count = 0;
        for mv in game.legal_moves() {
            let mut after = game.clone();
            after.play(mv).unwrap();
            capture |= after.position().occupied().popcount() < 3;
            promotion |= mv.promote;
            terminal |= after.result().is_some();
            if after.result().is_some() {
                terminal_count += 1;
                continue;
            }
            let n = f64::from(after.position().occupied().popcount());
            // 唯一の歩が成るとPSTは50から0へ変わる。FMは3駒で12、n駒で2n(n-1)。
            let placement = if i == 0 && mv.promote { -before } else { 0.0 };
            deltas.push(placement - 12.0 - 2.0 * n * (n - 1.0));
            pst_deltas.push(placement);
        }
        let row = &report["by_position"][i];
        assert_eq!(row["terminal_count"], terminal_count);
        total_terminal += terminal_count;
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
    assert_eq!(report["terminal_count"], total_terminal);
}

#[test]
fn moves_reject_nondefault_rules_and_empty_input() {
    let f = Fixture::new();
    f.baseline("base.bin", &vec![[0.0; 2]; FEATURE_COUNT], 0);
    f.network("net.bin", &tempo_model());
    f.data("data.mnsd", &[Position::initial()], 11, &[1], "JCSA");
    let error = f.diagnose(&moves_args()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ContractError>(),
        Some(&ContractError::NondefaultRules)
    );
    assert!(!f.path("report.json").exists());
    f.data("data.mnsd", &[], 11, &[], "lishogi");
    let error = f.diagnose(&moves_args()).unwrap_err();
    assert_eq!(
        error.downcast_ref::<ContractError>(),
        Some(&ContractError::EmptyPositions)
    );
    assert!(!f.path("report.json").exists());
}

fn json_nested_keys(json: &str, field: &str) -> Vec<String> {
    use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
    struct Keys<'a>(Option<&'a str>);
    impl<'de> DeserializeSeed<'de> for Keys<'_> {
        type Value = Vec<String>;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_map(self)
        }
    }
    impl<'de> Visitor<'de> for Keys<'_> {
        type Value = Vec<String>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut keys = Vec::new();
            while let Some(key) = map.next_key::<String>()? {
                match self.0 {
                    Some(field) if key == field => keys = map.next_value_seed(Keys(None))?,
                    Some(_) => {
                        map.next_value::<IgnoredAny>()?;
                    }
                    None => {
                        keys.push(key);
                        map.next_value::<IgnoredAny>()?;
                    }
                }
            }
            Ok(keys)
        }
    }
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let keys = Keys(Some(field)).deserialize(&mut deserializer).unwrap();
    deserializer.end().unwrap();
    keys
}

// 全特徴の第1層を同じ値にし、後段の全ての枝を手計算できるようにする。
fn nonlinear_model() -> TestModel {
    // K_t=1000からm=8、K_o=1016を求める。教師尺度を直接掛ける誤りも検出する。
    let mut m = model(&vec![[0.0; 2]; FEATURE_COUNT], 1000.0);
    let w = &mut m.parameters;
    w[Parameter::FtWeights][..(ROWS - 1) * H].fill(1.0 / 1024.0);
    w[Parameter::FtBiases].fill(0.125);
    for row in 0..94 * 72 {
        if ![11, 21].contains(&(row / 72 % 47)) {
            w[Parameter::Psqt][row] = if row < 47 * 72 { 0.03125 } else { -0.03125 };
        }
    }
    w[Parameter::L1Weights][..H].fill(1.0 / 256.0);
    w[Parameter::L1SharedWeights][..H].fill(1.0 / 512.0);
    w[Parameter::L1Biases][0] = 0.0625;
    w[Parameter::L1SharedBiases][0] = 0.0625;
    w[Parameter::L1Weights][15 * H..15 * H + H / 2].fill(1.0 / 512.0);
    w[Parameter::L1Biases][15] = 0.0625;
    w[Parameter::L2Weights][0] = 0.5;
    w[Parameter::L2Weights][15] = 0.25;
    w[Parameter::L2Biases][0] = 0.125;
    w[Parameter::L3Weights][0] = 0.5;
    w[Parameter::L3Bias][0] = 0.0625;
    m
}

// この試験ネットの式を、配列と順伝播の実装から独立したスカラー式で計算する。
fn nonlinear_output(features: usize) -> f64 {
    let alpha = 127.0 / 128.0;
    let a = (0.125 + features as f64 / 1024.0).clamp(0.0, 1.0);
    let p = alpha * a * a;
    let y = 0.125 + 1.5 * p;
    let u =
        (0.125 + 0.5 * (alpha * y * y).clamp(0.0, 1.0) + 0.25 * y.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    0.0625 + 0.0625 + 0.25 * p + 0.5 * u
}

#[test]
fn removal_matches_layerstack_formula_and_engine_deltas_in_both_views() {
    use minase::eval::nnue::network::Network;
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
            // 先獅子は盤上駒と別の入力行として残る。
            position.set_lion_capture(Square::from_dense(140)).unwrap();
            positions.push(position);
        }
    }
    f.data("data.mnsd", &positions, 11, &[4; 12], "JCSA");
    let m = nonlinear_model();
    let net = Network::decode(&fs::read(f.network("net.bin", &m)).unwrap()).unwrap();
    f.checkpoint(m);
    let report = f.success(&removal_args("12"), "report.json");
    let mut errors = Vec::new();
    let mut reversals = Vec::new();
    for position in &positions {
        let side = position.side_to_move();
        let count = position.occupied().popcount() as usize;
        let before = net.evaluate_for(position, side).unwrap();
        for square in Square::all() {
            let Some(piece) = position.piece_at(square) else {
                continue;
            };
            let sign = if piece.color() == Some(side) {
                1.0
            } else {
                -1.0
            };
            let expected =
                1016.0 * (nonlinear_output(count) - nonlinear_output(count + 1) - sign * 0.03125);
            let mut builder = PositionBuilder::new(side);
            for s in Square::all().filter(|&s| s != square) {
                if let Some(p) = position.piece_at(s) {
                    builder.put(s, p).unwrap();
                }
            }
            let mut after = builder.finish().unwrap();
            after
                .set_lion_capture(position.lion_capture_square())
                .unwrap();
            let delta = net.evaluate_for(&after, side).unwrap() - before;
            errors.push((f64::from(delta) - expected).abs());
            if f64::from(delta) * -sign < 0.0 {
                reversals.push(f64::from(delta).abs());
            }
        }
    }
    assert_eq!(report["removal_count"], 564);
    assert_eq!(report["reversals"]["total"], 564);
    assert_eq!(report["reversals"]["count"], reversals.len());
    assert_eq!(report["float_batch_vs_recomputed"]["passed"], true);
    assert_close(
        &report["float_vs_integer_recomputed"]["mean_absolute_cp"],
        errors.iter().sum::<f64>() / errors.len() as f64,
        1e-10,
    );
    assert_close(
        &report["float_vs_integer_recomputed"]["max_absolute_cp"],
        errors.iter().copied().fold(0.0, f64::max),
        1e-10,
    );
    assert!(report["by_piece_state"].as_object().unwrap().len() >= 2);
}

#[test]
fn removal_rejects_scale_mismatch_and_empty_removals_without_report() {
    let f = Fixture::new();
    f.data("data.mnsd", &[Position::initial()], 11, &[4], "JCSA");
    f.checkpoint(model(&pawn_linear(100.0), 679.03));
    f.network("net.bin", &model(&pawn_linear(100.0), 1016.0));
    let error = f.diagnose(&removal_args("1")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "LayerStack fv_scale mismatch: checkpoint=12, network=8"
    );
    assert!(!f.path("report.json").exists());
    f.checkpoint(model(&pawn_linear(100.0), 1016.0));
    let mut narrow =
        layerstack::quantize(&Parameters::zeros(), Scale::new(1016.0).unwrap()).unwrap();
    narrow.width = 2;
    narrow.ft_biases = vec![0; 2];
    narrow.ft_weights = vec![0; FEATURE_COUNT * 2];
    narrow.l1_weights = vec![0; 16 * 2];
    let narrow = minase::eval::nnue::network::LayerStackNetwork::new(narrow).unwrap();
    fs::write(f.path("net.bin"), narrow.encode()).unwrap();
    assert_eq!(
        f.diagnose(&removal_args("1")).unwrap_err().to_string(),
        "LayerStack width mismatch: checkpoint=256, network=2"
    );
    assert!(!f.path("report.json").exists());
    f.network("net.bin", &model(&pawn_linear(100.0), 1016.0));
    f.data(
        "data.mnsd",
        &[common::position(
            &[(0, 0, Color::Black, PieceKind::King)],
            Color::Black,
            None,
        )],
        11,
        &[4],
        "JCSA",
    );
    assert_eq!(
        f.diagnose(&removal_args("1")).unwrap_err().to_string(),
        "sample contains no non-royal removals"
    );
    assert!(!f.path("report.json").exists());
}

#[test]
fn failed_removal_is_saved_before_the_command_returns_exit_code_one() {
    let f = Fixture::new();
    let position = common::position(
        &[
            (0, 0, Color::Black, PieceKind::Pawn),
            (1, 0, Color::Black, PieceKind::SilverGeneral),
        ],
        Color::Black,
        None,
    );
    f.data("data.mnsd", &[position], 11, &[4], "JCSA");
    let mut m = nonlinear_model();
    // 巨大な行を引くと小さな行とバイアスが失われ、局面からの再計算との差が残る。
    m.parameters[Parameter::FtWeights][29 * 72 * H..(29 * 72 + 1) * H].fill(1e20);
    f.checkpoint(m);
    f.network("net.bin", &model(&vec![[0.0; 2]; FEATURE_COUNT], 1016.0));
    let mut args = removal_args("1");
    *args.last_mut().unwrap() = "nested/report.json";
    let output = f.run(&args);
    assert_eq!(output.status.code(), Some(1));
    let saved = fs::read(f.path("nested/report.json")).unwrap();
    let report: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(report["float_batch_vs_recomputed"]["passed"], false);
    assert!(
        report["float_batch_vs_recomputed"]["max_absolute_cp"]
            .as_f64()
            .unwrap()
            > 1e-6
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains(report["error"].as_str().unwrap()));
    assert!(f.diagnose(&args).is_err());
    assert_eq!(fs::read(f.path("nested/report.json")).unwrap(), saved);
}

#[test]
fn moves_record_all_terminal_moves_without_delta_statistics() {
    let f = Fixture::new();
    let position = common::position(
        &[
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
        ],
        Color::Black,
        None,
    );
    f.data("data.mnsd", &[position], 11, &[4], "lishogi");
    f.baseline("base.bin", &pawn_linear(50.0), 0);
    f.network("net.bin", &tempo_model());
    let report = f.success(&moves_args(), "report.json");
    // lishogiの王駒だけの裁定は、これら3手をすべて終局にする。
    assert_eq!(report["terminal_count"], 3);
    let row = &report["by_position"][0];
    assert_eq!(row["terminal_count"], 3);
    assert_eq!(row["matched_count"], 0);
    assert_eq!(row["candidate"]["count"], 0);
    assert!(row["candidate"]["mean_cp"].is_null());
}

#[test]
fn removals_after_captures_and_promotions_recompute_mirrored_features() {
    let f = Fixture::new();
    let initial = common::position(
        &[
            (0, 0, Color::Black, PieceKind::King),
            (11, 11, Color::White, PieceKind::King),
            (2, 7, Color::Black, PieceKind::Pawn),
            (5, 5, Color::Black, PieceKind::Rook),
            (5, 6, Color::White, PieceKind::SilverGeneral),
            (9, 9, Color::White, PieceKind::Pawn),
        ],
        Color::Black,
        None,
    );
    let game = Game::from_position(Rules::LISHOGI, initial.clone());
    let mut positions = vec![initial];
    let (mut captured, mut promoted) = (false, false);
    for mv in game.legal_moves() {
        let mut after = game.clone();
        after.play(mv).unwrap();
        if after.result().is_some() {
            continue;
        }
        let capture = after.position().occupied().popcount() < 6;
        if capture || mv.promote {
            positions.push(after.position().clone());
            captured |= capture;
            promoted |= mv.promote;
        }
    }
    assert!(captured && promoted);
    let scale = Scale::new(679.03).unwrap();
    let mut parameters = Parameters::initialize(91, scale);
    parameters[Parameter::FtBiases].fill(0.25);
    parameters[Parameter::L1Biases].fill(0.25);
    parameters[Parameter::L2Biases].fill(0.25);
    let model = TestModel { parameters, scale };
    f.network("net.bin", &model);
    f.checkpoint(model);
    f.data(
        "data.mnsd",
        &positions,
        11,
        &vec![4; positions.len()],
        "JCSA",
    );
    let count = positions.len().to_string();
    let report = f.success(&removal_args(&count), "report.json");
    assert_eq!(report["float_batch_vs_recomputed"]["passed"], true);
    assert_eq!(
        report["removal_count"],
        positions
            .iter()
            .map(|p| p.occupied().popcount() - 2)
            .sum::<u32>()
    );
    assert!(
        report["float_vs_integer_recomputed"]["mean_absolute_cp"]
            .as_f64()
            .unwrap()
            .is_finite()
    );
}

#[test]
fn removal_reference_excludes_deltas_clipped_to_zero() {
    let f = Fixture::new();
    let positions: Vec<_> = Color::ALL
        .into_iter()
        .map(|side| {
            let mut builder = PositionBuilder::new(side);
            for i in 0..13 {
                builder
                    .put(
                        Square::from_dense(i).unwrap(),
                        PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
                    )
                    .unwrap();
            }
            builder.finish().unwrap()
        })
        .collect();
    f.data("data.mnsd", &positions, 11, &[4; 2], "JCSA");
    let model = nonlinear_model();
    f.network("net.bin", &model);
    f.checkpoint(model);
    let report = f.success(&removal_args("2"), "report.json");
    // v0の獅子13枚は32,500 cp、除去後も30,000 cpなので、両端が±28,999に切り詰められる。
    assert_eq!(report["removal_count"], 26);
    assert_eq!(report["reversals"]["total"], 0);
    assert!(report["reversals"]["fraction"].is_null());
    assert_eq!(report["by_piece_state"]["20"]["total"], 0);
}

fn progress() -> minase_trainer::nnue::train::Progress {
    minase_trainer::nnue::train::Progress {
        completed_superbatches: 0,
        completed_steps: 0,
        elapsed_seconds: 0.0,
        config: minase_trainer::nn::checkpoint::Configuration {
            settings: serde_json::json!({}),
            inputs: serde_json::json!({}),
            preprocessing: serde_json::json!({}),
        },
    }
}

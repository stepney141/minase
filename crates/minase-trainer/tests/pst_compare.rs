//! PST比較診断の出力契約と明示された失敗条件を、合成MNSDで検査する。
use minase::{
    eval::{features::FEATURE_COUNT, pst::Pst},
    training::records::{Header, Outcome, Record},
};
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};
use minase_trainer::diagnostics::{self, Command, PstCompareArgs};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

fn piece_values() -> [i32; 47] {
    let mut values = [100; 47];
    values[11] = 200;
    values[21] = 200;
    values
}

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    data: Vec<PathBuf>,
    weights: Vec<[i16; 2]>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "pst-compare-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let mut weights = vec![[0; 2]; FEATURE_COUNT];
        for (feature, row) in weights[..94 * 144].iter_mut().enumerate() {
            let state = feature / 144 % 47;
            let sign = if feature < 47 * 144 { 1 } else { -1 };
            *row = [
                sign * (state as i16 + 10) * 8,
                sign * (state as i16 + 20) * 8,
            ];
        }
        let model = Pst::from_weights(&weights, piece_values(), 100.0, "L0,P0,R1,E0").unwrap();
        fs::write(root.join("base.bin"), model.encode()).unwrap();
        fs::write(root.join("candidate.bin"), model.encode()).unwrap();
        let floating: Vec<_> = weights
            .iter()
            .map(|w| w.map(|v| f32::from(v) / 8.0))
            .collect();
        fs::write(
            root.join("float.npz"),
            minase_trainer::pst::encode_pst_npz(&floating).unwrap(),
        )
        .unwrap();
        let mut fixture = Self {
            root,
            data: Vec::new(),
            weights,
        };
        fixture.input(11, 1, 0.75, None);
        fixture.input(22, 2, 0.0, None);
        fixture
    }
    fn input(&mut self, seed: u64, class: u8, mix: f64, human_id: Option<&str>) {
        let path = self.root.join(format!("input-{seed}.bin"));
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            format!("{class:040x}"),
            [class; 32],
            100,
            seed,
            6,
        )
        .unwrap();
        let mut bytes = header.encode().to_vec();
        let validation_game = if seed == 11 { 4 } else { 6 };
        for (i, (game, position, outcome)) in [
            (1, sparse(), Outcome::Win),
            (1, sparse(), Outcome::Loss),
            (1, sparse(), Outcome::Win),
            (validation_game, sparse(), Outcome::Win),
            (validation_game, sparse(), Outcome::Draw),
            (validation_game, Position::initial(), Outcome::Loss),
        ]
        .into_iter()
        .enumerate()
        {
            bytes.extend(
                Record::from_position(
                    &position,
                    [100, -200, 300, 400, -500, 600][i],
                    outcome,
                    game,
                    i as u16,
                )
                .encode(),
            );
        }
        fs::write(&path, &bytes).unwrap();
        let games = human_id.map(|id| json!([{"game":1,"id":"training-id","ply":0}, {"game":validation_game,"id":id,"ply":0}]));
        let metadata = json!({
            "format":"minase-provenance", "version":1,
            "mnsd_sha256":format!("{:x}",Sha256::digest(&bytes)),
            "teacher":{"generation_commit":format!("{class:040x}"), "network_checksum":format!("{class:02x}").repeat(32), "nodes":100, "rule_set":"L0,P0,R1,E0", "search_condition":"in-game"},
            "result_origin":if human_id.is_some() {"human"} else {"selfplay"},
            "start_origin":if human_id.is_some() {"human-game"} else {"random"},
            "lambda":mix, "games":games,
        });
        fs::write(
            path.with_extension("bin.provenance.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        self.data.push(path);
    }
    fn args(&self, name: &str) -> PstCompareArgs {
        PstCompareArgs {
            data: self.data.clone(),
            lambda_override: None,
            lookahead_gamma: Some(0.9),
            lookahead_plies: Some(40),
            base: self.root.join("base.bin"),
            candidate: self.root.join("candidate.bin"),
            candidate_float: self.root.join("float.npz"),
            output_dir: self.root.join(name),
            sample_size: 10_000,
            seed: 1,
        }
    }
    fn run(&self, name: &str) -> Value {
        diagnostics::run(Command::PstCompare(self.args(name))).unwrap();
        read_report(&self.root.join(name))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn sparse() -> Position {
    let mut builder = PositionBuilder::new(Color::Black);
    for (file, rank, color, kind) in [
        (0, 0, Color::Black, PieceKind::King),
        (11, 11, Color::White, PieceKind::King),
        (4, 7, Color::Black, PieceKind::Pawn),
        (7, 10, Color::White, PieceKind::Pawn),
    ] {
        builder
            .put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, kind).unwrap(),
            )
            .unwrap();
    }
    builder.finish().unwrap()
}
fn read_report(directory: &Path) -> Value {
    serde_json::from_slice(&fs::read(directory.join("report.json")).unwrap()).unwrap()
}
#[test]
fn synthetic_mnsd_preserves_bands_artifacts_promotions_and_deterministic_output() {
    let fixture = Fixture::new();
    let report = fixture.run("first");
    assert_eq!(report["bands"].as_array().unwrap().len(), 10);
    assert_eq!(report["rust_agreement"], json!({"base":6,"candidate":6}));
    assert_eq!(report["teacher_comparison_excluded"], 3);
    assert_eq!(report["teacher_ks"][1], Value::Null);
    assert_eq!(report["bands"][0]["samples"], 2);
    assert_eq!(report["bands"][1]["reason"], "empty band");
    assert!(report["bands"][1]["validation_loss"]["base"].is_null());
    assert_eq!(
        report["bands"][5]["teacher_comparison_reason"],
        "lambda is zero"
    );
    assert!(report["bands"][5]["base"].is_null());
    assert!(report["rust_promotion_agreement"]["base"].as_u64().unwrap() > 0);
    for representative in report["representatives"].as_array().unwrap() {
        if let Some(moves) = representative["promotions"]["base"].as_array() {
            for mv in moves {
                assert_eq!(
                    mv["delta"].as_i64().unwrap(),
                    mv["placement"].as_i64().unwrap() + mv["turn"].as_i64().unwrap()
                );
                assert_eq!(
                    mv["delta"].as_i64().unwrap(),
                    -mv["after"]["eval"].as_i64().unwrap()
                        - representative["evaluations"]["base"].as_i64().unwrap()
                );
            }
        }
    }
    let first = fixture.root.join("first");
    let bytes = fs::read(first.join("diagnostic-samples.bin")).unwrap();
    let header = Header::decode(&bytes[..136]).unwrap();
    assert_eq!(header.rule_set(), "L0,P0,R1,E0");
    assert_eq!(header.record_count(), 6);
    let expected: Vec<_> = fixture
        .data
        .iter()
        .flat_map(|path| fs::read(path).unwrap()[136 + 3 * 160..].to_vec())
        .collect();
    assert_eq!(&bytes[136..], &expected);
    let initial = fs::read(first.join("representatives.bin")).unwrap();
    assert_eq!(&initial[136 + 146..136 + 160], &[0; 14]);
    assert_eq!(report["representatives"][1]["index"], 3);
    assert_eq!(report["representatives"][2]["index"], 5);
    diagnostics::run(Command::PstCompare(fixture.args("repeated"))).unwrap();
    for entry in fs::read_dir(&first).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            fs::read(entry.path()).unwrap(),
            fs::read(fixture.root.join("repeated").join(entry.file_name())).unwrap()
        );
    }
    // 新規ディレクトリの契約。既存成果物を上書きしない。
    assert!(diagnostics::run(Command::PstCompare(fixture.args("first"))).is_err());
}

#[test]
fn quantization_failure_writes_report_before_exit_code_one() {
    let fixture = Fixture::new();
    let wrong = vec![[500.0; 2]; FEATURE_COUNT];
    fs::write(
        fixture.root.join("float.npz"),
        minase_trainer::pst::encode_pst_npz(&wrong).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
        .args(["diag", "pst-compare", "--data"])
        .args(&fixture.data)
        .arg("--base")
        .arg(fixture.root.join("base.bin"))
        .arg("--candidate")
        .arg(fixture.root.join("candidate.bin"))
        .arg("--candidate-float")
        .arg(fixture.root.join("float.npz"))
        .arg("--output-dir")
        .arg(fixture.root.join("failed"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report = read_report(&fixture.root.join("failed"));
    assert!(
        report["quantization"]["mean_absolute_error_cp"]
            .as_f64()
            .unwrap()
            > 2.0
    );
    assert!(report["error"].as_str().unwrap().contains("quantization"));
    assert!(fixture.root.join("failed/diagnostic-samples.bin").exists());
}

#[test]
fn removal_sign_reversal_is_a_saved_failure() {
    let fixture = Fixture::new();
    let reversed: Vec<_> = fixture.weights.iter().map(|w| w.map(|v| -v)).collect();
    let model = Pst::from_weights(&reversed, piece_values(), 100.0, "L0,P0,R1,E0").unwrap();
    fs::write(fixture.root.join("candidate.bin"), model.encode()).unwrap();
    let float: Vec<_> = reversed
        .iter()
        .map(|w| w.map(|v| f32::from(v) / 8.0))
        .collect();
    fs::write(
        fixture.root.join("float.npz"),
        minase_trainer::pst::encode_pst_npz(&float).unwrap(),
    )
    .unwrap();
    assert!(diagnostics::run(Command::PstCompare(fixture.args("reversal"))).is_err());
    let report = read_report(&fixture.root.join("reversal"));
    assert!(report["pst_removal_sign_reversals"].as_u64().unwrap() > 0);
    assert!(fixture.root.join("reversal/removals.bin").exists());
}

#[test]
fn human_game_ids_are_shared_across_files_and_random_seeds_are_distinct() {
    let mut fixture = Fixture::new();
    let random = fixture.run("random");
    assert_eq!(random["outcome_metrics"]["base"]["games"], 2);
    // 検証集合への所属はIDのSHA-256先頭8バイトの20での剰余で定義される。
    let id = (0..1000)
        .map(|i| format!("human-{i}"))
        .find(|id| {
            let hash = Sha256::digest(id.as_bytes());
            u64::from_le_bytes(hash[..8].try_into().unwrap()).is_multiple_of(20)
        })
        .unwrap();
    let hash = Sha256::digest(b"training-id");
    assert!(!u64::from_le_bytes(hash[..8].try_into().unwrap()).is_multiple_of(20));
    fixture.data.clear();
    fixture.input(33, 3, 0.0, Some(&id));
    fixture.input(44, 4, 0.0, Some(&id));
    let human = fixture.run("human");
    assert_eq!(human["outcome_metrics"]["base"]["games"], 1);
    assert_eq!(human["outcome_metrics"]["base"]["records"], 6);
}

#[test]
fn seeded_sampling_is_without_replacement_and_applies_the_cap_per_band() {
    let fixture = Fixture::new();
    for name in ["small1", "small2"] {
        let mut args = fixture.args(name);
        args.sample_size = 1;
        diagnostics::run(Command::PstCompare(args)).unwrap();
    }
    assert_eq!(
        fs::read(fixture.root.join("small1/report.json")).unwrap(),
        fs::read(fixture.root.join("small2/report.json")).unwrap()
    );
    let report = read_report(&fixture.root.join("small1"));
    assert_eq!(report["quantization"]["samples"], 4);
    for band in report["bands"].as_array().unwrap() {
        assert!(band["samples"].as_u64().unwrap() <= 1);
    }
}

#[test]
fn removing_the_promoted_kirin_trigger_keeps_its_restore_failure() {
    let fixture = Fixture::new();
    let mut builder = PositionBuilder::new(Color::Black);
    for (file, rank, color) in [(0, 0, Color::Black), (11, 11, Color::White)] {
        builder
            .put(
                Square::new(file, rank).unwrap(),
                PieceCode::new(color, PieceKind::King).unwrap(),
            )
            .unwrap();
    }
    let square = Square::new(7, 7).unwrap();
    builder
        .put(
            square,
            PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
        )
        .unwrap();
    let mut position = builder.finish().unwrap();
    position.set_lion_capture(Some(square)).unwrap();
    let path = &fixture.data[0];
    let mut bytes = fs::read(path).unwrap();
    let record = Record::from_position(&position, 400, Outcome::Win, 4, 3);
    bytes[136 + 3 * 160..136 + 4 * 160].copy_from_slice(&record.encode());
    fs::write(path, &bytes).unwrap();
    let sidecar = path.with_extension("bin.provenance.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
    metadata["mnsd_sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    fs::write(sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
    let report = fixture.run("skipped");
    assert_eq!(report["rust_removal_agreement"]["base"]["skipped"], 1);
    let removal = &report["representatives"][1]["removals"][0];
    assert_eq!(removal["square"], square.dense_index());
    assert!(removal["total_delta_cp"]["base"].is_null());
    assert!(removal["delta_cp"]["base"].is_i64());
    let mut removed = record.encode();
    removed[square.dense_index()] = 0;
    let reason = Record::decode(&removed)
        .unwrap()
        .to_position()
        .unwrap_err()
        .to_string();
    assert_eq!(removal["skipped"]["base"], reason);
    let removals = fs::read(fixture.root.join("skipped/removals.bin")).unwrap();
    assert!(removals[136..].chunks(160).any(|bytes| bytes == removed));
}

#[test]
fn mnpt_v3_base_is_rejected_before_data_loading_without_creating_reports() {
    assert_mnpt_v3_is_rejected("base");
}

#[test]
fn mnpt_v3_candidate_is_rejected_before_data_loading_without_creating_reports() {
    assert_mnpt_v3_is_rejected("candidate");
}

fn assert_mnpt_v3_is_rejected(name: &str) {
    use minase::eval::pst::{FM_RANK, FmWeights};
    let fixture = Fixture::new();
    let signs = [1; FM_RANK];
    // FM補正が0でも、版3であれば拒否する。
    for embedding in [2, 0] {
        let u = vec![[embedding; FM_RANK]; FEATURE_COUNT];
        let model = Pst::from_weights_with_fm(
            &fixture.weights,
            piece_values(),
            100.0,
            "L0,P0,R1,E0",
            FmWeights {
                embeddings: &u,
                signs: &signs,
                exponent: 1,
            },
        )
        .unwrap();
        fs::write(fixture.root.join(format!("{name}.bin")), model.encode()).unwrap();
        for missing_data in [false, true] {
            let data = if missing_data {
                vec![fixture.root.join("missing-data.bin")]
            } else {
                fixture.data.clone()
            };
            let output_dir = fixture
                .root
                .join(format!("rejected-{embedding}-{missing_data}"));
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_minase-trainer"))
                .args(["diag", "pst-compare", "--data"])
                .args(&data)
                .arg("--base")
                .arg(fixture.root.join("base.bin"))
                .arg("--candidate")
                .arg(fixture.root.join("candidate.bin"))
                .arg("--candidate-float")
                .arg(fixture.root.join("float.npz"))
                .arg("--output-dir")
                .arg(&output_dir)
                .output()
                .unwrap();
            let error = String::from_utf8(output.stderr).unwrap();
            assert_eq!(output.status.code(), Some(1), "{error}");
            assert!(error.contains(&format!("--{name}")), "{error}");
            for expected in [
                "PST-only weights",
                "MNPT version 2",
                "MNPT version 3",
                "FM",
                "minase-trainer fm scale --fm-scale 0",
            ] {
                assert!(error.contains(expected), "{error}");
            }
            assert!(!error.contains("loading diagnostic data"), "{error}");
            assert!(output.stdout.is_empty());
            assert!(!output_dir.exists());
        }
    }
}

#[test]
fn unreadable_or_invalid_mnpt_inputs_still_save_error_reports() {
    for name in ["base", "candidate"] {
        let fixture = Fixture::new();
        let path = fixture.root.join(format!("{name}.bin"));
        for failure in ["invalid", "missing"] {
            if failure == "invalid" {
                fs::write(&path, b"MNPT").unwrap();
            } else {
                fs::remove_file(&path).unwrap();
            }
            let error = diagnostics::run(Command::PstCompare(fixture.args(failure)))
                .unwrap_err()
                .to_string();
            let report = read_report(&fixture.root.join(failure));
            assert_eq!(report["error"], error);
        }
    }
}

#[test]
fn cli_requires_float_weights_and_paired_lookahead_and_uses_documented_defaults() {
    use clap::Parser;
    #[derive(Parser)]
    struct Parsed {
        #[command(flatten)]
        args: PstCompareArgs,
    }
    let required = [
        "pst-compare",
        "--data",
        "a.bin",
        "b.bin",
        "--base",
        "base.bin",
        "--candidate",
        "candidate.bin",
        "--candidate-float",
        "float.npz",
        "--output-dir",
        "new",
    ];
    let parsed = Parsed::try_parse_from(required).unwrap().args;
    assert_eq!((parsed.sample_size, parsed.seed), (10_000, 1));
    assert_eq!(parsed.data.len(), 2);
    for extra in [
        &["--lookahead-gamma", "0.9"][..],
        &["--lookahead-plies", "40"],
        &["--probe", "minase"],
        &["--rescore", "old.bin"],
    ] {
        assert!(Parsed::try_parse_from(required.into_iter().chain(extra.iter().copied())).is_err());
    }
    assert!(
        Parsed::try_parse_from(
            required
                .into_iter()
                .filter(|s| !["--candidate-float", "float.npz"].contains(s))
        )
        .is_err()
    );
    let fixture = Fixture::new();
    // 出力先を含むfixtureの生存中に検査する。
    let mut invalid = fixture.args("zero-sample");
    invalid.sample_size = 0;
    assert!(diagnostics::run(Command::PstCompare(invalid)).is_err());
    assert!(read_report(&fixture.root.join("zero-sample"))["error"].is_string());
}

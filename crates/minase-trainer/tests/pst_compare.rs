//! PST比較診断の出力契約と明示された失敗条件を、合成MNSDで検査する。
#[path = "common/mnsd.rs"]
mod mnsd;
use minase::{
    eval::{features::FEATURE_COUNT, pst::Pst},
    training::records::Header,
};
use minase_core::{Color, PieceCode, PieceKind, Position, PositionBuilder, Square};
use minase_trainer::data::{Outcome, Sample};
use minase_trainer::diagnostics::{self, Command, PstCompareArgs};
use minase_trainer::error::{ContractError, WeightRole};
use serde_json::{Value, json};
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
        fixture.input(11, 1, false);
        fixture.input(22, 2, true);
        fixture
    }
    fn input(&mut self, seed: u64, class: u8, human: bool) {
        let path = self.root.join(format!("input-{seed}.bin"));
        let header = Header::new(
            "L0,P0,R1,E0".into(),
            format!("{class:040x}"),
            [class; 32],
            minase::training::records::TeacherKind::AlphaBeta,
            100,
            seed,
        )
        .unwrap();
        let mut samples = Vec::new();
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
            samples.push(
                Sample::from_position(
                    &position,
                    [100, -200, 300, 400, -500, 600][i],
                    outcome,
                    game,
                    i as u16,
                    if human {
                        minase::training::records::Origin::HumanGame
                    } else {
                        minase::training::records::Origin::RandomSelfplay
                    },
                )
                .unwrap(),
            );
        }
        fs::write(&path, mnsd::independent(header, &samples)).unwrap();
        self.data.push(path);
    }
    fn args(&self, name: &str) -> PstCompareArgs {
        PstCompareArgs {
            teacher_cp_per_logit: minase_trainer::data::TeacherCpPerLogitSource::Calibrate,
            data: self.data.clone(),
            lambda: 0.75,
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
// 診断結果は同じで、記録された出力先だけが実行ごとに異なる。
fn assert_same_report(first: &Path, second: &Path) {
    let left = read_report(first);
    let mut right = read_report(second);
    assert_eq!(left["arguments"]["output_dir"], json!(first));
    assert_eq!(right["arguments"]["output_dir"], json!(second));
    right["arguments"]["output_dir"] = left["arguments"]["output_dir"].clone();
    assert_eq!(left, right);
}
#[test]
fn synthetic_mnsd_preserves_bands_artifacts_promotions_and_deterministic_output() {
    let fixture = Fixture::new();
    let report = fixture.run("first");
    assert_eq!(report["bands"].as_array().unwrap().len(), 10);
    assert_eq!(report["rust_agreement"], json!({"base":6,"candidate":6}));
    assert_eq!(report["teacher_comparison_excluded"], 3);
    assert_eq!(report["teacher_cp_per_logit"][1], Value::Null);
    assert_eq!(report["bands"][0]["samples"], 2);
    assert_eq!(report["bands"][1]["reason"], "empty band");
    assert!(report["bands"][1]["validation_loss"]["base"].is_null());
    assert_eq!(
        report["bands"][5]["teacher_comparison_reason"],
        "lambda is zero"
    );
    assert!(report["bands"][5]["base"].is_null());
    // 代表は初期局面、疎局面、初期局面。RULES.md第18条第1項により、
    // 成れる手は疎局面の先手歩がrank=7から敵陣のrank=8へ進む1手。
    assert_eq!(
        report["rust_promotion_agreement"],
        json!({"base": 1, "candidate": 1})
    );
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
    use minase::training::records::{
        Origin, Outcome as GameOutcome, Policy, Reader, Value as TeacherValue,
    };
    let mut reader = Reader::new(bytes.as_slice()).unwrap();
    assert_eq!(reader.header().rule_set(), "L0,P0,R1,E0");
    assert_eq!(reader.header().game_count(), 6);
    let dataset = minase_trainer::data::Dataset::open(
        &fixture.data,
        None,
        std::num::NonZeroUsize::new(1).unwrap(),
    )
    .unwrap();
    for (number, &index) in dataset.validation_indices().iter().enumerate() {
        let game = reader.read_game().unwrap().unwrap();
        assert_eq!(game.record().game_number(), number as u32 + 1);
        assert_eq!(
            game.positions()[0],
            dataset.record(index).unwrap().to_position().unwrap()
        );
        assert_eq!(
            game.record().start_ply(),
            dataset.record(index).unwrap().ply()
        );
    }
    assert!(reader.read_game().unwrap().is_none());
    for name in [
        "diagnostic-samples.bin",
        "representatives.bin",
        "removals.bin",
    ] {
        let mut reader = Reader::new(fs::File::open(first.join(name)).unwrap()).unwrap();
        let mut number = 0;
        while let Some(game) = reader.read_game().unwrap() {
            number += 1;
            assert_eq!(game.record().game_number(), number);
            assert_eq!(game.record().origin(), Origin::Synthetic);
            assert_eq!(game.record().outcome(), GameOutcome::NotFromGame);
            assert_eq!(game.record().entries().len(), 1);
            let entry = &game.record().entries()[0];
            assert_eq!(entry.value(), TeacherValue::None);
            assert_eq!(entry.policy(), &Policy::None);
            assert_eq!(entry.played_move(), None);
            game.positions()[0].validate().unwrap();
        }
    }
    let mut representatives =
        Reader::new(fs::File::open(first.join("representatives.bin")).unwrap()).unwrap();
    let initial = representatives.read_game().unwrap().unwrap();
    assert_eq!(initial.positions(), &[Position::initial()]);
    assert_eq!(initial.record().start_ply(), 0);
    assert_eq!(report["representatives"][1]["index"], 3);
    assert_eq!(report["representatives"][2]["index"], 5);
    diagnostics::run(Command::PstCompare(fixture.args("repeated"))).unwrap();
    for entry in fs::read_dir(&first).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "report.json" {
            assert_same_report(&first, &fixture.root.join("repeated"));
        } else {
            assert_eq!(
                fs::read(entry.path()).unwrap(),
                fs::read(fixture.root.join("repeated").join(entry.file_name())).unwrap()
            );
        }
    }
    // 新規ディレクトリの契約。既存成果物を上書きしない。
    let paths: Vec<_> = fs::read_dir(&first)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let before: Vec<_> = paths
        .iter()
        .map(|name| fs::read(fixture.root.join("first").join(name)).unwrap())
        .collect();
    let error = diagnostics::run(Command::PstCompare(fixture.args("first"))).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    for (name, bytes) in paths.iter().zip(before) {
        assert_eq!(
            fs::read(fixture.root.join("first").join(name)).unwrap(),
            bytes
        );
    }
}

#[test]
fn quantization_failure_writes_report_before_returning_error() {
    let fixture = Fixture::new();
    let wrong = vec![[500.0; 2]; FEATURE_COUNT];
    fs::write(
        fixture.root.join("float.npz"),
        minase_trainer::pst::encode_pst_npz(&wrong).unwrap(),
    )
    .unwrap();
    let mut args = fixture.args("failed");
    args.lookahead_gamma = None;
    args.lookahead_plies = None;
    let error = diagnostics::run(Command::PstCompare(args)).unwrap_err();
    let report = read_report(&fixture.root.join("failed"));
    assert!(
        report["quantization"]["mean_absolute_error_cp"]
            .as_f64()
            .unwrap()
            > 2.0
    );
    assert_eq!(report["error"], error.to_string());
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
    // 非王駒は初期局面の90枚が2組と疎局面の歩2枚。全重みの符号反転で全182件が反転する。
    assert_eq!(report["pst_removal_sign_reversals"], 182);
    assert!(fixture.root.join("reversal/removals.bin").exists());
}

#[test]
fn games_are_counted_by_file_and_game_number() {
    let mut fixture = Fixture::new();
    fixture.data.clear();
    fixture.input(11, 1, true);
    // 1局の3標本を合法な手順で作り、コピーした別ファイルでは別の対局として数える。
    use minase::training::records::{
        Entry, Exclusion, GameRecord, Origin, Outcome as GameOutcome, Policy, Reader,
        Value as TeacherValue, sorted_legal_moves,
    };
    use minase_core::{Game, GameResult, Rules, WinReason};
    let reader = Reader::new(fs::File::open(&fixture.data[0]).unwrap()).unwrap();
    let header = reader.header().clone();
    let start = sparse();
    let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start.clone());
    let mut entries = Vec::new();
    for (index, score) in [100, -200, 300].into_iter().enumerate() {
        let mv = sorted_legal_moves(&game).unwrap()[index];
        entries.push(
            Entry::new(
                Some(mv),
                TeacherValue::AlphaBeta(score),
                Exclusion::None,
                Policy::None,
                None,
            )
            .unwrap(),
        );
        game.play(mv).unwrap();
    }
    entries.push(
        Entry::new(
            None,
            TeacherValue::None,
            Exclusion::None,
            Policy::None,
            None,
        )
        .unwrap(),
    );
    let record = GameRecord::new(
        4,
        Origin::HumanGame,
        GameOutcome::Game(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::Resignation,
        }),
        0,
        start,
        entries,
    )
    .unwrap();
    let training = GameRecord::new(
        1,
        record.origin(),
        record.outcome(),
        record.start_ply(),
        record.start_position().clone(),
        record.entries().to_vec(),
    )
    .unwrap();
    fs::write(&fixture.data[0], mnsd::bytes(header, &[training, record])).unwrap();
    let duplicate = fixture.root.join("other.bin");
    fs::copy(&fixture.data[0], &duplicate).unwrap();
    fixture.data.push(duplicate);
    let report = fixture.run("human");
    assert_eq!(report["outcome_metrics"]["base"]["games"], 2);
    assert_eq!(report["outcome_metrics"]["base"]["records"], 6);
}

#[test]
fn seeded_sampling_is_without_replacement_and_applies_the_cap_per_band() {
    let fixture = Fixture::new();
    for name in ["small1", "small2"] {
        let mut args = fixture.args(name);
        args.sample_size = 1;
        diagnostics::run(Command::PstCompare(args)).unwrap();
    }
    assert_same_report(&fixture.root.join("small1"), &fixture.root.join("small2"));
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
    let bytes = fs::read(path).unwrap();
    use minase::training::records::{GameRecord, Reader};
    let mut reader = Reader::new(bytes.as_slice()).unwrap();
    let header = reader.header().clone();
    let mut games = Vec::new();
    while let Some(game) = reader.read_game().unwrap() {
        games.push(game.record().clone());
    }
    let previous = &games[3];
    games[3] = GameRecord::new(
        previous.game_number(),
        previous.origin(),
        previous.outcome(),
        previous.start_ply(),
        position.clone(),
        previous.entries().to_vec(),
    )
    .unwrap();
    fs::write(path, mnsd::bytes(header, &games)).unwrap();
    let report = fixture.run("skipped");
    assert_eq!(report["rust_removal_agreement"]["base"]["skipped"], 1);
    let removal = &report["representatives"][1]["removals"][0];
    assert_eq!(removal["square"], square.dense_index());
    assert!(removal["total_delta_cp"]["base"].is_null());
    assert!(removal["delta_cp"]["base"].is_i64());
    assert_eq!(
        removal["skipped"]["base"],
        "lion-promotion flag mismatch: recorded true, restored false"
    );
    let mut reader =
        Reader::new(fs::File::open(fixture.root.join("skipped/removals.bin")).unwrap()).unwrap();
    let mut count = 0;
    while let Some(game) = reader.read_game().unwrap() {
        game.positions()[0].validate().unwrap();
        count += 1;
    }
    assert_eq!(
        count,
        report["rust_removal_agreement"]["base"]["checked"]
            .as_u64()
            .unwrap()
    );
}

#[test]
fn mnpt_with_fm_is_rejected_before_data_loading_without_creating_reports() {
    use minase::eval::pst::{FM_RANK, FmWeights};
    for name in ["base", "candidate"] {
        let fixture = Fixture::new();
        // FM補正が0でも、次元32のFMを持てば拒否する。
        for embedding in [2, 0] {
            let u = vec![[embedding; FM_RANK]; FEATURE_COUNT];
            let model = Pst::from_weights_with_fm(
                &fixture.weights,
                piece_values(),
                100.0,
                "L0,P0,R1,E0",
                FmWeights {
                    embeddings: &u,
                    signs: &[1; FM_RANK],
                    exponent: 1,
                },
            )
            .unwrap();
            fs::write(fixture.root.join(format!("{name}.bin")), model.encode()).unwrap();
            for missing_data in [false, true] {
                let case = format!("{name}-{embedding}-{missing_data}");
                let mut args = fixture.args(&case);
                if missing_data {
                    args.data = vec![fixture.root.join("missing-data.bin")];
                }
                let output_dir = args.output_dir.clone();
                let error = diagnostics::run(Command::PstCompare(args)).unwrap_err();
                let role = if name == "base" {
                    WeightRole::Base
                } else {
                    WeightRole::Candidate
                };
                assert_eq!(
                    error.downcast_ref::<ContractError>(),
                    Some(&ContractError::FmWeights(role)),
                    "{case}"
                );
                assert!(!output_dir.exists(), "{case}");
            }
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
        "--teacher-cp-per-logit",
        "calibrate",
        "--lambda",
        "0.75",
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

#[test]
fn comparison_records_calibration_source_multiplier_bounds_and_inactive_classes() {
    use minase_trainer::data::{CalibrateArgs, TeacherCpPerLogitSource, calibrate};
    let fixture = Fixture::new();
    let estimated = fixture.run("estimated");
    assert_eq!(estimated["teacher_cp_per_logit_source"], "calibrate");
    assert_eq!(estimated["arguments"]["teacher_cp_per_logit"], "calibrate");
    assert!(estimated["teacher_cp_per_logit_at_search_bound"][1].is_null());
    assert!(estimated["teacher_score_multiplier"][1].is_null());
    let value = estimated["teacher_cp_per_logit"][0].as_f64().unwrap();
    assert_eq!(
        estimated["teacher_score_multiplier"][0],
        json!(100.0 / value)
    );
    let path = fixture.root.join("calibration.json");
    calibrate(CalibrateArgs {
        data: fixture.data.clone(),
        lookahead_gamma: Some(0.9),
        lookahead_plies: Some(40),
        output: path.clone(),
        threads: Some(1),
    })
    .unwrap();
    let mut calibration: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // 診断が求めたf64の尺度を固定し、較正ファイルの経路でも同じ報告値を得る。
    calibration["classes"][0]["teacher_cp_per_logit"] = json!(value);
    calibration["classes"][0]["at_search_bound"] =
        estimated["teacher_cp_per_logit_at_search_bound"][0].clone();
    fs::write(&path, serde_json::to_vec(&calibration).unwrap()).unwrap();
    let mut args = fixture.args("fixed");
    args.teacher_cp_per_logit = TeacherCpPerLogitSource::File(path.clone());
    diagnostics::run(Command::PstCompare(args)).unwrap();
    let fixed = read_report(&fixture.root.join("fixed"));
    for key in [
        "teacher_cp_per_logit",
        "teacher_score_multiplier",
        "teacher_cp_per_logit_at_search_bound",
        "cp_per_logit",
        "bands",
        "outcome_metrics",
    ] {
        assert_eq!(fixed[key], estimated[key], "{key}");
    }
    assert_eq!(
        fixed["teacher_cp_per_logit_source"],
        json!({"calibration_file": path.canonicalize().unwrap(), "calibration": calibration})
    );
}

#[test]
fn invalid_calibration_keeps_its_typed_error_and_is_saved_before_sampling() {
    use minase_trainer::data::{CalibrateArgs, Error, TeacherCpPerLogitSource, calibrate};
    let fixture = Fixture::new();
    let path = fixture.root.join("calibration.json");
    calibrate(CalibrateArgs {
        data: fixture.data.clone(),
        lookahead_gamma: Some(0.9),
        lookahead_plies: Some(40),
        output: path.clone(),
        threads: Some(1),
    })
    .unwrap();
    let mut calibration: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    calibration["classes"] = json!([]);
    fs::write(&path, serde_json::to_vec(&calibration).unwrap()).unwrap();
    let mut args = fixture.args("invalid-calibration");
    args.teacher_cp_per_logit = TeacherCpPerLogitSource::File(path);
    let error = diagnostics::run(Command::PstCompare(args)).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::MissingCalibrationClass { class: 0 })
    ));
    let report = read_report(&fixture.root.join("invalid-calibration"));
    assert!(report["error"].is_string());
    assert!(
        !fixture
            .root
            .join("invalid-calibration/diagnostic-samples.bin")
            .exists()
    );
}

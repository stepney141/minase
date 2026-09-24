//! phase3.mdおよびspsa.mdの参照値、符号、丸め、再開契約の検証。

#[path = "tests/simulation.rs"]
mod simulation;

use super::*;
use minase::rng::{XorShift64, derive_seed};
use minase::{Color, DrawReason, GameResult};
use model::*;
use params::{Declaration, Parameter, ParameterError};
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

fn parameter() -> Parameter {
    Parameter {
        name: "X".to_owned(),
        start: 250.0,
        min: 100,
        max: 400,
        c_end: 15.0,
        r_end: 0.002,
    }
}
fn settings(n: u64, pairs: usize, concurrency: usize) -> Settings {
    Settings {
        parameters: vec![parameter()],
        alpha: 0.602,
        gamma: 0.101,
        a: 0.1 * n as f64,
        iterations: n,
        pairs_per_iteration: pairs,
        seed: 42,
        concurrency,
    }
}
fn manifest(s: Settings) -> Manifest {
    Manifest {
        engine: EngineIdentity::Commit {
            hash: "a".repeat(40),
            sha256: "b".repeat(64),
        },
        engine_sha256: "b".repeat(64),
        settings: s,
        each: "nodes=1".to_owned(),
        rules_source: "engine-default".to_owned(),
        canonical_rules: vec!["L1".to_owned()],
        max_ply: 4096,
        response_timeout_secs: 120,
        cpu: CpuRecord {
            model: "test".to_owned(),
            physical_cores: Some(2),
            logical_cores: 2,
            physical_memory_bytes: Some(1024),
        },
        runner: HarnessRecord {
            version: "test".to_owned(),
            sha256: "c".repeat(64),
        },
    }
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/spsa-test");
        fs::create_dir_all(&parent).unwrap();
        Self(parent.join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// 先後を入れ替えた2局をharnessの得点変換へ渡す。signは候補側の勝敗。
fn pair(number: u64, signs: [i8; 2]) -> CompletedPair {
    let mut category = Some(0_u8);
    let games = [Color::Black, Color::White]
        .into_iter()
        .zip(signs)
        .map(|(color, sign)| {
            let termination = match sign {
                1 | -1 => {
                    let winner = if sign == 1 { color } else { color.opposite() };
                    if let Some(total) = &mut category {
                        *total += half_points(GameOutcome::Resigned { winner }, color);
                    }
                    TerminationRecord::Resigned {
                        loser: stored_color(winner.opposite()),
                    }
                }
                0 => {
                    if let Some(total) = &mut category {
                        *total += half_points(
                            GameOutcome::Adjudicated(GameResult::Draw {
                                reason: DrawReason::Repetition,
                            }),
                            color,
                        );
                    }
                    TerminationRecord::AdjudicatedDraw {
                        reason: "Repetition".to_owned(),
                    }
                }
                2 => {
                    category = None;
                    TerminationRecord::Cutoff
                }
                _ => panic!("invalid fixture outcome"),
            };
            GameRecord {
                candidate_color: stored_color(color),
                candidate_seed: 1,
                baseline_seed: 2,
                wall_time_ns: 1,
                candidate_cpu_time_ns: None,
                baseline_cpu_time_ns: None,
                candidate_peak_rss_bytes: None,
                baseline_peak_rss_bytes: None,
                turns: Vec::new(),
                termination,
            }
        })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    CompletedPair {
        number,
        output: String::new(),
        result: PairResult {
            category: category.map(usize::from),
            failures: FailureCounts::default(),
        },
        record: PairRecord {
            pair_number: number,
            pair_seed: 1,
            opening: OpeningRecord {
                seed: 1,
                moves: vec![],
            },
            games,
            category,
        },
    }
}

#[test]
fn rates_match_independent_fishtest_reference() {
    for (n, c, r, k, expected_c, expected_a, expected_ratio) in [
        (
            1500,
            15.0,
            0.002,
            1,
            31.396155848051,
            1.898426230660,
            0.060466836763,
        ),
        (
            1500,
            15.0,
            0.002,
            750,
            16.087749252269,
            0.648162435180,
            0.040289193039,
        ),
        (1500, 15.0, 0.002, 1500, 15.0, 0.45, 0.03),
        (
            30000,
            2.4,
            0.002,
            1,
            6.798302550397,
            0.048784711569,
            0.007176013602,
        ),
        (
            30000,
            2.4,
            0.002,
            12345,
            2.625184841829,
            0.018266098761,
            0.006958023858,
        ),
        (
            100,
            100.0,
            0.005,
            50,
            107.251661681794,
            72.018048353349,
            0.671486550642,
        ),
    ] {
        let p = Parameter {
            c_end: c,
            r_end: r,
            ..parameter()
        };
        let (c, a) = rates(&settings(n, 8, 1), &p, k);
        for (actual, expected) in [(c, expected_c), (a, expected_a), (a / c, expected_ratio)] {
            assert!(
                (actual - expected).abs() / expected < 1e-9,
                "{actual} != {expected}"
            );
        }
    }
}

#[test]
fn stochastic_rounding_is_unbiased_coupled_bounded_and_repeatable() {
    let p = parameter();
    let mut rng = XorShift64::new(derive_seed(99, 1));
    let mut sum = [0.0; 2];
    for _ in 0..200_000 {
        let u = uniform(&mut rng);
        let (plus, minus) = round_pair(&p, 250.25, 0.1, 1, u);
        assert_eq!(plus, (250.35 + u).floor() as i32);
        assert_eq!(minus, (250.15 + u).floor() as i32);
        sum[0] += f64::from(plus);
        sum[1] += f64::from(minus);
    }
    assert!((sum[0] / 200_000.0 - 250.35).abs() < 0.003);
    assert!((sum[1] / 200_000.0 - 250.15).abs() < 0.003);
    assert!(((sum[0] - sum[1]) / 200_000.0 - 0.2).abs() < 0.003);
    assert_eq!(round_pair(&p, 100.0, 10.0, 1, 0.99), (110, 100));
    assert_eq!(round_pair(&p, 400.0, 10.0, 1, 0.99), (400, 390));
    let s = settings(1500, 8, 1);
    let first = issue(&s, &[250.25], 721);
    let again = issue(&s, &[250.25], 721);
    assert_eq!(first.flip, again.flip);
    assert_eq!(first.pending, again.pending);
}

#[test]
fn candidate_scores_drive_signed_updates_and_discard_whole_pairs() {
    for (signs, d) in [([1, 1], 16), ([-1, -1], -16), ([0, 0], 0), ([1, 2], 0)] {
        let mut s = settings(1, 8, 1);
        s.parameters = (0..3)
            .map(|i| Parameter {
                name: format!("X{i}"),
                ..parameter()
            })
            .collect();
        let saved = Mutex::new(Vec::new());
        let summary = runner::run(
            &s,
            &BTreeMap::new(),
            |v, _| Ok(Some(pair(v.number, signs))),
            |r| {
                saved.lock().unwrap().push(r.clone());
                Ok(())
            },
        )
        .unwrap();
        let records = saved.into_inner().unwrap();
        assert_eq!(records[0].d, d);
        for (value, flip) in summary.theta.iter().zip(&records[0].flip) {
            let movement = (value - 250.0) * f64::from(*flip);
            assert!((movement - 0.03 * d as f64).abs() < 1e-12);
        }
        assert_eq!(
            summary.discarded_pairs,
            if signs.contains(&2) { 8 } else { 0 }
        );
    }
    let s = settings(1, 8, 1);
    let records = Mutex::new(Vec::new());
    runner::run(
        &s,
        &BTreeMap::new(),
        |v, _| {
            Ok(Some(pair(
                v.number,
                if v.number == 1 { [1, 2] } else { [1, 1] },
            )))
        },
        |r| {
            records.lock().unwrap().push(r.clone());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(records.into_inner().unwrap()[0].d, 14);
}

#[test]
fn parameter_errors_preserve_their_classification() {
    let d = vec![Declaration {
        name: "X".to_owned(),
        default: 200,
        min: 100,
        max: 400,
    }];
    let valid = "# comment\n\n X, 250.25, 100, 400, 15, 0.002\n";
    assert_eq!(parse_parameters(valid, &d).unwrap()[0].start, 250.25);
    assert!(matches!(
        parse_parameters("Other,200,100,400,15,0.002", &d),
        Err(ParameterError::UnknownName(_))
    ));
    for invalid in [
        "X,99,100,400,15,0.002",
        "X,200,0,400,15,0.002",
        "X,200,100,401,15,0.002",
        "X,NaN,100,400,15,0.002",
    ] {
        assert!(matches!(
            parse_parameters(invalid, &d),
            Err(ParameterError::Range(_))
        ));
    }
    assert!(matches!(
        parse_parameters(&format!("{valid}{valid}"), &d),
        Err(ParameterError::Duplicate(_))
    ));
    assert!(matches!(
        parse_parameters("X,200,100", &d),
        Err(ParameterError::FieldCount { .. })
    ));
    assert!(matches!(
        parse_parameters("X,200,100,400,0,0.002", &d),
        Err(ParameterError::NonPositiveRate(_))
    ));
    assert!(matches!(
        parse_parameters("X,200,100,400,15,inf", &d),
        Err(ParameterError::NonPositiveRate(_))
    ));
    assert!(matches!(
        parse_parameters("X,200,100,400,text,0.002", &d),
        Err(ParameterError::Float { .. })
    ));
    assert!(matches!(
        parse_parameters("X,200,bad,400,15,0.002", &d),
        Err(ParameterError::Integer { .. })
    ));
    assert!(matches!(
        declarations(&[]),
        Err(ParameterError::NoDeclarations)
    ));
    assert!(matches!(
        declarations(&["option name Tune_X type check default true".to_owned()]),
        Err(ParameterError::InvalidDeclaration(_))
    ));
    let parsed = declarations(&[
        "option name Threads type spin default 1 min 1 max 32".to_owned(),
        "option name Tune_X type spin default 200 min 100 max 400".to_owned(),
    ])
    .unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].name, "X");
}

#[test]
fn cli_requires_explicit_session_conditions_and_excludes_other_protocols() {
    let args = [
        "spsa_runner",
        "--run-dir",
        "run",
        "--seed",
        "0",
        "--engine",
        "./engine",
        "--params",
        "params.txt",
        "--rules",
        "engine-default",
        "--each",
        "nodes=1",
        "--concurrency",
        "2",
        "--iterations",
        "3",
        "--pairs-per-iteration",
        "8",
    ];
    let parsed = Arguments::try_parse_from(args).unwrap();
    assert_eq!(parsed.max_ply, 4096);
    assert_eq!(parsed.response_timeout, 120);
    for flag in [
        "--run-dir",
        "--seed",
        "--engine",
        "--params",
        "--rules",
        "--each",
        "--concurrency",
        "--iterations",
        "--pairs-per-iteration",
    ] {
        let index = args.iter().position(|s| *s == flag).unwrap();
        let mut missing = args.to_vec();
        missing.drain(index..index + 2);
        assert!(Arguments::try_parse_from(missing).is_err(), "{flag}");
    }
    assert!(
        Arguments::try_parse_from([
            "spsa_runner",
            "params",
            "--engine",
            "./engine",
            "--rules",
            "engine-default"
        ])
        .is_ok()
    );
    for spec in ["random", "cecp:engine"] {
        assert!(tuning_spec(spec).is_err());
    }
    let mut both = args.to_vec();
    both.extend(["--resume", "other"]);
    assert!(Arguments::try_parse_from(both).is_err());
}

// phase3.md「再開の検査」: 保存の前後と、発行順が逆転した完了を再現する。
#[test]
fn resume_reissues_only_missing_iterations_at_all_three_crash_boundaries() {
    for boundary in ["reversed", "temporary", "committed"] {
        let directory = Directory::new();
        let m = manifest(settings(4, 1, if boundary == "reversed" { 2 } else { 1 }));
        let store = Store::create(&directory.0, &m).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let receiver = Mutex::new(receiver);
        let run = runner::run(
            &m.settings,
            &BTreeMap::new(),
            |v, stop| {
                if boundary == "reversed" && v.number == 1 {
                    receiver
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    // 反復2を確定させた保存関数がエラーを返すまで、反復1は未完了。
                    while !stop.load(Ordering::Acquire) {
                        std::thread::yield_now();
                    }
                    return Ok(None);
                }
                Ok(Some(pair(v.number, [1, 1])))
            },
            |record| {
                if boundary == "temporary" {
                    fs::write(
                        directory
                            .0
                            .join("iterations")
                            .join(format!(".{:020}.json.tmp", record.k)),
                        b"{\"partial\":",
                    )?;
                } else {
                    store.save(record)?;
                }
                if boundary == "reversed" {
                    assert_eq!(record.k, 2);
                    sender.send(()).unwrap();
                }
                Err(io::Error::other("simulated interruption"))
            },
        );
        assert!(run.is_err());
        drop(store);
        let (store, records) = Store::resume(&directory.0, &m).unwrap();
        assert_eq!(
            records.keys().copied().collect::<Vec<_>>(),
            match boundary {
                "reversed" => vec![2],
                "temporary" => vec![],
                _ => vec![1],
            }
        );
        assert!(
            !directory
                .0
                .join("iterations/.00000000000000000001.json.tmp")
                .exists()
        );
        let executed = Mutex::new(Vec::new());
        let summary = runner::run(
            &m.settings,
            &records,
            |v, _| {
                executed.lock().unwrap().push(v.number);
                Ok(Some(pair(v.number, [1, 1])))
            },
            |r| store.save(r),
        )
        .unwrap();
        assert_eq!(summary.applied, 4);
        let executed = executed.into_inner().unwrap();
        for number in records.keys() {
            assert!(!executed.contains(number));
        }
        drop(store);
        let (_, records) = Store::resume(&directory.0, &m).unwrap();
        assert_eq!(
            records.keys().copied().collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(
            storage::validate_chain(&m.settings, &records).unwrap(),
            summary.theta
        );
        let mut order: Vec<_> = records.values().map(|r| r.application).collect();
        order.sort();
        assert_eq!(order, [1, 2, 3, 4]);
    }
}

#[test]
fn resume_rejects_double_updates_missing_applications_and_manifest_changes() {
    let directory = Directory::new();
    let m = manifest(settings(3, 1, 1));
    let store = Store::create(&directory.0, &m).unwrap();
    runner::run(
        &m.settings,
        &BTreeMap::new(),
        |v, _| Ok(Some(pair(v.number, [1, 1]))),
        |r| store.save(r),
    )
    .unwrap();
    assert_eq!(
        Store::resume(&directory.0, &m).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(store);
    let (_, records) = Store::resume(&directory.0, &m).unwrap();
    for change in ["seed", "iterations", "engine"] {
        let mut different = m.clone();
        match change {
            "seed" => different.settings.seed += 1,
            "iterations" => different.settings.iterations += 1,
            _ => different.engine_sha256.push('0'),
        }
        assert!(Store::resume(&directory.0, &different).is_err());
    }
    let file = directory.0.join("iterations/00000000000000000002.json");
    let mut doubled = records[&2].clone();
    doubled.theta = update(
        &m.settings,
        &doubled.theta,
        doubled.k,
        &doubled.flip,
        doubled.d,
    );
    fs::write(&file, serde_json::to_vec(&doubled).unwrap()).unwrap();
    assert!(Store::resume(&directory.0, &m).is_err());
    fs::remove_file(&file).unwrap();
    assert!(Store::resume(&directory.0, &m).is_err());
    let mut duplicate = records[&2].clone();
    duplicate.application = 1;
    fs::write(&file, serde_json::to_vec(&duplicate).unwrap()).unwrap();
    assert!(Store::resume(&directory.0, &m).is_err());
    // 打ち切りの局を含むペアが分類を持つ記録は、破棄の規則と矛盾する。
    let mut cutoff = records[&2].clone();
    cutoff.pairs[0].terminations[0] = TerminationRecord::Cutoff;
    fs::write(&file, serde_json::to_vec(&cutoff).unwrap()).unwrap();
    assert!(Store::resume(&directory.0, &m).is_err());
    fs::write(&file, serde_json::to_vec(&records[&2]).unwrap()).unwrap();
    assert!(Store::resume(&directory.0, &m).is_ok());
}

// 後発反復も発行時のθを使い、完了時には最新のθへ加算する。
#[test]
fn asynchronous_updates_use_issue_index_and_current_theta() {
    let s = settings(2, 1, 2);
    let (sender, receiver) = std::sync::mpsc::channel();
    let receiver = Mutex::new(receiver);
    let records = Mutex::new(Vec::new());
    let summary = runner::run(
        &s,
        &BTreeMap::new(),
        |v, _| {
            if v.number == 1 {
                receiver
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            let expected = issue(&s, &[250.0], v.number).pending.pop_front().unwrap();
            assert_eq!(v, &expected);
            Ok(Some(pair(v.number, [1, 1])))
        },
        |r| {
            records.lock().unwrap().push(r.clone());
            if r.k == 2 {
                sender.send(()).unwrap();
            }
            Ok(())
        },
    )
    .unwrap();
    let records = records.into_inner().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| (r.k, r.application))
            .collect::<Vec<_>>(),
        [(2, 1), (1, 2)]
    );
    let mut expected = 250.0;
    for r in &records {
        // N=2の独立した数式。発行番号と適用番号を取り違えると一致しない。
        let c = 15.0 * (2.0 / r.k as f64).powf(0.101);
        let a = 0.002 * 225.0 * (2.2 / (0.2 + r.k as f64)).powf(0.602);
        expected += a / c * 2.0 * f64::from(r.flip[0]);
    }
    assert!((summary.theta[0] - expected).abs() < 1e-12);
}

#[test]
fn abnormal_games_keep_opening_and_think_times_without_normal_movelist() {
    let s = settings(1, 1, 1);
    let values = issue(&s, &[250.0], 1).pending.pop_front().unwrap();
    let mut p = pair(1, [1, 1]);
    p.record.opening.moves.push("1a1b".to_owned());
    p.record.games[0].termination = TerminationRecord::Forfeit {
        loser: StoredColor::White,
        reason: FailureKind::TimeForfeit,
    };
    p.record.games[0].turns.push(TurnRecord {
        side: StoredColor::White,
        think_time_ns: 1_234_567,
        evaluation: None,
        stop_reason: None,
        completed_time_ms: None,
        ponder: None,
        response: TurnResponse::Failure {
            reason: FailureKind::TimeForfeit,
        },
    });
    p.result.failures.time_forfeits = 1;
    let observation = PairObservation::from_completed(values, p);
    assert_eq!(observation.difference(), 2);
    assert_eq!(observation.failures.time_forfeits, 1);
    let value = serde_json::to_value(&observation).unwrap();
    assert_eq!(value["abnormal_games"].as_array().unwrap().len(), 1);
    assert_eq!(value["abnormal_games"][0]["opening"]["moves"][0], "1a1b");
    assert_eq!(
        value["abnormal_games"][0]["game"]["turns"][0]["think_time_ns"],
        1_234_567
    );
    assert!(value.get("games").is_none());
}

// phase3.md「合成目的関数の試験」。最適値は範囲の中央、開始値は中央＋20%。
#[test]
#[ignore]
fn synthetic_objective_improves_at_least_eighteen_of_twenty_seeds() {
    // 範囲の出典はsrc/search/params.rs。表の宣言を読むのでfeatureに依存しない。
    let table = include_str!("../../search/params.rs")
        .split_once("parameters! {")
        .unwrap()
        .1
        .split_once("\n}")
        .unwrap()
        .0;
    let mut parameters = Vec::new();
    for line in table.lines().filter(|l| l.contains("): ")) {
        let Some((name, rest)) = line.trim().split_once('(') else {
            continue;
        };
        let Some((_, values)) = rest.split_once("): ") else {
            continue;
        };
        let values: Vec<i32> = values
            .trim_end_matches(';')
            .split(',')
            .map(|v| v.trim().parse().unwrap())
            .collect();
        let (min, max) = (values[1], values[2]);
        let width = f64::from(max - min);
        parameters.push(Parameter {
            name: name.to_owned(),
            start: f64::from(min) + 0.7 * width,
            min,
            max,
            c_end: width / 20.0,
            r_end: 0.002,
        });
    }
    assert_eq!(parameters.len(), 22);
    let elo = |values: &[i32]| -> f64 {
        values
            .iter()
            .zip(&parameters)
            .enumerate()
            .map(|(i, (&x, p))| {
                let width = f64::from(p.max - p.min);
                let mut normalized = (f64::from(x) - f64::from(p.min)) / width;
                if i == 10 {
                    normalized = (normalized * 10.0).floor() / 10.0;
                }
                -60.0 * (normalized - 0.5).powi(2)
            })
            .sum()
    };
    let mut improved = 0;
    for seed in 1..=20 {
        let s = Settings {
            parameters: parameters.clone(),
            seed,
            ..settings(1500, 8, 1)
        };
        let summary = runner::run(
            &s,
            &BTreeMap::new(),
            |v, _| {
                let advantage = elo(&v.plus) - elo(&v.minus);
                let probability = 1.0 / (1.0 + 10.0_f64.powf(-advantage / 400.0));
                // ペアごとの独立した系列から2局分を引く。
                let mut rng = XorShift64::new(derive_seed(seed * 1_000_000, v.number));
                let signs = std::array::from_fn(|_| {
                    if uniform(&mut rng) < probability {
                        1
                    } else {
                        -1
                    }
                });
                Ok(Some(pair(v.number, signs)))
            },
            |_| Ok(()),
        )
        .unwrap();
        let distance = summary
            .theta
            .iter()
            .zip(&parameters)
            .map(|(&x, p)| ((x - f64::from(p.min)) / f64::from(p.max - p.min) - 0.5).abs())
            .sum::<f64>()
            / 22.0;
        println!("seed={seed:02} mean_normalized_distance={distance:.12}");
        if distance < 0.2 {
            improved += 1;
        }
        assert_eq!(summary.valid_pairs, 12_000);
    }
    assert!(improved >= 18, "only {improved}/20 improved");
}

#[test]
fn command_engine_runs_resumes_and_sets_only_listed_parameters() {
    let directory = Directory::new();
    fs::create_dir(&directory.0).unwrap();
    let script = directory.0.join("engine.py");
    let transcript = directory.0.join("commands.txt");
    fs::write(
        &script,
        r#"
import sys
with open(sys.argv[1], 'a', buffering=1) as log:
    for line in sys.stdin:
        log.write(line)
        if line.strip() == 'usi':
            print('option name Tune_X type spin default 250 min 100 max 400')
            print('option name Tune_Y type spin default 50 min 0 max 100')
            print('usiok', flush=True)
        elif line.strip() == 'isready':
            print('readyok', flush=True)
        elif line.startswith('go '):
            print('bestmove resign', flush=True)
"#,
    )
    .unwrap();
    let params_file = directory.0.join("params.txt");
    fs::write(&params_file, "X,250.25,100,400,15,0.002\n").unwrap();
    let engine = format!("python3 -u {} {}", script.display(), transcript.display());
    let run_dir = directory.0.join("run");
    for operation in ["--run-dir", "--resume"] {
        let args = Arguments::try_parse_from([
            "spsa_runner",
            operation,
            run_dir.to_str().unwrap(),
            "--seed",
            "42",
            "--engine",
            &engine,
            "--params",
            params_file.to_str().unwrap(),
            "--rules",
            "engine-default",
            "--each",
            "nodes=1",
            "--concurrency",
            "1",
            "--iterations",
            "2",
            "--pairs-per-iteration",
            "1",
        ])
        .unwrap();
        execute(args).unwrap();
    }
    let commands = fs::read_to_string(&transcript).unwrap();
    assert_eq!(
        commands
            .lines()
            .filter(|l| l.starts_with("setoption name Tune_X "))
            .count(),
        8
    );
    for forbidden in [
        "setoption name Tune_Y ",
        "setoption name Threads ",
        "setoption name USI_Hash ",
    ] {
        assert!(!commands.contains(forbidden));
    }
    let m: Manifest =
        serde_json::from_slice(&fs::read(run_dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m.engine_sha256.len(), 64);
    let (_, records) = Store::resume(&run_dir, &m).unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .values()
            .all(|r| r.failures == FailureCounts::default())
    );
    assert_eq!(records[&2].theta, [250.25]);
}

#[test]
fn each_failure_reason_is_scored_as_the_offending_sides_loss() {
    for (kind, failure) in [
        (FailureKind::IllegalMove, EngineFailure::IllegalMove),
        (FailureKind::Crash, EngineFailure::Crash),
        (FailureKind::Timeout, EngineFailure::Timeout),
        (FailureKind::TimeForfeit, EngineFailure::TimeForfeit),
        (FailureKind::RejectedMove, EngineFailure::RejectedMove),
    ] {
        let s = settings(1, 8, 1);
        let summary = runner::run(
            &s,
            &BTreeMap::new(),
            |v, _| {
                let mut p = pair(v.number, [1, 1]);
                for (g, color) in p.record.games.iter_mut().zip([Color::Black, Color::White]) {
                    g.termination = TerminationRecord::Forfeit {
                        loser: stored_color(color.opposite()),
                        reason: kind,
                    };
                    assert_eq!(
                        half_points(
                            GameOutcome::Forfeit {
                                winner: color,
                                reason: failure
                            },
                            color
                        ),
                        2
                    );
                    p.result.failures.record(failure);
                }
                Ok(Some(p))
            },
            |r| {
                assert_eq!(r.d, 16);
                assert!(r.pairs.iter().all(|p| p.abnormal_games.len() == 2));
                Ok(())
            },
        )
        .unwrap();
        let mut expected = FailureCounts::default();
        for _ in 0..16 {
            expected.record(failure);
        }
        assert_eq!(summary.failures, expected);
    }
}

#[test]
fn worker_panics_cancel_other_workers_and_return_an_error() {
    let s = settings(2, 1, 2);
    let barrier = std::sync::Barrier::new(2);
    let result = runner::run(
        &s,
        &BTreeMap::new(),
        |v, stop| {
            barrier.wait();
            if v.number == 2 {
                panic!("injected worker panic");
            }
            while !stop.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            Ok(None)
        },
        |_| panic!("an interrupted iteration must not be saved"),
    );
    assert!(result.is_err());
}

// phase3.mdの乱数の消費順を独立したPython計算の参照値で固定する。
#[test]
fn iteration_stream_draws_all_flips_before_per_pair_rounding() {
    let mut s = settings(1500, 2, 1);
    s.parameters = (0..3)
        .map(|i| Parameter {
            name: format!("X{i}"),
            ..parameter()
        })
        .collect();
    let issued = issue(&s, &[250.25; 3], 721);
    assert_eq!(issued.flip, [-1, 1, -1]);
    assert_eq!(
        issued.pending[0],
        PairValues {
            number: 1441,
            plus: vec![234, 266, 234],
            minus: vec![266, 234, 267]
        }
    );
    assert_eq!(
        issued.pending[1],
        PairValues {
            number: 1442,
            plus: vec![234, 267, 234],
            minus: vec![267, 234, 266]
        }
    );
}

#[test]
fn session_rejects_counter_overflow_and_nonfinite_schedule() {
    let mut s = settings(u64::MAX, 8, 1);
    assert!(validate_settings(&s).is_err());
    s = settings(1500, 8, 1);
    s.parameters[0].r_end = f64::MAX;
    assert!(validate_settings(&s).is_err());
    s.parameters[0].r_end = 0.002;
    s.parameters[0].c_end = f64::MIN_POSITIVE;
    assert!(validate_settings(&s).is_err());
}

// spsa-apply.md「検証」: 保存記録は既存の構築手順で作り、エンジンを起動しない。
const APPLY_TABLE: &str = "// outside X(x): 250, 100, 400;\nparameters! {\n    /// X の説明。250 は変更しない。\n    X(x): +250, 100, 400;\n    Y(y): -2, -10, 10;\n    Z(z): 9, 0, 20;\n}\n// end\n";

fn apply_session(s: Settings, completed: u64, signs: [i8; 2]) -> Directory {
    let directory = Directory::new();
    let m = manifest(s);
    let store = Store::create(&directory.0, &m).unwrap();
    let mut theta = m.settings.initial_theta();
    for k in 1..=completed {
        let mut issued = issue(&m.settings, &theta, k);
        while let Some(values) = issued.pending.pop_front() {
            let observation = pair(values.number, signs);
            issued
                .finished
                .push(PairObservation::from_completed(values, observation));
        }
        let record = issued.complete(&m.settings, &theta, k);
        store.save(&record).unwrap();
        theta = record.theta;
    }
    directory
}

fn apply_source(directory: &Directory, text: &str) -> PathBuf {
    let source = directory.0.join("params.rs");
    fs::write(&source, text).unwrap();
    source
}

fn apply_error(directory: &Directory, text: &str) -> apply::ApplyError {
    let source = apply_source(directory, text);
    let result = apply::apply(&directory.0, &source).unwrap_err();
    assert_eq!(fs::read(&source).unwrap(), text.as_bytes());
    result
}

#[test]
fn apply_changes_only_default_literals_and_preserves_all_session_files() {
    let mut s = settings(1, 1, 1);
    s.alpha = 0.0;
    s.gamma = 0.0;
    s.parameters = vec![
        Parameter {
            c_end: 1.0,
            r_end: 0.25,
            ..parameter()
        },
        Parameter {
            name: "Y".to_owned(),
            start: -2.0,
            min: -10,
            max: 10,
            c_end: 1.0,
            r_end: 0.25,
        },
        Parameter {
            name: "Z".to_owned(),
            start: 9.0,
            min: 0,
            max: 20,
            c_end: 1.0,
            r_end: 0.125,
        },
    ];
    s.seed = (1..1000)
        .find(|seed| {
            let candidate = Settings {
                seed: *seed,
                ..s.clone()
            };
            flips(&candidate, 1) == [1, -1, 1]
        })
        .unwrap();
    let directory = apply_session(s, 1, [1, 1]);
    let source = apply_source(&directory, APPLY_TABLE);
    let temporary = directory
        .0
        .join("iterations/.00000000000000000002.json.tmp");
    fs::write(&temporary, b"{partial\r\n").unwrap();
    let saved: Vec<_> = [
        directory.0.join("manifest.json"),
        directory.0.join("iterations/00000000000000000001.json"),
        temporary,
    ]
    .into_iter()
    .map(|path| {
        let bytes = fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect();
    let report = apply::apply(&directory.0, &source).unwrap();
    let expected = APPLY_TABLE
        .replace("X(x): +250", "X(x): 251")
        .replace("Y(y): -2", "Y(y): -3");
    assert_eq!(fs::read(&source).unwrap(), expected.as_bytes());
    assert_eq!(
        report,
        format!(
            "X: start=250 final=250.5 integer=251 changed\nY: start=-2 final=-2.5 integer=-3 changed\nZ: start=9 final=9.25 integer=9 unchanged\nchanged=2 unchanged=1 source={}\n",
            source.display()
        )
    );
    for (path, bytes) in saved {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn apply_rejects_empty_and_truncated_sessions_before_chain_validation() {
    for completed in [0, 2] {
        let directory = apply_session(settings(3, 1, 1), completed, [0, 0]);
        assert!(
            matches!(apply_error(&directory, APPLY_TABLE), apply::ApplyError::Incomplete { completed: actual, total: 3 } if actual == completed)
        );
    }
}

#[test]
fn apply_rejects_a_complete_session_with_corrupted_theta() {
    let directory = apply_session(settings(2, 1, 1), 2, [0, 0]);
    let path = directory.0.join("iterations/00000000000000000002.json");
    let mut record: IterationRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record.theta[0] = 251.0;
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(matches!(
        apply_error(&directory, APPLY_TABLE),
        apply::ApplyError::Chain(_)
    ));
}

#[test]
fn apply_validates_manifest_settings_before_reading_iterations() {
    let base = settings(1, 1, 1);
    let mut invalid = Vec::new();
    let mut s = base.clone();
    s.pairs_per_iteration = 0;
    invalid.push(s);
    let mut s = base.clone();
    s.parameters[0].min = 401;
    invalid.push(s);
    let mut s = base.clone();
    s.parameters.clear();
    invalid.push(s);
    let mut s = base.clone();
    s.parameters.push(parameter());
    invalid.push(s);
    let mut s = base.clone();
    s.parameters[0].start = 99.0;
    invalid.push(s);
    let mut s = base.clone();
    s.parameters[0].c_end = 0.0;
    invalid.push(s);
    let mut s = base.clone();
    s.parameters[0].r_end = -1.0;
    invalid.push(s);
    let mut s = base.clone();
    s.iterations = 0;
    invalid.push(s);
    let mut s = base.clone();
    s.iterations = u64::MAX;
    invalid.push(s);
    let mut s = base.clone();
    s.concurrency = 0;
    invalid.push(s);
    let mut s = base;
    s.parameters[0].r_end = f64::MAX;
    invalid.push(s);
    for s in invalid {
        let directory = apply_session(s, 0, [0, 0]);
        // 実行条件が壊れていれば、反復ディレクトリの読み込みまで進まない。
        fs::remove_dir(directory.0.join("iterations")).unwrap();
        let error = apply_error(&directory, APPLY_TABLE);
        assert!(
            matches!(error, apply::ApplyError::InvalidSettings(_)),
            "{error:?}"
        );
        assert!(std::error::Error::source(&error).is_some());
    }
}

#[test]
fn apply_rejects_missing_and_duplicate_table_parameters() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    assert!(
        matches!(apply_error(&directory, &APPLY_TABLE.replace("X(x):", "Other(x):")), apply::ApplyError::MissingParameter(name) if name == "X")
    );
    assert!(
        matches!(apply_error(&directory, &APPLY_TABLE.replace("Y(y):", "X(y):")), apply::ApplyError::DuplicateParameter(name) if name == "X")
    );
}

#[test]
fn apply_requires_contained_ranges_and_exact_start_values() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    for text in [
        APPLY_TABLE.replace("+250, 100", "+250, 101"),
        APPLY_TABLE.replace("+250, 100, 400", "+250, 100, 399"),
    ] {
        assert!(
            matches!(apply_error(&directory, &text), apply::ApplyError::RangeMismatch(name) if name == "X")
        );
    }
    assert!(
        matches!(apply_error(&directory, &APPLY_TABLE.replace("+250", "251")), apply::ApplyError::StartMismatch { name, .. } if name == "X")
    );
    let mut s = settings(1, 1, 1);
    s.parameters[0].start = 250.25;
    let fractional = apply_session(s, 1, [0, 0]);
    assert!(matches!(
        apply_error(&fractional, APPLY_TABLE),
        apply::ApplyError::StartMismatch { .. }
    ));
    let wider = APPLY_TABLE.replace("+250, 100, 400", "+250, 0, 500");
    let source = apply_source(&directory, &wider);
    assert!(apply::apply(&directory.0, &source).is_ok());
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        wider.replace("+250", "250")
    );
}

#[test]
fn apply_rejects_unrecognized_table_lines_without_writing() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    for declaration in [
        "/* X(x): 250, 100, 400; */",
        "X(x): 250,\n100, 400;",
        "#[doc = \"X\"]",
        "X(x): 250_0, 100, 400;",
        "X(x): 0xfa, 100, 400;",
        "X(x): 2147483648, 100, 400;",
        "X(x): 250 , 100, 400;",
        "X(x): 250, 100, 400; // comment",
        "🦀(x): 250, 100, 400;",
    ] {
        let text = format!("parameters! {{\n{declaration}\n}}\n");
        assert!(
            matches!(
                apply_error(&directory, &text),
                apply::ApplyError::InvalidLine { line: 2, .. }
            ),
            "{declaration}"
        );
    }
}

#[test]
fn apply_requires_one_start_line_and_a_closing_line() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    assert!(matches!(
        apply_error(&directory, "// no invocation\n"),
        apply::ApplyError::InvocationCount(0)
    ));
    assert!(matches!(
        apply_error(&directory, &format!("{APPLY_TABLE}{APPLY_TABLE}")),
        apply::ApplyError::InvocationCount(2)
    ));
    assert!(matches!(
        apply_error(&directory, "parameters! {\nX(x): 250, 100, 400;\n"),
        apply::ApplyError::MissingEnd
    ));
}

#[test]
fn apply_preserves_indentation_trailing_space_and_crlf() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    let text = "outside\r\n    parameters! { \r\n/// 説明\r\n X(x): +0250, +100, 400; \t\r\n \r\n  } \r\noutside\r\n";
    let source = apply_source(&directory, text);
    apply::apply(&directory.0, &source).unwrap();
    assert_eq!(
        fs::read(source).unwrap(),
        text.replace("+0250", "250").as_bytes()
    );
}

#[test]
fn apply_rejects_locked_sessions_and_keeps_io_sources() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    let _lock = lock_run_directory(&directory.0).unwrap();
    let error = apply_error(&directory, APPLY_TABLE);
    assert!(
        matches!(error, apply::ApplyError::Lock(ref e) if e.kind() == io::ErrorKind::WouldBlock)
    );
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn apply_reports_and_preserves_parameters_not_in_session() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    let source = apply_source(&directory, APPLY_TABLE);
    let report = apply::apply(&directory.0, &source).unwrap();
    assert_eq!(
        fs::read(source.clone()).unwrap(),
        APPLY_TABLE.replace("+250", "250").as_bytes()
    );
    assert_eq!(
        report,
        format!(
            "X: start=250 final=250 integer=250 unchanged\nY: not in session\nZ: not in session\nchanged=0 unchanged=1 source={}\n",
            source.display()
        )
    );
}

#[test]
fn apply_rounds_half_values_away_from_zero_including_negative_values() {
    for (start, delta, expected_final, integer) in [
        (2, 0.5, 2.5, 3),
        (-2, -0.5, -2.5, -3),
        (0, -0.5, -0.5, -1),
        (-2, 0.5, -1.5, -2),
        (-2, -0.25, -2.25, -2),
    ] {
        let mut s = settings(1, 1, 1);
        s.alpha = 0.0;
        s.gamma = 0.0;
        s.parameters = vec![Parameter {
            start: f64::from(start),
            min: -10,
            max: 10,
            c_end: 1.0,
            r_end: f64::abs(delta) / 2.0,
            ..parameter()
        }];
        let sign = if delta > 0.0 {
            flips(&s, 1)[0]
        } else {
            -flips(&s, 1)[0]
        };
        let directory = apply_session(s, 1, [sign, sign]);
        let text = format!("parameters! {{\nX(x): {start}, -10, 10;\n}}");
        let source = apply_source(&directory, &text);
        let report = apply::apply(&directory.0, &source).unwrap();
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            format!("parameters! {{\nX(x): {integer}, -10, 10;\n}}")
        );
        assert!(report.contains(&format!(
            "start={start} final={expected_final} integer={integer} "
        )));
    }
}

#[test]
fn apply_parses_actual_search_parameter_table_and_valid_defaults() {
    let entries = apply::parse_table(include_str!("../../search/params.rs")).unwrap();
    assert!(!entries.is_empty());
    for entry in entries {
        assert!(
            (entry.min..=entry.max).contains(&entry.default),
            "{}",
            entry.name
        );
    }
}

#[test]
fn apply_cli_requires_both_paths_and_executes_without_engine_arguments() {
    for args in [
        vec!["spsa_runner", "apply", "--run-dir", "run"],
        vec!["spsa_runner", "apply", "--source", "params.rs"],
    ] {
        assert!(Arguments::try_parse_from(args).is_err());
    }
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    let source = apply_source(&directory, APPLY_TABLE);
    let arguments = Arguments::try_parse_from([
        "spsa_runner",
        "apply",
        "--run-dir",
        directory.0.to_str().unwrap(),
        "--source",
        source.to_str().unwrap(),
    ])
    .unwrap();
    execute(arguments).unwrap();
    assert_eq!(
        fs::read(source).unwrap(),
        APPLY_TABLE.replace("+250", "250").as_bytes()
    );
}

#[test]
fn apply_rejects_unknown_manifest_fields_without_writing() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    let path = directory.0.join("manifest.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["unexpected"] = serde_json::json!(true);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        apply_error(&directory, APPLY_TABLE),
        apply::ApplyError::Read(_)
    ));
}

#[test]
fn apply_preserves_source_and_existing_temporary_file_on_write_failure() {
    let directory = apply_session(settings(1, 1, 1), 1, [0, 0]);
    let temporary = directory
        .0
        .join(format!(".params.rs.{}.tmp", std::process::id()));
    fs::write(&temporary, b"existing temporary file").unwrap();
    assert!(matches!(
        apply_error(&directory, APPLY_TABLE),
        apply::ApplyError::Write(_)
    ));
    assert_eq!(fs::read(temporary).unwrap(), b"existing temporary file");
}

//! 自己対局測定ハーネスのプロセス境界を検査する統合テスト。

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn run_directory() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "minase-match-run-{}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn run_random_match(concurrency: &str, pairs: &str) -> String {
    let run_dir = run_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run"])
        .arg("--run-dir")
        .arg(&run_dir)
        .args([
            "--seed",
            "20260811",
            "--candidate",
            "random",
            "--baseline",
            "random",
            "--each",
            "nodes=1",
            "--response-timeout",
            "5",
            "--max-ply",
            "400",
            "--concurrency",
            concurrency,
            "elo",
            "--pairs",
            pairs,
        ])
        .output()
        .expect("match_runner must start");
    assert!(
        output.status.success(),
        "match_runner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&run_dir).expect("run directory must be removable");
    String::from_utf8(output.stdout).expect("match_runner output must be UTF-8")
}

fn without_elapsed(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.starts_with("elapsed:") && !line.starts_with("run_dir:"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run_minase_candidate_match(candidate: &str) -> String {
    let run_dir = run_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run"])
        .arg("--run-dir")
        .arg(&run_dir)
        .args([
            "--candidate",
            candidate,
            "--baseline",
            "random",
            "--each",
            "depth=1",
            "--seed",
            "20260824",
            "--max-ply",
            "400",
            "--concurrency",
            "1",
            "elo",
            "--pairs",
            "1",
        ])
        .output()
        .expect("match_runner must start");
    assert!(
        output.status.success(),
        "match_runner failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(&run_dir).expect("run directory must be removable");
    String::from_utf8(output.stdout).expect("match_runner output must be UTF-8")
}

#[test]
fn random_usi_match_output_is_independent_of_concurrency() {
    let sequential = run_random_match("1", "4");
    let parallel = run_random_match("4", "4");

    assert!(sequential.contains("summary: mode=elo pairs=4 valid_pairs=4 discarded_pairs=0"));
    assert!(sequential.contains("engine_failures: illegal_moves=0 crashes=0 timeouts=0"));
    assert_eq!(without_elapsed(&sequential), without_elapsed(&parallel));

    // sprt.md「ペア対局と再現性」: 開始局面はペア番号ごとに決定的に派生したシードで
    // 作る。全ペアが同一シードへ退化していないことをpair_seed=の相異で固定する
    // (変異検証フェーズ4で検出した派生配線の無検証を補強)。
    let seeds: Vec<&str> = sequential
        .lines()
        .filter_map(|line| {
            line.split_whitespace()
                .find(|token| token.starts_with("pair_seed="))
        })
        .collect();
    assert_eq!(seeds.len(), 4);
    let unique: std::collections::HashSet<&str> = seeds.iter().copied().collect();
    assert_eq!(unique.len(), 4);
}

// D8-HARN-20（sprt.md「エンジンの指定方法」、match-harness.md「CECPセッション
// 管理」）: minase自身をCECP候補にした対局は異常なく完走し、同じ起動バイナリをUSI
// 候補にした対局と同じ着手列・結果になる。spec原文の表示はD8-HARN-01の契約により
// 必ず異なるため、両原文だけを同じ表示へ置換して経過時間以外の全出力を比較する。
#[test]
fn minase_cecp_match_is_failure_free_and_matches_usi() {
    let minase = env!("CARGO_BIN_EXE_minase");
    let cecp_spec = format!("cecp:{minase} --protocol cecp --rules engine-default");
    let usi_spec = format!("{minase} --protocol usi --rules engine-default");
    let cecp = run_minase_candidate_match(&cecp_spec);
    let usi = run_minase_candidate_match(&usi_spec);
    let no_failures =
        "engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 rejected_moves=0";
    assert!(cecp.contains(no_failures));
    assert!(usi.contains(no_failures));

    let normalized_cecp = cecp.replace(&cecp_spec, "<candidate>");
    let normalized_usi = usi.replace(&usi_spec, "<candidate>");
    assert_eq!(
        without_elapsed(&normalized_cecp),
        without_elapsed(&normalized_usi)
    );
}

// match-harness-efficiency.md「実行記録と再開」: 欠番より後の確定済み記録を
// 変更せず、最小の欠番だけを再実行して番号順の統計を復元する。
#[test]
fn resume_fills_the_lowest_gap_and_preserves_later_records() {
    let run_dir = run_directory();
    let run = |operation: &str| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
        command.args(["match", "run"]).arg(operation).arg(&run_dir);
        if operation == "--run-dir" {
            command.args([
                "--seed",
                "20260828",
                "--candidate",
                "random",
                "--baseline",
                "random",
                "--each",
                "depth=1",
                "--response-timeout",
                "5",
                "--max-ply",
                "400",
                "--concurrency",
                "1",
                "gsprt",
                "--max-pairs",
                "3",
            ]);
        }
        command.output().expect("match_runner must start")
    };

    let initial = run("--run-dir");
    assert!(
        initial.status.success(),
        "{}",
        String::from_utf8_lossy(&initial.stderr)
    );
    let pair2 = run_dir.join("pairs/00000000000000000002.json");
    let pair3 = run_dir.join("pairs/00000000000000000003.json");
    let pair3_before = std::fs::read(&pair3).unwrap();
    std::fs::remove_file(&pair2).unwrap();
    let mut interrupted = json_file(&run_dir.join("summary.json"));
    interrupted["invocation_active"] = serde_json::json!(true);
    write_json(&run_dir.join("summary.json"), &interrupted);

    let resumed = run("--resume");
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert!(pair2.is_file());
    assert_eq!(std::fs::read(&pair3).unwrap(), pair3_before);

    let initial = String::from_utf8(initial.stdout).unwrap();
    let resumed = String::from_utf8(resumed.stdout).unwrap();
    for prefix in ["summary:", "pentanomial:", "llr:", "engine_failures:"] {
        assert_eq!(
            initial.lines().find(|line| line.starts_with(prefix)),
            resumed.lines().find(|line| line.starts_with(prefix))
        );
    }
    assert!(resumed.contains("pair 1: loaded from saved record"));
    assert!(resumed.contains("pair 3: loaded from saved record"));
    std::fs::remove_dir_all(run_dir).unwrap();
}

// D8-HARN-21（ponder.md「対局ハーネスの対局進行」）。条件違反はmkdirより前に拒否する。
#[test]
fn ponder_invalid_conditions_do_not_create_run_directory() {
    let run_dir = run_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run"])
        .arg("--run-dir")
        .arg(&run_dir)
        .arg("--ponder")
        .args(["--each", "depth=4"])
        .args(["--concurrency", "2", "elo", "--pairs", "1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--ponder"));
    assert!(!run_dir.exists());
}

// D8-HARN-21/26（ponder.md保存形式と再開）。再開集計は既存ペアを含む。
#[test]
fn ponder_resume_replays_all_counts() {
    let run_dir = run_directory();
    let run = |operation, pairs, ponder| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
        command.args(["match", "run"]).arg(operation).arg(&run_dir);
        if operation == "--resume" {
            command.args(["--target-pairs", pairs]);
        } else {
            command.args([
                "--seed",
                "1234",
                "--each",
                "time=10000+100",
                "--max-ply",
                "16",
                "--concurrency",
                "1",
            ]);
            if ponder {
                command.arg("--ponder");
            }
            command.args(["elo", "--pairs", pairs]);
        }
        command.output().unwrap()
    };

    let first = run("--run-dir", "1", true);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let pair_dir = run_dir.join("pairs");
    let resumed = run("--resume", "2", true);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let mut moves = [0_u64; 2];
    for path in std::fs::read_dir(&pair_dir).unwrap() {
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.unwrap().path()).unwrap()).unwrap();
        for game in record["games"].as_array().unwrap() {
            for turn in game["turns"].as_array().unwrap() {
                if turn["response"]["kind"] == "move" {
                    moves[usize::from(turn["side"] != game["candidate_color"])] += 1;
                }
            }
        }
    }
    let text = String::from_utf8(resumed.stdout).unwrap();
    for (name, count) in ["candidate", "baseline"].into_iter().zip(moves) {
        assert!(count > 0);
        assert!(text.contains(&format!(
            "ponder {name}: predictions=0 illegal_predictions=0 go_ponder=0 hits=0 moves={count}"
        )));
    }
    let mismatch = run("--resume", "2", false);
    assert!(!mismatch.status.success());
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("must exceed"));
    std::fs::remove_dir_all(run_dir).unwrap();
}

fn json_file(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn small_run(path: &std::path::Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run", "--run-dir"])
        .arg(path)
        .args([
            "--seed",
            "42",
            "--max-ply",
            "16",
            "--concurrency",
            "1",
            "elo",
            "--pairs",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn resume_run(path: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run", "--resume"])
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

fn small_gsprt_run(path: &std::path::Path, hypotheses: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run", "--run-dir"])
        .arg(path)
        .args([
            "--seed",
            "1234",
            "--max-ply",
            "16",
            "--concurrency",
            "1",
            "gsprt",
            "--max-pairs",
            "3",
        ])
        .args(hypotheses)
        .output()
        .unwrap()
}

// 仮説指定の契約: 新規実行で保存した値を逐次判定と最終サマリに使う。
#[cfg(unix)]
#[test]
fn gsprt_records_hypotheses_and_uses_them_for_each_llr() {
    let parent = run_directory();
    std::fs::create_dir(&parent).unwrap();
    let engine = parent.join("resign.sh");
    std::fs::write(
        &engine,
        r#"
while IFS= read -r line; do
    case "$line" in
        usi) echo usiok ;;
        isready) echo readyok ;;
        go*) echo 'bestmove resign' ;;
        quit) exit 0 ;;
    esac
done
"#,
    )
    .unwrap();
    let player = format!("/bin/sh {}", engine.display());
    for (elo0, elo1) in [
        ("-5", "5"),
        ("-500", "-400"),
        ("400", "500"),
        ("-12.5", "7.25"),
    ] {
        let dir = parent.join(format!("run-{elo0}-{elo1}"));
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["match", "run", "--run-dir"])
            .arg(&dir)
            .args([
                "--seed",
                "1234",
                "--max-ply",
                "16",
                "--concurrency",
                "1",
                "--candidate",
                &player,
                "--baseline",
                &player,
                "gsprt",
                "--max-pairs",
                "3",
                "--elo0",
                elo0,
                "--elo1",
                elo1,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(&format!("hypotheses: elo0={elo0} elo1={elo1}\n")));
        let h0 = elo0.parse::<f64>().unwrap();
        let h1 = elo1.parse::<f64>().unwrap();
        assert_eq!(
            json_file(&dir.join("manifest.json"))["mode"],
            serde_json::json!({
                "kind": "gsprt", "h0_elo": h0, "h1_elo": h1, "alpha": 0.05, "beta": 0.05
            })
        );
        let mut last_llr = None;
        let mut last_decision = None;
        for line in text.lines().filter(|line| line.starts_with("statistics:")) {
            let (counts, tail) = line
                .split_once("pentanomial=")
                .unwrap()
                .1
                .split_once(" llr=")
                .unwrap();
            let counts: [u64; 5] = serde_json::from_str(counts).unwrap();
            let expected = minase::stats::gsprt_llr_with_hypotheses(&counts, h0, h1);
            let (llr, decision) = tail.split_once(" decision=").unwrap();
            assert!((llr.parse::<f64>().unwrap() - expected).abs() < 1e-9);
            let boundary = 19.0_f64.ln();
            let expected_decision = if expected >= boundary {
                "H1"
            } else if expected <= -boundary {
                "H0"
            } else {
                "pending"
            };
            assert_eq!(decision, expected_decision);
            last_llr = Some(llr);
            last_decision = Some(decision);
        }
        assert_eq!(
            text.lines().find_map(|line| line.strip_prefix("llr: ")),
            Some(last_llr.unwrap())
        );
        assert_eq!(
            text.lines()
                .find_map(|line| line.strip_prefix("decision: ")),
            Some(last_decision.unwrap())
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    std::fs::remove_dir_all(parent).unwrap();
}

#[test]
fn gsprt_invalid_hypotheses_fail_before_creating_a_run() {
    for (elo0, elo1) in [
        ("5", "5"),
        ("5", "-5"),
        ("NaN", "5"),
        ("-inf", "5"),
        ("0", "inf"),
    ] {
        let dir = run_directory();
        let output = small_gsprt_run(&dir, &["--elo0", elo0, "--elo1", elo1]);
        assert_eq!(output.status.code(), Some(2));
        assert!(!dir.exists());
    }
}

#[test]
fn gsprt_default_and_explicit_default_have_identical_manifest_and_output() {
    let implicit = run_directory();
    let explicit = run_directory();
    let first = small_gsprt_run(&implicit, &[]);
    let second = small_gsprt_run(&explicit, &["--elo0", "0", "--elo1", "10"]);
    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(
        std::fs::read(implicit.join("manifest.json")).unwrap(),
        std::fs::read(explicit.join("manifest.json")).unwrap()
    );
    let first = String::from_utf8(first.stdout).unwrap();
    let second = String::from_utf8(second.stdout).unwrap();
    assert!(!first.contains("hypotheses:"));
    assert_eq!(without_elapsed(&first), without_elapsed(&second));
    assert_eq!(
        json_file(&implicit.join("manifest.json"))["mode"],
        serde_json::json!({
            "kind": "gsprt", "h0_elo": 0.0, "h1_elo": 10.0, "alpha": 0.05, "beta": 0.05
        })
    );
    std::fs::remove_dir_all(implicit).unwrap();
    std::fs::remove_dir_all(explicit).unwrap();
}

#[test]
fn gsprt_resume_checks_only_explicit_hypotheses_without_changing_saved_conditions() {
    let dir = run_directory();
    // elo0だけの照合時に、新規実行の既定elo1=10と比較して拒否しないことも検証する。
    assert!(
        small_gsprt_run(&dir, &["--elo0", "20", "--elo1", "30"])
            .status
            .success()
    );
    let saved: Vec<_> = ["manifest.json", "invocations.json", "summary.json"]
        .into_iter()
        .map(|file| (dir.join(file), std::fs::read(dir.join(file)).unwrap()))
        .collect();
    for args in [
        vec![],
        vec!["gsprt", "--elo0", "20"],
        vec!["gsprt", "--elo1", "30"],
        vec!["gsprt", "--elo0", "20", "--elo1", "30"],
    ] {
        let output = resume_run(&dir, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("hypotheses: elo0=20 elo1=30\n"));
    }
    for (flag, value) in [
        ("--elo0", "0"),
        ("--elo1", "10"),
        ("--elo0", "21"),
        ("--elo1", "31"),
    ] {
        let output = resume_run(&dir, &["--target-pairs", "4", "gsprt", flag, value]);
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(flag) && error.contains("does not match the recorded value"),
            "{error}"
        );
    }
    for (path, bytes) in saved {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let output = resume_run(
        &dir,
        &[
            "--target-pairs",
            "4",
            "gsprt",
            "--elo0",
            "20",
            "--elo1",
            "30",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("pairs/00000000000000000004.json").is_file());
    std::fs::remove_dir_all(dir).unwrap();

    let elo = run_directory();
    small_run(&elo);
    let output = resume_run(&elo, &["gsprt", "--elo0", "0"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("does not match the recorded mode"));
    std::fs::remove_dir_all(elo).unwrap();
}

// match-resume-simplification.md「起動の記録」: 表示だけでは履歴を変えず、
// 実行する再開はrunnerと測定機が変わっても通知して記録する。
#[test]
fn resume_records_only_invocations_that_execute_games() {
    let dir = run_directory();
    small_run(&dir);
    let manifest = std::fs::read(dir.join("manifest.json")).unwrap();
    let value = json_file(&dir.join("manifest.json"));
    assert_eq!(value["format_version"], 5);
    assert!(value.get("runner").is_none());
    assert!(value.get("cpu").is_none());
    assert_eq!(
        value["candidate"]["identity"],
        serde_json::json!({"kind": "random"})
    );
    let mut invocations = json_file(&dir.join("invocations.json"));
    invocations[0]["runner"]["sha256"] = serde_json::json!("previous runner");
    invocations[0]["cpu"]["model"] = serde_json::json!("previous machine");
    write_json(&dir.join("invocations.json"), &invocations);
    let before = std::fs::read(dir.join("invocations.json")).unwrap();
    let summary = std::fs::read(dir.join("summary.json")).unwrap();
    let output = resume_run(&dir, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(dir.join("invocations.json")).unwrap(), before);
    assert_eq!(std::fs::read(dir.join("summary.json")).unwrap(), summary);
    for target in ["0", "1"] {
        assert!(
            !resume_run(&dir, &["--target-pairs", target])
                .status
                .success()
        );
    }
    let output = resume_run(&dir, &["--target-pairs", "2"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter(|line| line.starts_with("notice:"))
            .count(),
        1
    );
    let history = json_file(&dir.join("invocations.json"));
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert_eq!(history[1]["target_pairs"], 2);
    assert!(dir.join("pairs/00000000000000000002.json").is_file());
    assert_eq!(std::fs::read(dir.join("manifest.json")).unwrap(), manifest);
    assert!(!resume_run(&dir, &["--target-pairs", "2"]).status.success());
    assert!(resume_run(&dir, &[]).status.success());
    assert_eq!(json_file(&dir.join("invocations.json")), history);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn resume_condition_arguments_are_clap_errors() {
    for args in [
        vec!["--seed", "42"],
        vec!["--candidate", "random"],
        vec!["--baseline", "random"],
        vec!["--each", "depth=1"],
        vec!["--rules", "engine-default"],
        vec!["--max-ply", "16"],
        vec!["--response-timeout", "120"],
        vec!["--concurrency", "1"],
        vec!["--ponder"],
        vec!["--candidate-limit", "depth=1"],
        vec!["--baseline-limit", "depth=1"],
        vec!["--candidate-hash", "256"],
        vec!["--baseline-hash", "256"],
        vec!["elo", "--pairs", "1"],
        vec!["gsprt"],
        vec!["gsprt", "--max-pairs", "1"],
    ] {
        let output = resume_run(std::path::Path::new("nonexistent-run"), &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args([
            "match",
            "run",
            "--run-dir",
            "nonexistent-run",
            "--target-pairs",
            "2",
            "elo",
            "--pairs",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn resume_rejects_version_four_and_changed_engine_conditions_without_writes() {
    let dir = run_directory();
    small_run(&dir);
    let original = json_file(&dir.join("manifest.json"));
    let history = std::fs::read(dir.join("invocations.json")).unwrap();
    let summary = std::fs::read(dir.join("summary.json")).unwrap();
    for (pointer, replacement, message) in [
        (
            "/format_version",
            serde_json::json!(4),
            "unsupported manifest format version 4",
        ),
        (
            "/engine_threads/candidate",
            serde_json::json!(2),
            "Threads default differs",
        ),
        (
            "/candidate/identity",
            serde_json::json!({"kind":"command", "program":"unused", "args":[], "protocol":"usi", "working_directory":"/nonexistent-recorded-directory"}),
            "working directory differs",
        ),
    ] {
        let mut manifest = original.clone();
        *manifest.pointer_mut(pointer).unwrap() = replacement;
        write_json(&dir.join("manifest.json"), &manifest);
        let output = resume_run(&dir, &["--target-pairs", "2"]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            std::fs::read(dir.join("invocations.json")).unwrap(),
            history
        );
        assert_eq!(std::fs::read(dir.join("summary.json")).unwrap(), summary);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn resume_rejects_recorded_commit_digest_mismatch_before_engine_start() {
    let repository = run_directory();
    let clone = Command::new("git")
        .args(["clone", "--shared", "--no-checkout"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(revision.status.success());
    let hash = String::from_utf8(revision.stdout)
        .unwrap()
        .trim()
        .to_owned();
    let cache = repository.join("target/match-cache").join(&hash);
    std::fs::create_dir_all(&cache).unwrap();
    // 解決器が検証するキャッシュの署名は正しく、manifestの署名だけを変える。
    std::fs::write(cache.join("minase"), b"must never be executed").unwrap();
    let digest = minase::harness::sha256_file(&cache.join("minase")).unwrap();
    std::fs::write(cache.join("minase.sha256"), digest).unwrap();
    let dir = repository.join("run");
    small_run(&dir);
    let mut manifest = json_file(&dir.join("manifest.json"));
    manifest["candidate"]["identity"] =
        serde_json::json!({"kind":"commit", "hash":hash, "sha256":"0".repeat(64)});
    write_json(&dir.join("manifest.json"), &manifest);
    let before = std::fs::read(dir.join("invocations.json")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run", "--resume"])
        .arg(&dir)
        .args(["--target-pairs", "2"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("engine identity does not match"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(dir.join("invocations.json")).unwrap(), before);
    std::fs::remove_dir_all(repository).unwrap();
}

#[cfg(unix)]
#[test]
fn resume_uses_recorded_hypotheses_and_restores_command_defaults() {
    let parent = run_directory();
    std::fs::create_dir(&parent).unwrap();
    let engine = parent.join("engine.sh");
    let log = parent.join("commands.log");
    // 引数に空白を含む場合も、記録した配列の要素をそのまま渡す。
    std::fs::write(&engine, r#"
test "$2" = 'argument with spaces' || exit 1
while IFS= read -r line; do
    printf '%s\n' "$line" >> "$1"
    case "$line" in
        usi) printf 'option name Threads type spin default 1 min 1 max 8\noption name USI_Hash type spin default 16 min 1 max 1024\nusiok\n' ;;
        isready) echo readyok ;;
        go*) echo 'bestmove resign' ;;
        quit) exit 0 ;;
    esac
done
"#).unwrap();
    let dir = parent.join("run");
    small_run(&dir);
    let mut manifest = json_file(&dir.join("manifest.json"));
    for side in ["candidate", "baseline"] {
        manifest[side]["identity"] = serde_json::json!({
            "kind":"command", "program":"/bin/sh", "args":[engine, log, "argument with spaces"],
            "protocol":"usi", "working_directory":std::env::current_dir().unwrap()
        });
        manifest["hash_mb"][side] = serde_json::json!(16);
    }
    manifest["mode"] =
        serde_json::json!({"kind":"gsprt", "h0_elo":-5.0, "h1_elo":5.0, "alpha":0.1, "beta":0.2});
    write_json(&dir.join("manifest.json"), &manifest);
    // 欠番を再実行して両局が投了で終わるペアを保存する。
    std::fs::remove_file(dir.join("pairs/00000000000000000001.json")).unwrap();
    let output = resume_run(&dir, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("pentanomial: [0, 0, 1, 0, 0]"));
    let llr: f64 = text
        .lines()
        .find_map(|line| line.strip_prefix("llr: "))
        .unwrap()
        .parse()
        .unwrap();
    assert!(llr.abs() < 1e-9);
    assert!(
        !std::fs::read_to_string(&log)
            .unwrap()
            .contains("setoption name USI_Hash")
    );
    let before = std::fs::read(dir.join("invocations.json")).unwrap();
    assert!(resume_run(&dir, &[]).status.success());
    assert_eq!(std::fs::read(dir.join("invocations.json")).unwrap(), before);
    manifest["hash_mb"]["candidate"] = serde_json::json!(32);
    write_json(&dir.join("manifest.json"), &manifest);
    let output = resume_run(&dir, &["--target-pairs", "2"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        std::fs::read_to_string(&log)
            .unwrap()
            .contains("setoption name USI_Hash value 32")
    );
    std::fs::remove_dir_all(parent).unwrap();
}

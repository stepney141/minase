//! 自己対局測定ハーネスのプロセス境界を検査する統合テスト。

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn run_directory() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "minase-match-run-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn run_random_match(concurrency: &str, pairs: &str) -> String {
    let run_dir = run_directory();
    let output = Command::new(env!("CARGO_BIN_EXE_match_runner"))
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
    let output = Command::new(env!("CARGO_BIN_EXE_match_runner"))
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
        Command::new(env!("CARGO_BIN_EXE_match_runner"))
            .arg(operation)
            .arg(&run_dir)
            .args([
                "--seed",
                "20260828",
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
                "1",
                "elo",
                "--pairs",
                "3",
            ])
            .output()
            .expect("match_runner must start")
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
    for prefix in ["summary:", "pentanomial:", "engine_failures:"] {
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
    for extra in [
        vec!["--each", "depth=4"],
        vec!["--each", "nodes=1000"],
        vec!["--each", "time=1000+10", "--baseline-limit", "depth=1"],
        vec![
            "--each",
            "time=1000+10",
            "--baseline",
            "cecp:missing-engine",
        ],
    ] {
        let run_dir = run_directory();
        let output = Command::new(env!("CARGO_BIN_EXE_match_runner"))
            .arg("--run-dir")
            .arg(&run_dir)
            .arg("--ponder")
            .args(extra)
            .args(["--concurrency", "2", "elo", "--pairs", "1"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--ponder"));
        assert!(!run_dir.exists());
    }
}

// D8-HARN-21/26（ponder.md保存形式と再開）。再開集計は既存ペアを含む。
#[test]
fn ponder_resume_preserves_pairs_and_replays_all_counts() {
    let run_dir = run_directory();
    let run = |operation, pairs, ponder| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_match_runner"));
        command.arg(operation).arg(&run_dir).args([
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
        command.args(["elo", "--pairs", pairs]).output().unwrap()
    };
    let first = run("--run-dir", "1", true);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let pair_dir = run_dir.join("pairs");
    let first_path = std::fs::read_dir(&pair_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let before = std::fs::read(&first_path).unwrap();
    let resumed = run("--resume", "2", true);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(std::fs::read(first_path).unwrap(), before);
    let mut moves = [0_u64; 2];
    for path in std::fs::read_dir(&pair_dir).unwrap() {
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.unwrap().path()).unwrap()).unwrap();
        for game in record["games"].as_array().unwrap() {
            for turn in game["turns"].as_array().unwrap() {
                assert!(turn.as_object().unwrap().contains_key("ponder"));
                assert!(turn["ponder"].is_null());
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
    assert!(String::from_utf8_lossy(&mismatch.stderr).contains("manifest does not match"));
    std::fs::remove_dir_all(run_dir).unwrap();
}

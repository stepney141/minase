//! チェックサムの記録、再開、およびワーカーからの異常終了の契約。

use super::*;
use std::path::Path;
use std::process::Output;

const A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const B: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

fn network_file(parent: &Path, value: Option<&str>) {
    std::fs::write(
        parent.join("network.txt"),
        match value {
            Some(value) => format!("info string network_sha256 {value}\n"),
            None => String::new(),
        },
    )
    .unwrap();
}

fn engine(parent: &Path, on_ready: &str) -> String {
    let script = parent.join("network-engine.sh");
    std::fs::write(&script, format!(r#"
model=
while IFS= read -r line; do
    printf '%s %s\n' "$$" "$line" >> "$1/commands.log"
    case "$line" in
        usi) printf 'option name Threads type spin default 1 min 1 max 1\noption name USI_Hash type spin default 256 min 1 max 65536\noption name NnModel type string default <empty>\nusiok\n' ;;
        'setoption name NnModel value '*) model="${{line#setoption name NnModel value }}" ;;
        isready)
            [ -n "$model" ] || exit 31
            {on_ready}
            echo readyok ;;
        go*) echo 'bestmove resign' ;;
    esac
done
"#)).unwrap();
    format!("sh {} {}", script.display(), parent.display())
}

fn start(parent: &Path, spec: &str, side: &str, pairs: &str, concurrency: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["match", "run", "--rules", "JCSA", "--run-dir"])
        .arg(parent.join("run"))
        .args([&format!("--{side}"), spec, &format!("--{side}-option")])
        .arg(format!("NnModel={}", parent.join("network.txt").display()))
        .args([
            "--each",
            "nodes=1",
            "--seed",
            "20260811",
            "--max-ply",
            "100",
            "--response-timeout",
            "5",
            "--concurrency",
            concurrency,
            "elo",
            "--pairs",
            pairs,
        ])
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn network_checksums_are_recorded_and_resume_checks_values_and_null_for_both_sides() {
    for side in ["candidate", "baseline"] {
        for (before, after) in [
            (Some(A), Some(A)),
            (None, None),
            (Some(A), Some(B)),
            (None, Some(A)),
            (Some(A), None),
        ] {
            let parent = run_directory();
            std::fs::create_dir(&parent).unwrap();
            network_file(&parent, before);
            let spec = engine(&parent, "cat \"$model\"");
            assert_success(&start(&parent, &spec, side, "1", "1"));
            let run = parent.join("run");
            let manifest = json_file(&run.join("manifest.json"));
            assert_eq!(manifest["format_version"], 8);
            assert_eq!(manifest[side]["network_sha256"], serde_json::json!(before));
            for engine in ["candidate", "baseline"] {
                assert!(manifest[engine].get("network_sha256").is_some());
                if engine != side {
                    assert!(manifest[engine]["network_sha256"].is_null());
                }
            }
            let history = std::fs::read(run.join("invocations.json")).unwrap();
            let pair = std::fs::read(run.join("pairs/00000000000000000001.json")).unwrap();
            network_file(&parent, after);
            std::fs::write(parent.join("commands.log"), "").unwrap();
            let output = resume_run(&run, &["--target-pairs", "2"]);
            if before == after {
                assert_success(&output);
                assert!(run.join("pairs/00000000000000000002.json").exists());
            } else {
                assert_eq!(output.status.code(), Some(1));
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(stderr.contains(&format!("{side} engine")), "{stderr}");
                assert!(stderr.contains("network_sha256 mismatch"), "{stderr}");
                assert!(
                    stderr.contains(&format!("recorded {}", before.unwrap_or("null"))),
                    "{stderr}"
                );
                assert!(
                    stderr.contains(&format!("received {}", after.unwrap_or("null"))),
                    "{stderr}"
                );
                assert_eq!(
                    std::fs::read(run.join("invocations.json")).unwrap(),
                    history
                );
                assert!(!run.join("pairs/00000000000000000002.json").exists());
            }
            let commands = std::fs::read_to_string(parent.join("commands.log")).unwrap();
            let expected = format!(
                "setoption name NnModel value {}",
                parent.join("network.txt").display()
            );
            let mut configured = std::collections::BTreeSet::new();
            let mut ready_count = 0;
            for line in commands.lines() {
                let (process, command) = line.split_once(' ').unwrap();
                if command == expected {
                    configured.insert(process);
                }
                if command == "isready" {
                    assert!(configured.contains(process));
                    ready_count += 1;
                }
            }
            assert!(ready_count >= 1);
            assert_eq!(
                std::fs::read(run.join("pairs/00000000000000000001.json")).unwrap(),
                pair
            );
            assert_eq!(json_file(&run.join("manifest.json")), manifest);
        }
    }
}

#[test]
fn network_failure_mid_run_keeps_completed_pairs_and_resume_replays_only_missing_pairs() {
    let parent = run_directory();
    std::fs::create_dir(&parent).unwrap();
    network_file(&parent, Some(A));
    std::fs::create_dir(parent.join("starts")).unwrap();
    let spec = engine(
        &parent,
        &format!(
            r#"
number=1
while ! mkdir "$1/starts/$number" 2>/dev/null; do number=$((number + 1)); done
if [ "$number" -ge 4 ] && [ ! -e "$1/repaired" ]; then
    printf 'info string network_sha256 {B}\n'
else
    cat "$model"
fi
"#
        ),
    );
    let output = start(&parent, &spec, "candidate", "3", "1");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "candidate engine: network_sha256 mismatch: recorded {A}, received {B}"
        )),
        "{stderr}"
    );
    assert!(!stderr.contains("panicked"));
    let run = parent.join("run");
    let pair_path = run.join("pairs/00000000000000000001.json");
    let saved = std::fs::read(&pair_path).unwrap();
    assert_eq!(std::fs::read_dir(run.join("pairs")).unwrap().count(), 1);
    std::fs::write(parent.join("repaired"), "").unwrap();
    let output = resume_run(&run, &[]);
    assert_success(&output);
    assert_eq!(std::fs::read(pair_path).unwrap(), saved);
    assert_eq!(std::fs::read_dir(run.join("pairs")).unwrap().count(), 3);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pair 1: loaded from saved record"));
    assert!(stdout.contains("illegal_moves=0 crashes=0 timeouts=0"));
    assert_eq!(json_file(&run.join("summary.json"))["interrupted"], true);
}

// 2ワーカーをFIFOの応答で同期し、終了待ちで回収する2個目の原因も保持する。
#[test]
fn network_failures_from_concurrent_workers_keep_causes_and_saved_pairs() {
    let parent = run_directory();
    std::fs::create_dir(&parent).unwrap();
    network_file(&parent, Some(A));
    std::fs::create_dir(parent.join("starts")).unwrap();
    for fifo in ["first", "second"] {
        assert!(
            Command::new("mkfifo")
                .arg(parent.join(fifo))
                .status()
                .unwrap()
                .success()
        );
    }
    let spec = engine(
        &parent,
        &format!(
            r#"
number=1
while ! mkdir "$1/starts/$number" 2>/dev/null; do number=$((number + 1)); done
if [ "$number" = 5 ]; then
    echo started > "$1/first"
    IFS= read -r answer < "$1/second"
    printf 'info string network_sha256 {B}\n'
elif [ "$number" = 6 ]; then
    IFS= read -r answer < "$1/first"
    echo started > "$1/second"
    printf 'info string network_sha256 {B}\n'
else
    cat "$model"
fi
"#
        ),
    );
    assert_success(&start(&parent, &spec, "candidate", "1", "2"));
    let run = parent.join("run");
    let saved = std::fs::read(run.join("pairs/00000000000000000001.json")).unwrap();
    let output = resume_run(&run, &["--target-pairs", "4"]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = format!("candidate engine: network_sha256 mismatch: recorded {A}, received {B}");
    assert_eq!(stderr.matches(&reason).count(), 2, "{stderr}");
    assert!(!stderr.contains("panicked"));
    assert_eq!(std::fs::read_dir(parent.join("starts")).unwrap().count(), 6);
    assert_eq!(std::fs::read_dir(run.join("pairs")).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(run.join("pairs/00000000000000000001.json")).unwrap(),
        saved
    );
    let output = resume_run(&run, &[]);
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("pair 1: loaded from saved record"));
    assert_eq!(std::fs::read_dir(run.join("pairs")).unwrap().count(), 4);
    assert_eq!(
        std::fs::read(run.join("pairs/00000000000000000001.json")).unwrap(),
        saved
    );
}

#[test]
fn network_checksum_field_is_required_when_resuming_and_reporting() {
    let run = run_directory();
    small_run(&run);
    let manifest = json_file(&run.join("manifest.json"));
    let history = std::fs::read(run.join("invocations.json")).unwrap();
    for side in ["candidate", "baseline"] {
        let mut modified = manifest.clone();
        modified[side]
            .as_object_mut()
            .unwrap()
            .remove("network_sha256");
        write_json(&run.join("manifest.json"), &modified);
        for output in [
            resume_run(&run, &[]),
            Command::new(env!("CARGO_BIN_EXE_minase"))
                .args(["match", "report", "--run-dir"])
                .arg(&run)
                .output()
                .unwrap(),
        ] {
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("missing field `network_sha256`")
            );
        }
        assert_eq!(
            std::fs::read(run.join("invocations.json")).unwrap(),
            history
        );
    }
}

#[test]
fn network_probe_format_errors_name_the_side_and_do_not_create_a_measurement() {
    for side in ["candidate", "baseline"] {
        for lines in [
            format!("info string network_sha256 {}\n", "A".repeat(64)),
            format!("info string network_sha256 {A}\ninfo string network_sha256 {A}\n"),
        ] {
            let parent = run_directory();
            std::fs::create_dir(&parent).unwrap();
            std::fs::write(parent.join("network.txt"), lines).unwrap();
            let spec = engine(&parent, "cat \"$model\"");
            let output = start(&parent, &spec, side, "1", "1");
            assert_eq!(output.status.code(), Some(1));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(&format!("{side} engine")), "{stderr}");
            assert!(stderr.contains("network_sha256"));
            assert!(stderr.contains("recorded null"));
            assert!(stderr.contains("received"));
            assert!(!parent.join("run").exists());
        }
    }
}

use std::process::Command;

fn run_random_match() -> String {
    let expected_random = std::path::Path::new(env!("CARGO_BIN_EXE_match_runner"))
        .with_file_name(format!("usi_random{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(
        expected_random,
        std::path::Path::new(env!("CARGO_BIN_EXE_usi_random"))
    );
    let output = Command::new(env!("CARGO_BIN_EXE_match_runner"))
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
    String::from_utf8(output.stdout).expect("match_runner output must be UTF-8")
}

fn without_elapsed(output: &str) -> String {
    output
        .lines()
        .filter(|line| !line.starts_with("elapsed:"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn random_usi_match_is_reproducible_for_the_same_seed() {
    let first = run_random_match();
    let second = run_random_match();

    assert!(first.contains("summary: mode=elo pairs=1 valid_pairs=1 discarded_pairs=0"));
    assert!(first.contains("engine_failures: illegal_moves=0 crashes=0 timeouts=0"));
    assert_eq!(without_elapsed(&first), without_elapsed(&second));
}

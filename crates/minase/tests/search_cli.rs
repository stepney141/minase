//! 探索方式の起動契約を、ONNX Runtimeを読み込まずにプロセス境界で検査する。

use std::io::Write;
use std::process::{Command, Output, Stdio};

fn session(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["--protocol", "usi", "--search", "mcts"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn removed_model_argument_is_a_clap_error_and_cecp_stays_unsupported() {
    // 削除済みの名前を組み立て、現行の引数の利用箇所と区別する。
    let removed = format!("--{}-model", "nn");
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["--protocol", "usi", "--search", "mcts", &removed, "x.onnx"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument"));

    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["--protocol", "cecp", "--search", "mcts"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: --search mcts requires --protocol usi; CECP is not supported\n"
    );
}

#[test]
fn required_model_and_rules_errors_end_the_process_without_readyok() {
    for (input, reason) in [
        (
            "setoption name RuleSet value JCSA\nisready\n",
            "required option NnModel is not set",
        ),
        ("isready\n", "required option RuleSet is not set"),
    ] {
        let output = session(input);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("info string error: {reason}\n")
        );
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("error: {reason}\n")
        );
    }
}

#[test]
fn mcts_initialization_error_reaches_process_exit_without_readyok() {
    struct NetworkFile(std::path::PathBuf);
    impl Drop for NetworkFile {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).unwrap();
        }
    }
    let path = NetworkFile(std::env::temp_dir().join(format!(
        "minase-search-cli-cache-error-{}.onnx",
        std::process::id()
    )));
    std::fs::File::create_new(&path.0)
        .unwrap()
        .write_all(b"not an ONNX model")
        .unwrap();
    let output = session(&format!(
        "setoption name RuleSet value JCSA\nsetoption name NnModel value {}\nsetoption name NnCacheDir value <empty>\nisready\nusi\n",
        path.0.display()
    ));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "info string error: NnCacheDir must be set for NnBackend tensorrt\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: NnCacheDir must be set for NnBackend tensorrt\n"
    );
}

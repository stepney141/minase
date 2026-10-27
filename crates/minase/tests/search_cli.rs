//! 探索方式の起動契約を、ONNX Runtimeを読み込まずにプロセス境界で検査する。

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn invalid_search_arguments_exit_one_on_stderr_before_session_start() {
    for (protocol, args, reason) in [
        (
            "usi",
            vec!["--nn-model", "/nonexistent.onnx"],
            "--nn-model requires --search mcts",
        ),
        (
            "usi",
            vec!["--search", "alphabeta", "--nn-model", "/nonexistent.onnx"],
            "--nn-model requires --search mcts",
        ),
        (
            "usi",
            vec!["--search", "mcts"],
            "--search mcts requires --nn-model",
        ),
        (
            "cecp",
            vec!["--search", "mcts", "--nn-model", "/nonexistent.onnx"],
            "--search mcts requires --protocol usi; CECP is not supported",
        ),
        (
            "usi",
            vec!["--search", "mcts", "--nn-model", "/nonexistent.onnx"],
            "cannot read network /nonexistent.onnx:",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(["--protocol", protocol, "--rules", "engine-default"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with(&format!("error: {reason}")), "{stderr}");
        assert_eq!(stderr.lines().count(), 1);
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
    let path = NetworkFile(
        std::env::temp_dir().join(format!("minase-search-cli-{}.onnx", std::process::id())),
    );
    // 起動時にはバイト列だけを読み、空のキャッシュ先をネットの検証より先に拒否する。
    std::fs::File::create_new(&path.0)
        .unwrap()
        .write_all(b"not an ONNX model")
        .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args([
            "--protocol",
            "usi",
            "--rules",
            "engine-default",
            "--search",
            "mcts",
            "--nn-model",
        ])
        .arg(&path.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"setoption name NnCacheDir value <empty>\nisready\nusi\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
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

//! 規則未設定の通知と終了コードを実行ファイルの境界で検査する。

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn missing_ruleset_flushes_the_protocol_error_and_exits_one() {
    for (args, input, error) in [
        (
            vec!["--protocol", "usi"],
            "isready\n",
            "info string error: required option RuleSet is not set",
        ),
        (
            vec!["--protocol", "cecp"],
            "xboard\nprotover 2\nnew\n",
            "Error (required option RuleSet is not set): new",
        ),
        (
            vec!["dev", "usi-random"],
            "isready\n",
            "info string error: required option RuleSet is not set",
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_minase"))
            .args(args)
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
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(stdout.lines().last(), Some(error));
        assert!(!stdout.contains("readyok"));
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("required option RuleSet is not set")
        );
    }
}

#[test]
fn engine_startup_rejects_the_removed_rules_argument() {
    let output = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["--protocol", "usi", "--rules", "JCSA"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("unexpected argument '--rules'")
    );
}

//! 固定USIオプションの引数検証は、エンジンを起動する前にclapの終了コード2で拒否する。
use std::process::Command;

#[test]
fn fixed_options_reject_duplicates_tuning_names_and_resume_overrides() {
    for (options, expected) in [
        (vec!["EvalFile=a", "evalfile=b"], "duplicate fixed option"),
        (vec!["Tune_X=1"], "must not start with Tune_"),
        (vec!["tUnE_X=1"], "must not start with Tune_"),
        (vec!["bad"], "NAME=VALUE"),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_minase"));
        command.args([
            "spsa",
            "--run-dir",
            "unused",
            "--seed",
            "1",
            "--engine",
            "nonexistent-engine",
            "--params",
            "unused.txt",
            "--rules",
            "JCSA",
            "--each",
            "nodes=1",
            "--concurrency",
            "1",
            "--iterations",
            "1",
            "--pairs-per-iteration",
            "1",
        ]);
        for option in options {
            command.args(["--option", option]);
        }
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
    }
    let result = Command::new(env!("CARGO_BIN_EXE_minase"))
        .args(["spsa", "--resume", "unused", "--option", "EvalFile=a"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("cannot be used with"));
}

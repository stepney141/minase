//! 学習局面の生成コミットの取得と検査。

use std::io;
use std::process::Command;

use super::invalid_data;

/// gitサブコマンドを実行し、末尾の改行を除いた標準出力を返す。
pub fn git_output(arguments: &[&str]) -> io::Result<String> {
    let output = Command::new("git").args(arguments).output()?;
    if !output.status.success() {
        return Err(invalid_data(format!(
            "git {} failed with status {}: {}",
            arguments.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|error| invalid_data(format!("git output is not UTF-8: {error}")))
}

/// 生成コミットが40桁の16進ASCIIであることを検査する。
pub fn validate_commit_hash(commit: &str) -> io::Result<()> {
    if commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid_data(format!(
            "git rev-parse HEAD returned an invalid full hash: {commit:?}"
        )))
    }
}

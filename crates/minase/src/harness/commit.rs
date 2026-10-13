//! コミットのビルドと実行ファイルのキャッシュ。
//!
//! # キャッシュの付随ファイル
//!
//! `target/match-cache/<キー>/minase.sha256`は、実行ファイル全体のSHA-256を
//! 小文字の16進数字64文字とLFで保存するテキストで、版欄はない。
//! 読み手は前後の空白を除き、再計算した小文字16進文字列との一致を調べる。
//! `.build.lock`は新規作成時0バイトで、内容を読み書きせず、OSの排他ロックに使う。
//! ロックを解放してもファイルは残り、既存ファイルを切り詰めない。
//! ビルド中の`.source-<キー>-<プロセス番号>.tar`は`git archive --format=tar`による
//! ソースの一時アーカイブであり、展開後に削除する。

use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io, process};

/// Gitコマンドの失敗内容を標準出力と標準エラーを含めて返す。
fn command_error(action: &str, output: &process::Output) -> io::Error {
    let mut message = format!("{action} failed with {}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.trim().is_empty() {
        write!(message, "\nstdout:\n{}", stdout.trim_end()).expect("writing to String cannot fail");
    }
    if !stderr.trim().is_empty() {
        write!(message, "\nstderr:\n{}", stderr.trim_end()).expect("writing to String cannot fail");
    }
    io::Error::other(message)
}

/// リビジョンをコミットの完全ハッシュへ正規化する。
fn normalize_commit(repository: &Path, revision: &str) -> io::Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{revision}^{{commit}}")])
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(command_error("git rev-parse --verify", &output));
    }
    let hash = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(hash.trim().to_owned())
}

/// コミットをビルドし、実行ファイル、完全ハッシュ、SHA-256を返す。
///
/// カレントディレクトリをリポジトリとし、`git rev-parse --verify <rev>^{commit}`で
/// リビジョンを完全ハッシュへ正規化する。本リポジトリでは40桁である。
/// `feature`が`None`なら完全ハッシュをキーにする。
/// 指定時はASCII英数字、`_`、`-`からなる空でない1つの名前を受け付け、
/// `<完全ハッシュ>-<feature名>`をキーにする。
///
/// キャッシュは`target/match-cache/<キー>/minase`と`minase.sha256`である。
/// 実行ファイル名にはOS固有の接尾辞を付け、検査和は拡張子を`sha256`に替えたパスへ置く。
/// 実行ファイルのSHA-256が記録と一致するときだけ命中とし、欠損や不一致は再ビルドする。
/// `git archive`の展開先で`cargo build --release --bin minase`を実行するため、
/// 作業ツリーの未コミットの変更は含まない。指定されたfeatureはビルド時に有効にする。
/// 同じキーのビルドは`.build.lock`の排他ロックで直列化する。
/// 成果物と検査和を一時ファイルへ書いて同期し、改名した後にディレクトリも同期する。
/// 理由は`docs/adr/compare-commits-as-separate-engine-processes.md`にある。
pub fn resolve_commit(
    revision: &str,
    feature: Option<&str>,
) -> io::Result<(PathBuf, String, String)> {
    let repository = std::env::current_dir()?;
    let report = |message: String| {
        if feature.is_some() {
            eprintln!("{message}");
        } else {
            println!("{message}");
        }
    };
    report(format!("resolving commit {revision}..."));
    let hash = normalize_commit(&repository, revision)?;
    let cache_root = repository.join("target/match-cache");
    let binary_name = format!("minase{}", std::env::consts::EXE_SUFFIX);
    let key = match feature {
        None => hash.clone(),
        Some(feature) => {
            if feature.is_empty()
                || !feature
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "feature must be a nonempty name containing ASCII letters, digits, underscores, or hyphens",
                ));
            }
            format!("{hash}-{feature}")
        }
    };
    let cache_path = cache_root.join(&key).join(&binary_name);
    let cache_directory = cache_path.parent().expect("cache path has a parent");
    fs::create_dir_all(cache_directory)?;
    let cache_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(cache_directory.join(".build.lock"))?;
    FileExt::lock_exclusive(&cache_lock)?;
    let digest_path = cache_path.with_extension("sha256");
    let cached_digest = fs::read_to_string(&digest_path).ok();
    if cache_path.is_file()
        && cached_digest.as_deref().is_some_and(|digest| {
            sha256_file(&cache_path).is_ok_and(|actual| actual == digest.trim())
        })
    {
        let sha256 = cached_digest
            .expect("the preceding condition requires a digest")
            .trim()
            .to_owned();
        report(format!("cached: {}", cache_path.display()));
        return Ok((cache_path, hash, sha256));
    }

    report(format!("building commit {hash}..."));
    fs::create_dir_all(&cache_root)?;
    let source_tree = cache_root.join(format!(".source-{key}-{}", process::id()));
    let archive_path = cache_root.join(format!(".source-{key}-{}.tar", process::id()));
    fs::create_dir(&source_tree)?;
    let archive_output = Command::new("git")
        .args(["archive", "--format=tar", "--output"])
        .arg(&archive_path)
        .arg(&hash)
        .current_dir(&repository)
        .output()?;
    let build_result = if !archive_output.status.success() {
        Err(command_error("git archive", &archive_output))
    } else {
        let extract_output = Command::new("tar")
            .args(["-xf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&source_tree)
            .output()?;
        if !extract_output.status.success() {
            Err(command_error("tar -xf", &extract_output))
        } else {
            (|| {
                let mut command = Command::new("cargo");
                command.args(["build", "--release", "--bin", "minase"]);
                if let Some(feature) = feature {
                    command.args(["--features", feature]);
                }
                let output = command.current_dir(&source_tree).output()?;
                if !output.status.success() {
                    return Err(command_error("cargo build --release --bin minase", &output));
                }
                let temporary_binary =
                    cache_directory.join(format!(".{binary_name}.{}.tmp", process::id()));
                let temporary_digest =
                    cache_directory.join(format!(".{binary_name}.sha256.{}.tmp", process::id()));
                let source = source_tree.join("target/release").join(&binary_name);
                let install_result = (|| {
                    fs::copy(&source, &temporary_binary)?;
                    File::open(&temporary_binary)?.sync_all()?;
                    let digest = sha256_file(&temporary_binary)?;
                    fs::write(&temporary_digest, format!("{digest}\n"))?;
                    File::open(&temporary_digest)?.sync_all()?;
                    fs::rename(&temporary_binary, &cache_path)?;
                    fs::rename(&temporary_digest, &digest_path)?;
                    File::open(cache_directory)?.sync_all()?;
                    Ok::<_, io::Error>(())
                })();
                if install_result.is_err() {
                    let _ = fs::remove_file(&temporary_binary);
                    let _ = fs::remove_file(&temporary_digest);
                }
                install_result?;
                Ok(())
            })()
        }
    };
    let archive_remove_result = match fs::remove_file(&archive_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    let source_remove_result = fs::remove_dir_all(&source_tree);
    build_result?;
    archive_remove_result?;
    source_remove_result?;
    report(format!("cached: {}", cache_path.display()));
    let sha256 = sha256_file(&cache_path)?;
    Ok((cache_path, hash, sha256))
}

/// ファイル全体のSHA-256を小文字16進数で返す。
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 規範文書に明文がないため、不明リビジョンの解決が対局実行前に失敗することを実装契約とする。
    // 診断文言は契約ではないため種別(Err)だけを検証する。
    #[test]
    fn unknown_commit_revision_fails_resolution() {
        assert!(
            normalize_commit(
                Path::new(env!("CARGO_MANIFEST_DIR")),
                "definitely-not-a-minase-commit",
            )
            .is_err()
        );
    }
}

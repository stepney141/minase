//! PGOの生成条件とプロファイルを照合し、古いプロファイルの使用を拒否する。

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

/// 読み取り対象をCargoに通知し、読めないファイルを診断へ追加する。
fn read_file(path: &Path, errors: &mut Vec<String>) -> Option<Vec<u8>> {
    println!("cargo:rerun-if-changed={}", path.display());
    match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            errors.push(format!("{}: {error}", path.display()));
            None
        }
    }
}

/// ソースの追加と削除も検出できるように、各ディレクトリを監視する。
fn source_paths(directory: &Path, paths: &mut Vec<String>, errors: &mut Vec<String>) {
    println!("cargo:rerun-if-changed={}", directory.display());
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("{}: {error}", directory.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("{}: {error}", directory.display()));
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            source_paths(&path, paths, errors);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
            let name = path.to_string_lossy().replace('\\', "/");
            if name.ends_with(".rs")
                && !name.contains("/tests/")
                && !name.ends_with("_tests.rs")
                && name != "src/test_util.rs"
            {
                paths.push(name);
            }
        }
    }
}

/// 指定した相対パスとSHA-256の対応を作る。不足は診断に残す。
fn hashes(paths: &[impl AsRef<str>], errors: &mut Vec<String>) -> BTreeMap<String, String> {
    paths
        .iter()
        .filter_map(|path| {
            let name = path.as_ref();
            read_file(Path::new(name), errors)
                .map(|bytes| (name.to_owned(), format!("{:x}", Sha256::digest(bytes))))
        })
        .collect()
}

/// 現在のソース、生成条件、プロファイルを同じ手順で記録する。
fn current_manifest(errors: &mut Vec<String>) -> Value {
    let mut sources = Vec::new();
    source_paths(Path::new("src"), &mut sources, errors);
    let toolchain = env::var_os("RUSTC")
        .ok_or_else(|| "RUSTC is not set".to_owned())
        .and_then(|rustc| {
            println!("cargo:rerun-if-changed={}", Path::new(&rustc).display());
            let output = Command::new(rustc)
                .arg("-Vv")
                .output()
                .map_err(|error| format!("rustc -Vv: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "rustc -Vv: {}: {}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            String::from_utf8(output.stdout).map_err(|error| format!("rustc -Vv: {error}"))
        });
    let toolchain = match toolchain {
        Ok(version) => Some(version),
        Err(error) => {
            errors.push(error);
            None
        }
    };
    let profile = read_file(Path::new("pgo/minase.profdata"), errors).map(
        |bytes| json!({"sha256": format!("{:x}", Sha256::digest(&bytes)), "bytes": bytes.len()}),
    );
    json!({
        "sources": hashes(&sources, errors),
        "data": hashes(&["nets/pst.bin", "nets/pst-init.bin"], errors),
        "dependencies": hashes(&["Cargo.toml", "Cargo.lock"], errors),
        "toolchain": toolchain,
        "apply": hashes(&[".cargo/config.toml", "scripts/pgo_rustc.py"], errors),
        "training": hashes(&["scripts/pgo_profile.py", "pgo/training.json"], errors),
        "profile": profile,
    })
}

/// キー集合も比較し、不足、追加、値の不一致をすべて列挙する。
fn differences(expected: &Value, actual: &Value, path: &str, errors: &mut Vec<String>) {
    if let (Some(expected), Some(actual)) = (expected.as_object(), actual.as_object()) {
        let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).collect();
        for key in keys {
            let name = format!("{path}/{key}");
            match (expected.get(key), actual.get(key)) {
                (Some(expected), Some(actual)) => differences(expected, actual, &name, errors),
                (Some(_), None) => errors.push(format!("{name}: missing")),
                (None, Some(_)) => errors.push(format!("{name}: added")),
                (None, None) => unreachable!("キーはいずれかの記録に存在する"),
            }
        }
    } else if expected != actual {
        errors.push(format!(
            "{path}: mismatch (recorded={expected}, current={actual})"
        ));
    }
}

/// 診断をまとめて表示し、再生成するまでビルドを停止する。
fn fail(errors: &[String]) -> ! {
    panic!(
        "PGO validation failed:\n{}\nRun python3 scripts/pgo_profile.py to regenerate the profile.",
        errors.join("\n")
    );
}

/// releaseビルドでは検証し、生成時に指定された場合だけ記録を書き出す。
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for variable in [
        "PROFILE",
        "MINASE_PGO_GENERATE",
        "MINASE_PGO_WRITE_MANIFEST",
        "RUSTC",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    if env::var("PROFILE").as_deref() != Ok("release")
        || env::var_os("MINASE_PGO_GENERATE").is_some()
    {
        return;
    }

    let mut errors = Vec::new();
    let current = current_manifest(&mut errors);
    let manifest_path = Path::new("pgo/manifest.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    if env::var("MINASE_PGO_WRITE_MANIFEST").as_deref() == Ok("1") {
        if !errors.is_empty() {
            fail(&errors);
        }
        let result = serde_json::to_string_pretty(&current)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                fs::write(manifest_path, text + "\n").map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            fail(&[format!("{}: {error}", manifest_path.display())]);
        }
    } else {
        if let Some(bytes) = read_file(manifest_path, &mut errors) {
            match serde_json::from_slice(&bytes) {
                Ok(expected) => differences(&expected, &current, "manifest", &mut errors),
                Err(error) => errors.push(format!("{}: {error}", manifest_path.display())),
            }
        }
        if !errors.is_empty() {
            fail(&errors);
        }
    }
}

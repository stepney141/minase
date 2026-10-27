//! 調整用ビルドのエンジンの指定と解決。

use minase::harness::*;
use std::{fs, io};

pub(super) fn tuning_spec(text: &str) -> Result<PlayerSpec, String> {
    let spec = parse_player_spec(text)?;
    if matches!(spec.kind, PlayerKind::Random | PlayerKind::Cecp { .. }) {
        return Err(
            "SPSA requires a USI tuning engine; random and cecp: are not supported".to_owned(),
        );
    }
    Ok(spec)
}

pub(super) fn resolve_engine(
    spec: PlayerSpec,
    each: SearchLimit,
    rules: &str,
) -> io::Result<PlayerConfig> {
    if let PlayerKind::Commit(revision) = &spec.kind {
        let (path, hash, sha256) = resolve_commit(revision, Some("tuning"))?;
        Ok(PlayerConfig {
            text: spec.text,
            identity: EngineIdentity::Commit {
                hash,
                sha256,
                args: Vec::new(),
            },
            path,
            args: vec![
                "--protocol".to_owned(),
                "usi".to_owned(),
                "--rules".to_owned(),
                rules.to_owned(),
            ],
            protocol: Protocol::Usi,
            is_random: false,
            limit: each,
            hash_mb: None,
            rules_source: rules.to_owned(),
            options: Vec::new(),
        })
    } else {
        let mut player = resolve_player(spec, each, None, rules, Vec::new(), None)?;
        // 裸のコマンド名は起動時と同じPATHで解決し、ハッシュ対象を固定する。
        if player.path.components().count() == 1 {
            let path = std::env::var_os("PATH")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is not set"))?;
            player.path = std::env::split_paths(&path)
                .map(|p| p.join(&player.path))
                .find(|p| p.is_file())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "engine command is not in PATH")
                })?;
        }
        player.path = fs::canonicalize(&player.path)?;
        Ok(player)
    }
}

/// 保存された条件で調整エンジンを復元し、コミットまたは実行ファイルの同一性を検査する。
pub(super) fn restore_engine(manifest: &super::storage::Manifest) -> io::Result<PlayerConfig> {
    let spec = restore_player_spec(&manifest.engine)?;
    let each = parse_search_limit(&manifest.each)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let player = resolve_engine(spec, each, &manifest.rules_source)?;
    let sha256 = sha256_file(&player.path)?;
    check_engine_identity(manifest, &player.identity, &sha256)?;
    if matches!(manifest.engine, EngineIdentity::Commit { .. }) && sha256 != manifest.engine_sha256
    {
        eprintln!(
            "notice: engine SHA-256 differs from the session start: recorded={}, current={sha256}",
            manifest.engine_sha256
        );
    }
    Ok(player)
}

/// コミット指定は完全ハッシュ、それ以外は開始時の実行ファイルのSHA-256で照合する。
pub(super) fn check_engine_identity(
    manifest: &super::storage::Manifest,
    identity: &EngineIdentity,
    sha256: &str,
) -> Result<(), ResumeError> {
    let matches = match &manifest.engine {
        EngineIdentity::Commit { hash, args, .. } => {
            matches!(identity, EngineIdentity::Commit { hash: resolved, args: resolved_args, .. } if resolved == hash && resolved_args == args)
        }
        _ => sha256 == manifest.engine_sha256,
    };
    if !matches {
        return Err(ResumeError::EngineIdentityMismatch);
    }
    Ok(())
}

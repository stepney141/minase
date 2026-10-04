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
            identity: EngineIdentity::Commit { hash, sha256 },
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
        let mut player = resolve_player(spec, each, None, rules, Vec::new())?;
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

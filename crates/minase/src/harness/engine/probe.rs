//! 対局前のエンジンの既定値とオプションの確認。

use crate::harness::engine::EngineDefaults;
use crate::harness::engine::cecp::CECP_MEMORY_MB;
use crate::harness::engine::process::EngineProcess;
use crate::harness::failure::EngineFailure;
use crate::harness::player::{PlayerConfig, Protocol};
use crate::harness::records::EngineIdentity;
use std::io;
use std::time::Duration;

/// 1エンジンを事前起動し、実際のプロトコル応答から既定資源を得る。
pub fn probe_engine_defaults(
    player: &PlayerConfig,
    timeout: Duration,
) -> io::Result<EngineDefaults> {
    if player.is_random {
        return Ok(EngineDefaults {
            threads: Some(1),
            hash_mb: None,
        });
    }
    if player.protocol == Protocol::Cecp {
        return Ok(EngineDefaults {
            threads: None,
            hash_mb: Some(CECP_MEMORY_MB),
        });
    }
    let mut process = EngineProcess::spawn(player, timeout).map_err(|_| {
        io::Error::other(format!(
            "failed to start {} for USI resource probe",
            player.text
        ))
    })?;
    let defaults = process.read_usi_defaults().map_err(|failure| {
        io::Error::other(format!(
            "failed to read USI resource defaults from {}: {failure:?}",
            player.text
        ))
    })?;
    require_commit_threads(player, defaults)
}

/// `commit:`のエンジンに`Threads`の既定値を要求する。
///
/// 同時対局数の自動計算に使うので`Threads`は必須とし、`USI_Hash`は宣言しない方式
/// （`--search mcts`）があるので要求しない。
fn require_commit_threads(
    player: &PlayerConfig,
    defaults: EngineDefaults,
) -> io::Result<EngineDefaults> {
    if matches!(player.identity, EngineIdentity::Commit { .. }) && defaults.threads.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "commit engine {} did not report a Threads default",
                player.text
            ),
        ));
    }
    Ok(defaults)
}

/// USI握手で宣言されたoption行を、期限つきの既存通信経路で取得する。
pub fn probe_usi_options(
    player: &PlayerConfig,
    timeout: Duration,
) -> Result<Vec<String>, EngineFailure> {
    let mut process = EngineProcess::spawn(player, timeout)?;
    process.send("usi")?;
    let mut options = Vec::new();
    process.receive_until(|line| {
        if line.starts_with("option ") {
            options.push(line.to_owned());
        }
        line.trim() == "usiok"
    })?;
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::limit::parse_search_limit;
    use std::path::PathBuf;

    fn commit_player() -> PlayerConfig {
        PlayerConfig {
            text: "commit:abc".to_owned(),
            identity: EngineIdentity::Commit {
                hash: "a".repeat(40),
                sha256: "b".repeat(64),
            },
            path: PathBuf::from("minase"),
            args: Vec::new(),
            protocol: Protocol::Usi,
            is_random: false,
            limit: parse_search_limit("depth=1").unwrap(),
            hash_mb: None,
            rules_source: "engine-default".to_owned(),
            options: Vec::new(),
        }
    }

    // alphazero.md「共存の構造」: ハーネスはcommit:のエンジンに`Threads`の既定値だけを要求し、
    // `USI_Hash`を宣言しないエンジン（--search mcts）も受理する。
    #[test]
    fn commit_engines_need_threads_but_not_hash() {
        let player = commit_player();
        let without_hash = EngineDefaults {
            threads: Some(1),
            hash_mb: None,
        };
        assert_eq!(
            require_commit_threads(&player, without_hash).unwrap(),
            without_hash
        );
        let error = require_commit_threads(
            &player,
            EngineDefaults {
                threads: None,
                hash_mb: Some(256),
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("did not report a Threads default")
        );
    }
}

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
    if matches!(player.identity, EngineIdentity::Commit { .. })
        && (defaults.threads.is_none() || defaults.hash_mb.is_none())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "commit engine {} did not report Threads and USI_Hash defaults",
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

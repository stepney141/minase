//! 保存した実行条件からの対局設定の復元。

use super::cli::Mode;
use super::storage::{EngineRecord, ManifestMode, RunManifest, StoredSearchLimit};
use minase::harness::*;
use minase_core::{Rules, rules::parse_rule_set};
use std::{io, time::Duration};

/// 再開時に明示された仮説だけを保存済みの条件と照合する。
pub(super) fn verify_mode(requested: Option<&Mode>, recorded: &ManifestMode) -> Result<(), String> {
    let Some(Mode::Gsprt { elo0, elo1, .. }) = requested else {
        return Ok(());
    };
    let ManifestMode::Gsprt { h0_elo, h1_elo, .. } = recorded else {
        return Err("gsprt does not match the recorded mode".to_owned());
    };
    for (name, requested, saved) in [("elo0", elo0, h0_elo), ("elo1", elo1, h1_elo)] {
        if let Some(value) = requested
            && value != saved
        {
            return Err(format!(
                "--{name} {value} does not match the recorded value {saved}"
            ));
        }
    }
    Ok(())
}

pub(super) fn restore(manifest: &RunManifest) -> io::Result<(Rules, PlayerConfig, PlayerConfig)> {
    let codes = parse_rule_set(&manifest.rules_source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let rules = Rules::from_codes(&codes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let candidate = restore_player(
        &manifest.candidate,
        manifest.engine_threads.candidate,
        manifest.hash_mb.candidate,
        manifest,
    )?;
    let baseline = restore_player(
        &manifest.baseline,
        manifest.engine_threads.baseline,
        manifest.hash_mb.baseline,
        manifest,
    )?;
    Ok((rules, candidate, baseline))
}

fn restore_player(
    record: &EngineRecord,
    threads: Option<u32>,
    hash_mb: Option<u64>,
    manifest: &RunManifest,
) -> io::Result<PlayerConfig> {
    let spec = restore_player_spec(&record.identity)?;
    let limit = match record.limit {
        StoredSearchLimit::Fixed { depth, nodes } => SearchLimit::Fixed { depth, nodes },
        StoredSearchLimit::Time {
            base_ms,
            increment_ms,
            byoyomi_ms,
        } => SearchLimit::Time(TimeControl {
            base_ms,
            increment_ms,
            byoyomi_ms,
        }),
    };
    let mut player = resolve_player(spec, limit, hash_mb, &manifest.rules_source, Vec::new())?;
    verify_identity(&record.identity, &player.identity)?;
    let defaults =
        probe_engine_defaults(&player, Duration::from_secs(manifest.response_timeout_secs))?;
    if defaults.threads != threads {
        return Err(ResumeError::ThreadsMismatch.into());
    }
    player.hash_mb = restored_hash(hash_mb, defaults.hash_mb)?;
    Ok(player)
}

fn verify_identity(
    recorded: &EngineIdentity,
    resolved: &EngineIdentity,
) -> Result<(), ResumeError> {
    if recorded != resolved {
        return Err(ResumeError::EngineIdentityMismatch);
    }
    Ok(())
}

fn restored_hash(recorded: Option<u64>, default: Option<u64>) -> Result<Option<u64>, ResumeError> {
    if recorded == default {
        Ok(None)
    } else {
        recorded.map(Some).ok_or(ResumeError::HashUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_rejects_changed_engine_sha256() {
        let recorded = EngineIdentity::Commit {
            hash: "a".repeat(40),
            sha256: "b".repeat(64),
        };
        let resolved = EngineIdentity::Commit {
            hash: "a".repeat(40),
            sha256: "c".repeat(64),
        };
        assert_eq!(
            verify_identity(&recorded, &resolved),
            Err(ResumeError::EngineIdentityMismatch)
        );
    }

    #[test]
    fn resume_sends_hash_only_when_it_differs_from_the_default() {
        assert_eq!(restored_hash(Some(256), Some(256)), Ok(None));
        assert_eq!(restored_hash(Some(128), Some(256)), Ok(Some(128)));
        assert_eq!(restored_hash(Some(128), None), Ok(Some(128)));
        assert_eq!(restored_hash(None, None), Ok(None));
        assert_eq!(
            restored_hash(None, Some(256)),
            Err(ResumeError::HashUnavailable)
        );
    }
}

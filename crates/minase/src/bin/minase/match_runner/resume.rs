//! 保存した実行条件からの対局設定の復元。

use super::storage::{EngineRecord, ResumeError, RunManifest, StoredSearchLimit};
use minase::harness::*;
use minase_core::{Rules, rules::parse_rule_set};
use std::{io, time::Duration};

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
    let kind = match &record.identity {
        EngineIdentity::Random => PlayerKind::Random,
        EngineIdentity::Commit { hash, .. } => PlayerKind::Commit(hash.clone()),
        EngineIdentity::Command {
            program,
            args,
            protocol,
            working_directory,
        } => {
            if std::env::current_dir()? != *working_directory {
                return Err(ResumeError::WorkingDirectoryMismatch.into());
            }
            match protocol {
                StoredProtocol::Usi => PlayerKind::Command {
                    program: program.clone(),
                    args: args.clone(),
                },
                StoredProtocol::Cecp => PlayerKind::Cecp {
                    program: program.clone(),
                    args: args.clone(),
                },
            }
        }
    };
    let text = match &kind {
        PlayerKind::Random => "random".to_owned(),
        PlayerKind::Commit(hash) => format!("commit:{hash}"),
        PlayerKind::Command { program, args } | PlayerKind::Cecp { program, args } => {
            format!("{} {}", program.display(), args.join(" "))
        }
    };
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
    let mut player = resolve_player(
        PlayerSpec { text, kind },
        limit,
        hash_mb,
        &manifest.rules_source,
        Vec::new(),
    )?;
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

//! 保存した実行条件からの対局設定の復元。
//!
//! 読み込む実行条件と起動履歴の欄は書き手の[`super::storage`]に定める。

use super::cli::Mode;
use super::storage::{EngineRecord, ManifestMode, RunManifest, StoredSearchLimit};
use minase::harness::*;
use minase_core::{Rules, rules::parse_rule_set};
use std::{io, time::Duration};

/// 再開時の`gsprt --elo0`と`--elo1`を、指定された値だけ保存済み仮説と照合する。
/// この指定で統計モードや仮説を変更することはできない。
pub(super) fn verify_mode(
    requested: Option<&Mode>,
    recorded: &ManifestMode,
) -> Result<(), ModeMismatch> {
    let Some(Mode::Gsprt { elo0, elo1, .. }) = requested else {
        return Ok(());
    };
    let ManifestMode::Gsprt { h0_elo, h1_elo, .. } = recorded else {
        return Err(ModeMismatch::Mode);
    };
    for (name, requested, saved) in [("elo0", elo0, h0_elo), ("elo1", elo1, h1_elo)] {
        if let Some(value) = requested
            && value != saved
        {
            return Err(ModeMismatch::Hypothesis {
                name,
                requested: *value,
                saved: *saved,
            });
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
pub(super) enum ModeMismatch {
    Mode,
    Hypothesis {
        name: &'static str,
        requested: f64,
        saved: f64,
    },
}
impl std::fmt::Display for ModeMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mode => f.write_str("gsprt does not match the recorded mode"),
            Self::Hypothesis {
                name,
                requested,
                saved,
            } => write!(
                f,
                "--{name} {requested} does not match the recorded value {saved}"
            ),
        }
    }
}
impl std::error::Error for ModeMismatch {}

/// 保存した`manifest.json`から規則と両エンジンを復元する。
///
/// 再開の統計モードと仮説、基本シード、手数上限、応答期限、先読みの有無、
/// 置換表容量、および同時対局数も、呼び出し側が同じmanifestから読む。
/// `commit:`は記録した完全ハッシュを再解決し、SHA-256を含む同一性の一致を要求する。
/// 起動コマンドは記録したプログラム、引数、プロトコルを使い、
/// 記録した作業ディレクトリと同じ場所からだけ復元する。
/// 任意の起動コマンドの実体と`random`にはSHA-256の照合がない。
/// `random`は現在の実行プログラム自身を`dev usi-random`で起動する。
///
/// 各エンジンを1回起動して読んだ`Threads`の既定値が記録と異なればエラーとする。
/// 置換表容量は記録値と既定値が等しければ送らず、異なれば記録値を送る。
/// 理由は`docs/adr/resume-from-the-manifest-and-accept-rebuilt-tuning-commits.md`にある。
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
    fn verify_mode_checks_only_explicit_hypotheses() {
        let recorded = ManifestMode::Gsprt {
            h0_elo: 20.0,
            h1_elo: 30.0,
            alpha: 0.05,
            beta: 0.05,
        };
        assert_eq!(verify_mode(None, &recorded), Ok(()));
        for (elo0, elo1, expected) in [
            (None, None, Ok(())),
            (Some(20.0), None, Ok(())),
            (None, Some(30.0), Ok(())),
            (Some(20.0), Some(30.0), Ok(())),
            (
                Some(0.0),
                None,
                Err(ModeMismatch::Hypothesis {
                    name: "elo0",
                    requested: 0.0,
                    saved: 20.0,
                }),
            ),
            (
                None,
                Some(10.0),
                Err(ModeMismatch::Hypothesis {
                    name: "elo1",
                    requested: 10.0,
                    saved: 30.0,
                }),
            ),
            (
                Some(21.0),
                None,
                Err(ModeMismatch::Hypothesis {
                    name: "elo0",
                    requested: 21.0,
                    saved: 20.0,
                }),
            ),
            (
                None,
                Some(31.0),
                Err(ModeMismatch::Hypothesis {
                    name: "elo1",
                    requested: 31.0,
                    saved: 30.0,
                }),
            ),
        ] {
            let requested = Mode::Gsprt {
                max_pairs: None,
                elo0,
                elo1,
            };
            assert_eq!(verify_mode(Some(&requested), &recorded), expected);
        }
        let requested = Mode::Gsprt {
            max_pairs: None,
            elo0: Some(0.0),
            elo1: None,
        };
        assert_eq!(
            verify_mode(Some(&requested), &ManifestMode::Elo),
            Err(ModeMismatch::Mode)
        );
    }

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

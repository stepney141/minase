//! 保存したエンジン指定の復元と再開エラー。

use super::{EngineIdentity, PlayerKind, PlayerSpec, StoredProtocol};
use std::io;

/// 保存条件を安全に復元できない理由。
#[derive(Debug, PartialEq, Eq)]
pub enum ResumeError {
    /// 起動の記録が空である。
    EmptyInvocations,
    /// 起動ごとの目標が正かつ非減少になっていない。
    InvalidTarget,
    /// 指定した目標が前回の目標を超えない。
    TargetNotIncreased,
    /// エンジンを起動する作業ディレクトリが変わった。
    WorkingDirectoryMismatch,
    /// エンジンの実行ファイルまたはコミットが変わった。
    EngineIdentityMismatch,
    /// 探索ワーカー数の既定値が変わった。
    ThreadsMismatch,
    /// 保存した置換表容量を再現できない。
    HashUnavailable,
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::EmptyInvocations => "invocations must contain at least one invocation",
            Self::InvalidTarget => "invocation targets must be positive and nondecreasing",
            Self::TargetNotIncreased => "--target-pairs must exceed the previous invocation target",
            Self::WorkingDirectoryMismatch => {
                "engine working directory differs from the recorded working_directory"
            }
            Self::EngineIdentityMismatch => {
                "engine identity does not match the recorded SHA-256 or commit"
            }
            Self::ThreadsMismatch => {
                "engine Threads default differs from the recorded worker count"
            }
            Self::HashUnavailable => {
                "recorded hash size is unavailable but the engine now reports a default"
            }
        })
    }
}

impl std::error::Error for ResumeError {}

impl From<ResumeError> for io::Error {
    fn from(error: ResumeError) -> Self {
        Self::new(io::ErrorKind::InvalidData, error)
    }
}

/// 保存したコミットまたは起動コマンドを現在の作業ディレクトリで復元する。
pub fn restore_player_spec(identity: &EngineIdentity) -> io::Result<PlayerSpec> {
    if let EngineIdentity::Commit {
        working_directory, ..
    }
    | EngineIdentity::Command {
        working_directory, ..
    } = identity
        && std::env::current_dir()? != *working_directory
    {
        return Err(ResumeError::WorkingDirectoryMismatch.into());
    }
    let kind = match identity {
        EngineIdentity::Random => PlayerKind::Random,
        EngineIdentity::Commit { hash, .. } => PlayerKind::Commit(hash.clone()),
        EngineIdentity::Command {
            program,
            args,
            protocol,
            ..
        } => match protocol {
            StoredProtocol::Usi => PlayerKind::Command {
                program: program.clone(),
                args: args.clone(),
            },
            StoredProtocol::Cecp => PlayerKind::Cecp {
                program: program.clone(),
                args: args.clone(),
            },
        },
    };
    let text = match &kind {
        PlayerKind::Random => "random".to_owned(),
        PlayerKind::Commit(hash) => format!("commit:{hash}"),
        PlayerKind::Command { program, args } | PlayerKind::Cecp { program, args } => {
            format!("{} {}", program.display(), args.join(" "))
        }
    };
    Ok(PlayerSpec { text, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_resume_requires_the_recorded_working_directory_with_or_without_arguments() {
        let current = std::env::current_dir().unwrap();
        for args in [
            Vec::new(),
            vec!["--nn-model".to_owned(), "net.onnx".to_owned()],
        ] {
            let mut identity = EngineIdentity::Commit {
                hash: "a".repeat(40),
                sha256: "b".repeat(64),
                args,
                working_directory: current.clone(),
            };
            assert!(matches!(restore_player_spec(&identity).unwrap().kind,
                PlayerKind::Commit(hash) if hash == "a".repeat(40)));
            let EngineIdentity::Commit {
                working_directory, ..
            } = &mut identity
            else {
                unreachable!();
            };
            *working_directory = current.join("different-directory");
            let error = restore_player_spec(&identity).unwrap_err();
            assert_eq!(
                error.get_ref().unwrap().downcast_ref::<ResumeError>(),
                Some(&ResumeError::WorkingDirectoryMismatch)
            );
        }
    }
}

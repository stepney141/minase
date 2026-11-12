//! 調整用ビルドのエンジンの指定と解決。

use minase::harness::*;
use std::{fs, io};

/// 調整エンジンの起動コマンドを解決できない原因。
#[derive(Debug)]
enum CommandError {
    /// 実行ファイルを検索するPATHが未設定である。
    PathNotSet,
    /// 実行ファイルがPATH内に見つからない。
    NotFound,
    /// 指定されたプレイヤーはUSIの調整エンジンではない。
    UnsupportedPlayer,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PathNotSet => "PATH is not set",
            Self::NotFound => "engine command is not in PATH",
            Self::UnsupportedPlayer => "SPSA requires a USI tuning engine",
        })
    }
}

impl std::error::Error for CommandError {}

pub(super) fn tuning_spec(text: &str) -> Result<PlayerSpec, String> {
    let spec = parse_player_spec(text)?;
    if matches!(spec.kind, PlayerKind::Random | PlayerKind::Cecp { .. }) {
        return Err(
            "SPSA requires a USI tuning engine; random and cecp: are not supported".to_owned(),
        );
    }
    Ok(spec)
}

/// 起動コマンドを解決する。握手だけの照会では対局の規則や探索制限を要求しない。
pub(super) fn resolve_command(
    spec: &PlayerSpec,
) -> io::Result<(std::path::PathBuf, Vec<String>, EngineIdentity)> {
    match &spec.kind {
        PlayerKind::Commit(revision) => {
            let (path, hash, sha256) = resolve_commit(revision, Some("tuning"))?;
            Ok((
                path,
                vec!["--protocol".to_owned(), "usi".to_owned()],
                EngineIdentity::Commit {
                    hash,
                    sha256,
                    args: Vec::new(),
                    working_directory: std::env::current_dir()?,
                },
            ))
        }
        PlayerKind::Command { program, args } => {
            // 裸のコマンド名は起動時と同じPATHで解決し、ハッシュ対象を固定する。
            let mut path = program.clone();
            if path.components().count() == 1 {
                let search_path = std::env::var_os("PATH").ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, CommandError::PathNotSet)
                })?;
                path = std::env::split_paths(&search_path)
                    .map(|p| p.join(program))
                    .find(|p| p.is_file())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, CommandError::NotFound)
                    })?;
            }
            Ok((
                fs::canonicalize(path)?,
                args.clone(),
                EngineIdentity::Command {
                    program: program.clone(),
                    args: args.clone(),
                    protocol: StoredProtocol::Usi,
                    working_directory: std::env::current_dir()?,
                },
            ))
        }
        PlayerKind::Random | PlayerKind::Cecp { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            CommandError::UnsupportedPlayer,
        )),
    }
}

pub(super) fn resolve_engine(
    spec: PlayerSpec,
    each: SearchLimit,
    rules: &str,
) -> io::Result<PlayerConfig> {
    let (path, args, identity) = resolve_command(&spec)?;
    Ok(PlayerConfig {
        text: spec.text,
        identity,
        path,
        args,
        protocol: Protocol::Usi,
        is_random: false,
        limit: each,
        hash_mb: None,
        rules_source: rules.to_owned(),
        options: Vec::new(),
        network_sha256: None,
    })
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

/// コミット指定は完全ハッシュ、引数、作業ディレクトリ、それ以外は開始時の実行ファイルのSHA-256で照合する。
pub(super) fn check_engine_identity(
    manifest: &super::storage::Manifest,
    identity: &EngineIdentity,
    sha256: &str,
) -> Result<(), ResumeError> {
    let matches = match &manifest.engine {
        EngineIdentity::Commit {
            hash,
            args,
            working_directory,
            ..
        } => {
            matches!(identity, EngineIdentity::Commit {
                hash: resolved, args: resolved_args, working_directory: resolved_directory, ..
            } if resolved == hash && resolved_args == args && resolved_directory == working_directory)
        }
        _ => sha256 == manifest.engine_sha256,
    };
    if !matches {
        return Err(ResumeError::EngineIdentityMismatch);
    }
    Ok(())
}

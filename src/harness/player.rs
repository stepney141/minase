//! 外部エンジンの指定と起動構成。

use crate::harness::commit::{resolve_commit, sha256_file};
use crate::harness::engine::validate_cecp_limit;
use crate::harness::limit::SearchLimit;
use crate::harness::records::{EngineIdentity, StoredProtocol};
use std::io;
use std::path::PathBuf;

/// 外部エンジンの指定。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlayerSpec {
    /// 入力された指定の原文。表示に使う。
    pub text: String,
    /// 指定の解釈結果。
    pub kind: PlayerKind,
}

/// ランダムエンジンまたは実行ファイルの起動コマンドを表す。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlayerKind {
    /// 同梱の校正用ランダムエンジン。
    Random,
    /// ビルドして使うgitリビジョン。
    Commit(String),
    /// 任意のUSIエンジンの起動コマンド。
    Command {
        /// 実行ファイルのパス。
        program: PathBuf,
        /// 起動引数。
        args: Vec<String>,
    },
    /// 任意のCECPエンジンの起動コマンド。
    Cecp {
        /// 実行ファイルのパス。
        program: PathBuf,
        /// 起動引数。
        args: Vec<String>,
    },
}

/// 外部エンジンとの通信プロトコル。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    /// USIプロトコル。
    Usi,
    /// CECP（XBoard）プロトコル。
    Cecp,
}

/// 起動に必要な解決済みプレイヤー設定。
#[derive(Clone)]
pub struct PlayerConfig {
    /// 入力された指定の原文。表示に使う。
    pub text: String,
    /// 再開時の照合に使う完全コミットハッシュまたは起動指定。
    pub identity: EngineIdentity,
    /// 実行ファイルのパス。
    pub path: PathBuf,
    /// 起動引数。
    pub args: Vec<String>,
    /// エンジンとの通信プロトコル。
    pub protocol: Protocol,
    /// 校正用ランダムエンジンかどうか。真ならシードを設定する。
    pub is_random: bool,
    /// このプレイヤーに適用する思考制限。
    pub limit: SearchLimit,
    /// このプレイヤーに適用する置換表容量(MB)。省略時はエンジンの既定値。
    pub hash_mb: Option<u64>,
    /// 起動引数と規則オプションへ渡す`--rules`入力原文。
    pub rules_source: String,
    /// 握手後、isreadyより前に列順で送るUSIオプションの名前と値。
    pub options: Vec<(String, String)>,
}

impl PlayerConfig {
    /// specと実効制限を組み合わせた表示名を返す。
    pub fn name(&self) -> String {
        format!("{} limit={}", self.text, self.limit.cli_text())
    }
}

/// 外部エンジン指定を解析する。空白区切りの2語目以降は起動引数として渡す。
pub fn parse_player_spec(input: &str) -> Result<PlayerSpec, String> {
    if input == "random" {
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Random,
        });
    }
    let mut tokens = input.split_whitespace();
    let Some(program) = tokens.next() else {
        return Err("engine spec must be a command line or 'random'".to_owned());
    };
    if let Some(program) = program.strip_prefix("cecp:") {
        if program.is_empty() {
            return Err("cecp: engine spec requires a startup command".to_owned());
        }
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Cecp {
                program: PathBuf::from(program),
                args: tokens.map(str::to_owned).collect(),
            },
        });
    }
    if program.starts_with("depth=") {
        return Err("the legacy depth=N player spec is not supported; use --each".to_owned());
    }
    if let Some(revision) = program.strip_prefix("commit:") {
        if revision.is_empty() {
            return Err("commit: engine spec requires a revision".to_owned());
        }
        if tokens.next().is_some() {
            return Err("commit: engine spec does not accept startup arguments".to_owned());
        }
        return Ok(PlayerSpec {
            text: input.to_owned(),
            kind: PlayerKind::Commit(revision.to_owned()),
        });
    }
    Ok(PlayerSpec {
        text: input.to_owned(),
        kind: PlayerKind::Command {
            program: PathBuf::from(program),
            args: tokens.map(str::to_owned).collect(),
        },
    })
}

/// specを起動コマンドと実効制限へ解決する。
///
/// `options`はUSIの握手後、`isready`より前に列順で送る名前と値である。
/// CECP指定で列が空でなければ入力エラーを返す。コミット指定は通常ビルドを使う。
pub fn resolve_player(
    spec: PlayerSpec,
    limit: SearchLimit,
    hash_mb: Option<u64>,
    rules_text: &str,
    options: Vec<(String, String)>,
) -> io::Result<PlayerConfig> {
    if matches!(spec.kind, PlayerKind::Cecp { .. }) && !options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CECP engine does not support USI setoption settings",
        ));
    }
    if let Some(hash_mb) = hash_mb {
        if hash_mb == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "engine hash size must be at least 1 MB",
            ));
        }
        if matches!(spec.kind, PlayerKind::Random) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "random engine does not support a hash size",
            ));
        }
        if matches!(spec.kind, PlayerKind::Cecp { .. }) && !hash_mb.is_power_of_two() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CECP engine hash size must be a power of two in MB because HaChu rounds its memory allocation",
            ));
        }
    }
    let working_directory = std::env::current_dir()?;
    let (path, args, protocol, is_random, identity) = match spec.kind {
        PlayerKind::Random => {
            let current = std::env::current_exe()?;
            let filename = format!("usi_random{}", std::env::consts::EXE_SUFFIX);
            let path = current.with_file_name(filename);
            (
                path.clone(),
                Vec::new(),
                Protocol::Usi,
                true,
                EngineIdentity::Random {
                    sha256: sha256_file(&path)?,
                },
            )
        }
        PlayerKind::Commit(revision) => {
            let (path, hash, sha256) = resolve_commit(&revision, None)?;
            (
                path,
                vec![
                    "--protocol".to_owned(),
                    "usi".to_owned(),
                    "--rules".to_owned(),
                    rules_text.to_owned(),
                ],
                Protocol::Usi,
                false,
                EngineIdentity::Commit { hash, sha256 },
            )
        }
        PlayerKind::Command { program, args } => (
            program.clone(),
            args.clone(),
            Protocol::Usi,
            false,
            EngineIdentity::Command {
                program,
                args,
                protocol: StoredProtocol::Usi,
                working_directory,
            },
        ),
        PlayerKind::Cecp { program, args } => {
            validate_cecp_limit(limit)?;
            (
                program.clone(),
                args.clone(),
                Protocol::Cecp,
                false,
                EngineIdentity::Command {
                    program,
                    args,
                    protocol: StoredProtocol::Cecp,
                    working_directory,
                },
            )
        }
    };
    Ok(PlayerConfig {
        text: spec.text,
        identity,
        path,
        args,
        protocol,
        is_random,
        limit,
        hash_mb,
        rules_source: rules_text.to_owned(),
        options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::engine::{EngineDefaults, probe_engine_defaults};
    use crate::harness::limit::parse_search_limit;
    use std::time::Duration;

    // D8-HARN-01（sprt.md「エンジンの指定方法」、match-harness.md「エンジン指定と
    // 解決」）: specは`commit:<hash>`・USI起動コマンド・`random`・CECP起動コマンドの
    // 4形式であり、起動コマンドの2語目以降は引数になる。
    #[test]
    fn engine_specs_cover_the_documented_four_forms() {
        let random = parse_player_spec("random").expect("the reserved spec must be accepted");
        assert_eq!(random.kind, PlayerKind::Random);

        let commit = parse_player_spec("commit:0045833")
            .expect("a commit spec must be accepted without resolving it");
        assert_eq!(commit.kind, PlayerKind::Commit("0045833".to_owned()));

        // 起動コマンド形式: 2語目以降は起動引数として渡す
        let command = parse_player_spec("target/release/minase --protocol usi --rules R1")
            .expect("a command line spec must be accepted");
        assert_eq!(
            command.kind,
            PlayerKind::Command {
                program: PathBuf::from("target/release/minase"),
                args: vec![
                    "--protocol".to_owned(),
                    "usi".to_owned(),
                    "--rules".to_owned(),
                    "R1".to_owned(),
                ],
            }
        );

        let hachu = parse_player_spec("cecp:../hachu-debian/hachu")
            .expect("a CECP command without arguments must be accepted");
        assert_eq!(hachu.text, "cecp:../hachu-debian/hachu");
        assert_eq!(
            hachu.kind,
            PlayerKind::Cecp {
                program: PathBuf::from("../hachu-debian/hachu"),
                args: Vec::new(),
            }
        );
        let minase =
            parse_player_spec("cecp:target/release/minase --protocol cecp --rules engine-default")
                .expect("a CECP command with arguments must be accepted");
        assert_eq!(
            minase.kind,
            PlayerKind::Cecp {
                program: PathBuf::from("target/release/minase"),
                args: vec![
                    "--protocol".to_owned(),
                    "cecp".to_owned(),
                    "--rules".to_owned(),
                    "engine-default".to_owned(),
                ],
            }
        );

        // 4形式に含まれない入力は拒否される。旧`depth=N` specの削除は
        // match-harness.md適用範囲に明文がある。空リビジョンとcommit形式への
        // 起動引数付与の拒否は[実装契約](SPEC_UNCLEAR-05関連)。
        for invalid in [
            "",
            "depth=1",
            "depth=2,nodes=1000",
            "commit:",
            "commit:abc --x",
            "cecp:",
            "cecp:   ",
        ] {
            assert!(
                parse_player_spec(invalid).is_err(),
                "spec {invalid:?} must be rejected"
            );
        }
    }

    // D8-HARN-01境界（sprt.md「エンジンの指定方法」、match-harness.md「CECP
    // セッション管理」）と1手固定時間の仕様: CECPに写せないnodes、固定時間以外の
    // 秒読み、秒未満の時間単位は解決時にInvalidInputとして拒否する。
    #[test]
    fn cecp_resolution_rejects_unsupported_search_limits() {
        for limit in [
            parse_search_limit("nodes=1").unwrap(),
            parse_search_limit("depth=1,nodes=1").unwrap(),
            parse_search_limit("time=1000+0,byoyomi=1000").unwrap(),
            parse_search_limit("time=0+1000,byoyomi=1000").unwrap(),
            parse_search_limit("time=0+0,byoyomi=1500").unwrap(),
            parse_search_limit("time=1500+0").unwrap(),
            parse_search_limit("time=1000+1500").unwrap(),
        ] {
            let spec = parse_player_spec("cecp:engine").unwrap();
            let error = resolve_player(spec, limit, None, "R1", Vec::new())
                .err()
                .expect("the unsupported CECP limit must fail resolution");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    // D8-HARN-01/20（sprt.md「エンジンの指定方法」、match-harness.md「CECP
    // セッション管理」）と1手固定時間の仕様: 深さ固定、秒単位時間制御、
    // 整数秒の秒読みだけを使う固定時間はCECP設定へ解決できる。
    #[test]
    fn cecp_resolution_accepts_supported_limits_and_marks_the_protocol() {
        let depth = resolve_player(
            parse_player_spec("cecp:engine --option value").unwrap(),
            parse_search_limit("depth=4").unwrap(),
            None,
            "R1",
            Vec::new(),
        )
        .unwrap();
        assert_eq!(depth.protocol, Protocol::Cecp);
        assert_eq!(depth.path, PathBuf::from("engine"));
        assert_eq!(depth.args, ["--option", "value"]);
        assert_eq!(
            probe_engine_defaults(&depth, Duration::from_secs(1)).unwrap(),
            EngineDefaults {
                threads: None,
                hash_mb: Some(256),
            }
        );

        for limit in ["time=61000+2000", "time=0+0,byoyomi=2000"] {
            let time = resolve_player(
                parse_player_spec("cecp:engine").unwrap(),
                parse_search_limit(limit).unwrap(),
                None,
                "R1",
                Vec::new(),
            )
            .unwrap();
            assert_eq!(time.protocol, Protocol::Cecp);
        }
    }

    // 置換表容量の検証: 0、CECPでの2の冪以外、randomへの指定は拒否する。
    #[test]
    fn hash_resolution_rejects_invalid_or_inapplicable_sizes() {
        for (spec, hash_mb) in [
            ("engine", 0),
            ("cecp:engine", 0),
            ("cecp:engine", 3),
            ("cecp:engine", 255),
            ("cecp:engine", 257),
            ("random", 256),
        ] {
            let error = resolve_player(
                parse_player_spec(spec).unwrap(),
                parse_search_limit("depth=1").unwrap(),
                Some(hash_mb),
                "R1",
                Vec::new(),
            )
            .err()
            .expect("invalid or inapplicable hash size must fail resolution");
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        }
    }

    // phase2.md §4: CECPで追加のUSI設定があれば、プロセスを起動する前に拒否する。
    #[test]
    fn cecp_resolution_rejects_nonempty_usi_options() {
        let result = resolve_player(
            parse_player_spec("cecp:engine-that-does-not-exist").unwrap(),
            parse_search_limit("depth=1").unwrap(),
            None,
            "R1",
            vec![("Tune_First".to_owned(), "17".to_owned())],
        );
        assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidInput);
    }
}

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
///
/// 追加のUSIオプションは同じ応答の宣言と名前を完全一致で照合する。
/// `spin`は宣言範囲の整数、`combo`は宣言された選択肢、`check`は`true`か`false`を受理する。
/// `string`の値は変更せず、未宣言の名前や未対応の型は入力エラーにする。
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
    let (defaults, declarations) = process.read_usi_declarations().map_err(|failure| {
        io::Error::other(format!(
            "failed to read USI resource defaults from {}: {failure:?}",
            player.text
        ))
    })?;
    validate_usi_options(&player.options, &declarations).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid USI options for {}: {error}", player.text),
        )
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
    let (_, options) = process.read_usi_declarations()?;
    Ok(options)
}

/// 指定した名前の宣言を選び、型が定める値の範囲を検査する。
fn validate_usi_options(
    options: &[(String, String)],
    declarations: &[String],
) -> Result<(), String> {
    for (name, value) in options {
        let mut matching = declarations.iter().filter_map(|line| {
            let tokens: Vec<_> = line.split_whitespace().collect();
            if !tokens.starts_with(&["option", "name"]) {
                return None;
            }
            let type_index = tokens[2..].iter().position(|token| *token == "type")? + 2;
            (tokens[2..type_index].join(" ") == *name).then(|| tokens[type_index + 1..].to_vec())
        });
        let declaration = matching
            .next()
            .ok_or_else(|| format!("USI option '{name}' is not declared"))?;
        if matching.next().is_some() {
            return Err(format!("USI option '{name}' has multiple declarations"));
        }
        validate_usi_value(value, &declaration)
            .map_err(|error| format!("USI option '{name}': {error}"))?;
    }
    Ok(())
}

/// 宣言の型に従って値を検査し、範囲の欠落や未対応の型も拒否する。
fn validate_usi_value(value: &str, declaration: &[&str]) -> Result<(), String> {
    match declaration.first().copied() {
        Some("spin") => {
            let bound = |field| {
                declaration
                    .windows(2)
                    .find(|pair| pair[0] == field)
                    .and_then(|pair| pair[1].parse::<i128>().ok())
                    .ok_or_else(|| format!("spin declaration requires an integer {field}"))
            };
            let min = bound("min")?;
            let max = bound("max")?;
            if min > max {
                return Err("spin declaration has min greater than max".to_owned());
            }
            let number = value
                .parse::<i128>()
                .map_err(|_| format!("spin value '{value}' must be an integer"))?;
            if !(min..=max).contains(&number) {
                return Err(format!("spin value '{value}' must be in {min}..={max}"));
            }
        }
        Some("combo") => {
            let valid = declaration[1..]
                .split(|token| *token == "var")
                .skip(1)
                .any(|choice| {
                    choice
                        .iter()
                        .copied()
                        .take_while(|token| *token != "default")
                        .collect::<Vec<_>>()
                        .join(" ")
                        == value
                });
            if !valid {
                return Err(format!("combo value '{value}' is not a declared choice"));
            }
        }
        Some("check") if matches!(value, "true" | "false") => {}
        Some("check") => return Err("check value must be true or false".to_owned()),
        Some("string") => {}
        Some(kind) => return Err(format!("unsupported option type '{kind}'")),
        None => return Err("option declaration is missing its type".to_owned()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{parse_player_spec, parse_search_limit, resolve_player};

    fn probe(declarations: &str, name: &str, value: &str) -> io::Result<EngineDefaults> {
        let mut player = resolve_player(
            parse_player_spec("python3").unwrap(),
            parse_search_limit("depth=1").unwrap(),
            None,
            "engine-default",
            vec![(name.to_owned(), value.to_owned())],
            None,
        )
        .unwrap();
        player.args = vec![
            "-u".to_owned(),
            "-c".to_owned(),
            r#"
import sys
for line in sys.stdin:
    if line.strip() == 'usi':
        print(sys.argv[1] + '\nusiok', flush=True)
"#
            .to_owned(),
            declarations.to_owned(),
        ];
        probe_engine_defaults(&player, Duration::from_secs(5))
    }

    // フェーズ3の設計: 宣言範囲の両端、選択肢、真偽値、文字列を型に応じて受理する。
    #[test]
    fn probe_validates_requested_values_against_usi_declarations() {
        let declarations = concat!(
            "option name Threads type spin default 1 min 1 max 8\n",
            "option name USI_Hash type spin default 16 min 1 max 1024\n",
            "option name NnBatchSize type spin default 256 min 1 max 1024\n",
            "option name Offset type spin default 0 min -8 max 16\n",
            "option name Backend type combo default cpu var cpu var cuda graph\n",
            "option name Verify type check default false\n",
            "option name Output File type string default <empty>\n",
            "option name Clear type button\n",
        );
        for (name, value, valid) in [
            ("NnBatchSize", "1", true),
            ("NnBatchSize", "1024", true),
            ("NnBatchSize", "0", false),
            ("NnBatchSize", "1025", false),
            ("NnBatchSize", "1.0", false),
            ("NnBatchSize", "", false),
            (
                "NnBatchSize",
                "999999999999999999999999999999999999999999",
                false,
            ),
            ("Offset", "-8", true),
            ("Offset", "16", true),
            ("Offset", "-9", false),
            ("Offset", "17", false),
            ("Backend", "cpu", true),
            ("Backend", "cuda graph", true),
            ("Backend", "cuda", false),
            ("Backend", "", false),
            ("Verify", "true", true),
            ("Verify", "false", true),
            ("Verify", "True", false),
            ("Verify", "1", false),
            ("Output File", " path=a b ", true),
            ("Output File", "", true),
            ("Unknown", "1", false),
            ("Clear", "", false),
        ] {
            let result = probe(declarations, name, value);
            if valid {
                assert_eq!(
                    result.unwrap(),
                    EngineDefaults {
                        threads: Some(1),
                        hash_mb: Some(16)
                    },
                    "{name}={value}"
                );
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{name}={value}");
                assert!(error.to_string().contains(name));
            }
        }
    }

    #[test]
    fn probe_rejects_declarations_that_cannot_validate_the_requested_value() {
        for declaration in [
            "option name Setting type spin default 1 max 8",
            "option name Setting type spin default 1 min 0",
            "option name Setting type spin default 1 min x max 8",
            "option name Setting type spin default 1 min 8 max 0",
            "option name Setting type combo default 1",
            "option name Setting type filename default path",
            "option name Setting type",
            "option name Setting type string\noption name Setting type spin min 0 max 8",
        ] {
            assert_eq!(
                probe(declaration, "Setting", "1").unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{declaration}"
            );
        }
    }
}

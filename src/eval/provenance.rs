//! 段階9「教師の分類と来歴」のMNSD付随JSON。

use std::collections::HashSet;
use std::fmt;
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use super::rescore::is_hex;

/// 教師の探索条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum SearchCondition {
    /// 対局履歴と置換表を引き継ぐ探索。
    InGame,
    /// 局面だけを初期履歴とし、置換表を消去した探索。
    Standalone,
}

/// 対局結果の由来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ResultOrigin {
    /// 自己対局。
    Selfplay,
    /// 実戦棋譜。
    Human,
}

/// 開始局面の由来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum StartOrigin {
    /// 乱数による序盤。
    Random,
    /// 実戦棋譜の途中局面。
    HumanGame,
}

/// 教師の探索を特定する情報。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Teacher {
    /// 生成コミットの40桁16進表現。
    pub generation_commit: String,
    /// 重み本体の検査和の64桁16進表現。
    pub network_checksum: String,
    /// 教師探索のノード上限。
    pub nodes: u32,
    /// 規則セット名。
    pub rule_set: String,
    /// 対局中または局面単独の探索。
    pub search_condition: SearchCondition,
}

/// MNSDの対局番号と実戦棋譜の対応。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameOrigin {
    /// 1から始まるMNSD内の対局番号。
    pub game: u32,
    /// 実戦棋譜のID。
    pub id: String,
    /// 実戦開始の自己対局の開始手数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ply: Option<u32>,
}

/// 来歴JSONの交換形式。読み書きには検証付きの`read`と`write`を使う。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// 形式識別子。`minase-provenance`に固定。
    pub format: String,
    /// 形式の版。1に固定。
    pub version: u32,
    /// MNSD全体のSHA-256の64桁16進表現。
    pub mnsd_sha256: String,
    /// 教師の探索条件。
    pub teacher: Teacher,
    /// 対局結果の由来。
    pub result_origin: ResultOrigin,
    /// 開始局面の由来。
    pub start_origin: StartOrigin,
    /// 教師値に占める探索値の比率。
    pub lambda: f64,
    /// 実戦棋譜との対応。乱数開始の自己対局ではnull。
    #[serde(deserialize_with = "deserialize_games")]
    pub games: Option<Vec<GameOrigin>>,
}

/// `games`の省略を拒否し、明示的なnullまたは配列を要求する。
fn deserialize_games<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<GameOrigin>>, D::Error> {
    Option::<Vec<GameOrigin>>::deserialize(deserializer)
}

/// 来歴の解析、検証または出力の失敗。
#[derive(Debug)]
pub enum Error {
    /// JSONの構造、列挙値または数値型が不正。
    Json(serde_json::Error),
    /// 指定欄が契約を満たさない。
    InvalidField(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => error.fmt(f),
            Self::InvalidField(field) => write!(f, "invalid provenance field: {field}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl Provenance {
    /// 形式、教師の分類、混合比、および実戦棋譜との対応を検証する。
    pub fn validate(&self) -> Result<(), Error> {
        if self.format != "minase-provenance" {
            return Err(Error::InvalidField("format"));
        }
        if self.version != 1 {
            return Err(Error::InvalidField("version"));
        }
        if !is_hex(&self.mnsd_sha256, 64) {
            return Err(Error::InvalidField("mnsd_sha256"));
        }
        if !is_hex(&self.teacher.generation_commit, 40) {
            return Err(Error::InvalidField("teacher.generation_commit"));
        }
        if !is_hex(&self.teacher.network_checksum, 64) {
            return Err(Error::InvalidField("teacher.network_checksum"));
        }
        if self.teacher.nodes == 0 {
            return Err(Error::InvalidField("teacher.nodes"));
        }
        let rules = &self.teacher.rule_set;
        if rules.is_empty() || rules.len() > 32 || rules.contains('\0') {
            return Err(Error::InvalidField("teacher.rule_set"));
        }
        if !self.lambda.is_finite() || !(0.0..=1.0).contains(&self.lambda) {
            return Err(Error::InvalidField("lambda"));
        }
        let expected_lambda = match self.result_origin {
            ResultOrigin::Selfplay => 0.75,
            ResultOrigin::Human => 0.0,
        };
        if self.lambda != expected_lambda {
            return Err(Error::InvalidField("lambda for result_origin"));
        }
        let requires_games = self.result_origin == ResultOrigin::Human
            || self.start_origin == StartOrigin::HumanGame;
        if requires_games != self.games.is_some() {
            return Err(Error::InvalidField("games"));
        }
        if let Some(games) = &self.games {
            let mut numbers = HashSet::new();
            for game in games {
                if game.game == 0 || !numbers.insert(game.game) {
                    return Err(Error::InvalidField("games.game"));
                }
                if game.id.is_empty() {
                    return Err(Error::InvalidField("games.id"));
                }
                if game.ply.is_some() != (self.start_origin == StartOrigin::HumanGame) {
                    return Err(Error::InvalidField("games.ply"));
                }
            }
        }
        Ok(())
    }

    /// JSONを読み、契約を検証する。
    pub fn read(input: impl Read) -> Result<Self, Error> {
        let value: Self = serde_json::from_reader(input).map_err(Error::Json)?;
        value.validate()?;
        Ok(value)
    }

    /// 契約を検証してJSONを書く。
    pub fn write(&self, output: impl Write) -> Result<(), Error> {
        self.validate()?;
        serde_json::to_writer_pretty(output, self).map_err(Error::Json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        json!({
            "format": "minase-provenance", "version": 1, "mnsd_sha256": "a".repeat(64),
            "teacher": {"generation_commit": "b".repeat(40), "network_checksum": "c".repeat(64), "nodes": 200, "rule_set": "engine-default", "search_condition": "in-game"},
            "result_origin": "selfplay", "start_origin": "random", "lambda": 0.75, "games": null
        })
    }

    fn read(value: &Value) -> Result<Provenance, Error> {
        Provenance::read(value.to_string().as_bytes())
    }

    #[test]
    fn provenance_round_trips_all_supported_origins() {
        for human in [false, true] {
            for start_from_game in [false, true] {
                let mut value = fixture();
                if human {
                    value["result_origin"] = json!("human");
                    value["lambda"] = json!(0.0);
                }
                if start_from_game {
                    value["start_origin"] = json!("human-game");
                }
                if human || start_from_game {
                    value["games"] = json!([{"game": 1, "id": "fixture-game"}]);
                    if start_from_game {
                        value["games"][0]["ply"] = json!(40);
                    }
                }
                for condition in ["in-game", "standalone"] {
                    value["teacher"]["search_condition"] = json!(condition);
                    let provenance = read(&value).unwrap();
                    let mut output = Vec::new();
                    provenance.write(&mut output).unwrap();
                    assert_eq!(Provenance::read(output.as_slice()).unwrap(), provenance);
                    assert_eq!(serde_json::from_slice::<Value>(&output).unwrap(), value);
                }
            }
        }
    }

    #[test]
    fn provenance_rejects_invalid_fields_and_missing_game_mappings() {
        for (pointer, replacement) in [
            ("/format", json!("other")),
            ("/version", json!(2)),
            ("/mnsd_sha256", json!("a".repeat(63))),
            ("/mnsd_sha256", json!("z".repeat(64))),
            ("/teacher/generation_commit", json!("b".repeat(39))),
            ("/teacher/network_checksum", json!("g".repeat(64))),
            ("/teacher/nodes", json!(0)),
            ("/teacher/rule_set", json!("")),
            ("/teacher/search_condition", json!("unknown")),
            ("/result_origin", json!("unknown")),
            ("/start_origin", json!("unknown")),
            ("/lambda", json!(-0.1)),
            ("/lambda", json!(1.1)),
            ("/lambda", json!(0.5)),
            ("/lambda", json!(null)),
            ("/games", json!([])),
            ("/start_origin", json!("human-game")),
        ] {
            let mut value = fixture();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(read(&value).is_err(), "{pointer}");
        }
        let mut value = fixture();
        value.as_object_mut().unwrap().remove("games");
        assert!(read(&value).is_err());
        let mut value = fixture();
        value["result_origin"] = json!("human");
        value["lambda"] = json!(0.0);
        assert!(matches!(read(&value), Err(Error::InvalidField("games"))));
        value["games"] = json!([{"game": 0, "id": "x"}]);
        assert!(read(&value).is_err());
        value["games"] = json!([{"game": 1, "id": ""}]);
        assert!(read(&value).is_err());
        value["games"] = json!([{"game": 1, "id": "x"}, {"game": 1, "id": "y"}]);
        assert!(read(&value).is_err());
        value["start_origin"] = json!("human-game");
        value["games"] = json!([{"game": 1, "id": "x"}]);
        assert!(read(&value).is_err());
        let mut provenance = read(&fixture()).unwrap();
        provenance.lambda = f64::NAN;
        assert!(matches!(
            provenance.validate(),
            Err(Error::InvalidField("lambda"))
        ));
    }
}

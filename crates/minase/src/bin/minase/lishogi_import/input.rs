//! lishogiのゲームNDJSONの型と持ち時間の解釈。
//!
//! # 入力NDJSON
//!
//! UTF-8の各行を独立したJSONオブジェクトとして読む。
//! 見出し、空行、コメント行は認めず、未知のキーは読み飛ばす。
//! マジックと版欄はない。
//!
//! | キー | 型 | 内容 |
//! |---|---|---|
//! | `id` | 文字列、必須 | 空白を含まない空でない棋譜IDを指定する。 |
//! | `moves` | 文字列、必須 | 空白で区切ったUSI着手列を指定する。 |
//! | `variant` | 文字列またはnull、省略可 | 対局の変種を示す。 |
//! | `initialSfen` | 任意のJSON値、省略可 | 初期局面指定の有無を示し、nullでも欄があれば指定ありとして保持する。 |
//! | `players` | オブジェクトまたはnull、省略可 | `sente`と`gote`に対局者を収める。 |
//! | `rated` | 真偽値またはnull、省略可 | レーティング対局かを示す。 |
//! | `speed` | 文字列またはnull、省略可 | 持ち時間の区分を示す。 |
//! | `status` | 文字列またはnull、省略可 | 終局状態を示す。 |
//! | `winner` | 文字列またはnull、省略可 | 勝者を示す。 |
//! | `clock` | 任意のJSON値、省略可 | リアルタイム対局の時計情報を保持する。 |
//! | `daysPerTurn` | 任意のJSON値、省略可 | 通信対局の持ち時間を保持する。 |
//!
//! `players.sente`と`players.gote`は省略可能なオブジェクトまたはnullである。
//! 各対局者は`user`、`aiLevel`、`rating`、`provisional`を持ち、いずれも省略またはnullを許す。
//! `user`は省略可能な文字列`title`を持つオブジェクト、`aiLevel`は任意のJSON値、
//! `rating`はi32、`provisional`は真偽値である。
//! 欄の復号後、採用する棋譜の条件は[`super::filter::metadata_exclusion`]で別に判定する。

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InputGame {
    pub(super) id: String,
    pub(super) variant: Option<String>,
    #[serde(default, deserialize_with = "present_field")]
    pub(super) initial_sfen: Option<Value>,
    pub(super) players: Option<Players>,
    pub(super) rated: Option<bool>,
    speed: Option<String>,
    pub(super) status: Option<String>,
    pub(super) winner: Option<String>,
    pub(super) moves: String,
    pub(super) clock: Option<Value>,
    pub(super) days_per_turn: Option<Value>,
}

/// 持ち時間の区分を返す。lishogiの書き出しは`speed`欄を省くことが多いので、
/// `clock`（リアルタイム）または`daysPerTurn`（通信対局）からも導く。どちらもなければ`None`。
pub(super) fn time_control(game: &InputGame) -> Option<String> {
    if let Some(speed) = game.speed.as_ref().filter(|s| !s.is_empty()) {
        return Some(speed.clone());
    }
    if game.clock.as_ref().is_some_and(Value::is_object) {
        return Some("realTime".to_owned());
    }
    if game.days_per_turn.as_ref().is_some_and(|v| !v.is_null()) {
        return Some("correspondence".to_owned());
    }
    None
}

/// nullを含め、欄が存在すること自体を保持する。
fn present_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
pub(super) struct Players {
    pub(super) sente: Option<Player>,
    pub(super) gote: Option<Player>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Player {
    pub(super) user: Option<User>,
    pub(super) ai_level: Option<Value>,
    pub(super) rating: Option<i32>,
    pub(super) provisional: Option<bool>,
}

#[derive(Deserialize)]
pub(super) struct User {
    pub(super) title: Option<String>,
}

/// 1行の読み取り結果を、行番号と原因型を保って復号する。
pub(super) fn parse_line(
    line: std::io::Result<String>,
    line_number: usize,
) -> std::io::Result<InputGame> {
    use super::ImportError;
    use minase::datagen::data_error;
    let line = line.map_err(|source| {
        data_error(ImportError::ReadLine {
            line: line_number,
            source,
        })
    })?;
    let input: InputGame = serde_json::from_str(&line).map_err(|source| {
        data_error(ImportError::JsonLine {
            line: line_number,
            source,
        })
    })?;
    if input.id.is_empty() || input.id.chars().any(char::is_whitespace) {
        return Err(data_error(ImportError::InvalidId { line: line_number }));
    }
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::super::ImportError;
    use super::*;
    use std::io::BufRead;

    #[test]
    fn parse_line_rejects_malformed_missing_null_numeric_and_non_utf8_input() {
        for (bytes, expected) in [
            (b"{".as_slice(), serde_json::error::Category::Eof),
            (b"{}".as_slice(), serde_json::error::Category::Data),
            (b"null".as_slice(), serde_json::error::Category::Data),
            (
                br#"{"id":42,"moves":""}"#.as_slice(),
                serde_json::error::Category::Data,
            ),
        ] {
            let error = parse_line(bytes.lines().next().unwrap(), 2).err().unwrap();
            let ImportError::JsonLine { line, source } = error
                .get_ref()
                .unwrap()
                .downcast_ref::<ImportError>()
                .unwrap()
            else {
                panic!("expected JSON failure")
            };
            assert_eq!(*line, 2);
            assert_eq!(source.classify(), expected);
        }
        let error = parse_line(b"\xff".as_slice().lines().next().unwrap(), 2)
            .err()
            .unwrap();
        let ImportError::ReadLine { line, source } = error
            .get_ref()
            .unwrap()
            .downcast_ref::<ImportError>()
            .unwrap()
        else {
            panic!("expected UTF-8 failure")
        };
        assert_eq!(*line, 2);
        assert_eq!(source.kind(), std::io::ErrorKind::InvalidData);
        let parsed = parse_line(Ok(r#"{"id":"game","moves":""}"#.into()), 1).unwrap();
        assert_eq!(parsed.id, "game");
    }
}

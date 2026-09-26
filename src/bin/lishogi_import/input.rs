//! lishogiのゲームNDJSONの型と持ち時間の解釈。

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

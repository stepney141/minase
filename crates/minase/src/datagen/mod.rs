//! 学習局面の生成と棋譜取り込みで共有する支援処理。

use std::io;

use crate::training::records::Error as TrainingDataError;
use minase_core::Game;

pub mod game;
pub mod git;
pub mod provenance;
pub mod statistics;

/// 教師から除外する探索値の絶対値下限29,000。
/// 静的評価は絶対値28,999以下、詰み値は[`crate::search::MATE_THRESHOLD`]の29,744以上なので、その間に29,000〜29,743の空き帯を置く。
/// 詰み値は終局までの手数の目盛りであり、駒得の目盛りではないため教師にしない。
/// 理由は`docs/adr/exclude-mate-tactical-and-repeated-positions-from-teacher-data.md`にある。
pub const MATE_BAND_START: u32 = 29_000;

/// 現局面の探索キーが同じ対局の過去に現れたかを調べ、教師からの除外に使う。
/// 探索は対局履歴や探索経路へ戻る手順を引き分けとして読むため、再出現した局面の探索値は履歴の影響を強く受ける。
/// 静的評価には履歴を入力しないので、教師には対局内の最初の出現だけを残す。
/// 理由は`docs/adr/exclude-mate-tactical-and-repeated-positions-from-teacher-data.md`にある。
pub fn current_position_is_repeated(game: &Game) -> bool {
    let history = game.search_key_history();
    let Some((current, previous)) = history.split_last() else {
        return false;
    };
    previous.contains(current)
}

/// 下位エラーの型を保持してコマンドの不正データエラーへ変換する。
pub fn data_error(error: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// 学習データエラーをコマンドの不正データエラーへ変換する。
pub fn training_error(error: TrainingDataError) -> io::Error {
    invalid_data(error.to_string())
}

/// 説明を`InvalidData`の入出力エラーへ変換する。
pub fn invalid_data(error: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.into())
}

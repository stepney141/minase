//! 学習局面の生成と棋譜取り込みで共有する支援処理。

use std::io;

use crate::Game;
use crate::training::records::Error as TrainingDataError;

pub mod game;
pub mod git;
pub mod provenance;
pub mod statistics;

/// 詰み帯として除外する探索値の絶対値下限。
pub const MATE_BAND_START: u32 = 29_000;

/// 現局面の探索キーが同じ対局の過去に現れているかを返す。
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

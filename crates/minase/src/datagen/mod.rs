//! 学習局面の生成と棋譜取り込みで共有する支援処理。

use std::io;

use minase_core::{Game, IllegalMove, Move, MoveGenerator, Position};

pub mod game;
pub mod git;
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

/// 説明を`InvalidData`の入出力エラーへ変換する。
pub fn invalid_data(error: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.into())
}

/// バイト列を小文字の16進文字列へ変換する。
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 合法な最善手が捕獲または成りなら`true`を返す。
///
/// `best_move`が指定局面の合法手でなければ`IllegalMove`を返し、局面は変更しない。
pub fn best_move_is_tactical(
    position: &Position,
    generator: &MoveGenerator,
    best_move: Move,
) -> Result<bool, IllegalMove> {
    let mut legal_moves = Vec::new();
    generator.generate_moves(position, &mut legal_moves);
    if !legal_moves.contains(&best_move) {
        return Err(IllegalMove(best_move));
    }
    Ok(best_move.promote
        || position
            .captured_squares(best_move)
            .into_iter()
            .any(|square| square.is_some()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::{Color, test_util::sq};
    // 公開境界は未検証の着手をパニックせず型付きエラーとして拒否する。
    #[test]
    fn tactical_move_classification_rejects_a_move_without_an_origin_piece() {
        let position = Position::empty(Color::Black);
        let invalid = Move {
            from: sq(0, 0),
            mid: None,
            to: sq(0, 1),
            promote: false,
        };

        assert_eq!(
            best_move_is_tactical(&position, &MoveGenerator::standard(), invalid),
            Err(IllegalMove(invalid))
        );
    }
}

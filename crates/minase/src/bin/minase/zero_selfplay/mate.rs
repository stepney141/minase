//! 王手限定の3手以内の勝ちを、対局履歴を保った審判層の裁定で判定する。
//! 攻め方は最後の王駒の捕獲と王手だけを指し、受け方は全合法手を指す。
//! 受け方の王駒が2枚なら対象外とし、応手で2枚になった場合も受けとする。

use std::fmt;

use minase_core::{Color, Game, GameError, GameResult, GameStatus, Move, Square};

/// 手番側についての判定結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 王手限定の3手以内に、手番側の勝ちを強制できる。
    Win,
    /// この手の制限と手数では勝ちを強制できない。
    NoWin,
    /// 相手の王駒が2枚あるため判定しない。
    TwoRoyals,
}

/// 判定の前提または審判層の着手処理に関するエラー。
#[derive(Debug)]
pub enum Error {
    /// 終局後の呼び出し、または着手の受理に失敗した。
    Game(GameError),
    /// 継続中の局面で、受け方の王駒が1枚でも2枚でもない。
    RoyalCount {
        /// 受け方の色。
        color: Color,
        /// 王駒の枚数。
        count: u32,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Game(error) => write!(f, "mate-in-three: {error}"),
            Self::RoyalCount { color, count } => {
                write!(
                    f,
                    "mate-in-three: {color:?} has {count} royals in an ongoing game"
                )
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Game(error) => Some(error),
            Self::RoyalCount { .. } => None,
        }
    }
}

impl From<GameError> for Error {
    fn from(error: GameError) -> Self {
        Self::Game(error)
    }
}

/// 着手直後の継続中の対局から、次に指す側の王手限定の3手以内の勝ちを調べる。
///
/// 入力の対局を変更せず、各枝で複製した[`Game`]に着手して裁定を得る。
/// 攻め手の絞り込みと応手の並べ替えに使う利き調査は、局面の複製で行う。
///
/// # Errors
///
/// 終局済みの対局、受け方の王駒が0枚または3枚以上の継続局面、
/// および審判層が生成した合法手を受理しない場合にエラーを返す。
pub fn mate_in_three(game: &Game) -> Result<Verdict, Error> {
    if game.result().is_some() {
        return Err(GameError::GameAlreadyOver.into());
    }
    let attacker = game.position().side_to_move();
    if sole_defender_royal(game, attacker)?.is_none() {
        return Ok(Verdict::TwoRoyals);
    }
    Ok(if winning_attack(game, attacker, true)? {
        Verdict::Win
    } else {
        Verdict::NoWin
    })
}

/// 受け方の唯一の王駒を返す。2枚なら`None`を返す。
fn sole_defender_royal(game: &Game, attacker: Color) -> Result<Option<Square>, Error> {
    let color = attacker.opposite();
    let royals = game.position().royal_pieces(color);
    match royals.popcount() {
        1 => Ok(royals.lsb()),
        2 => Ok(None),
        count => Err(Error::RoyalCount { color, count }),
    }
}

/// 合法手が最後の王駒の捕獲または王手であるかを調べる。
fn is_attack(game: &Game, mv: Move, attacker: Color, royal: Square) -> bool {
    let mut position = game.position().clone();
    position.make_move_unchecked(mv, game.rules().moves);
    position.royal_pieces(attacker.opposite()).is_empty()
        || !position
            .attackers_to_by(attacker, royal, position.occupied())
            .is_empty()
}

/// m1またはm3を調べる。`read_replies`が真ならm2とm3まで読む。
fn winning_attack(game: &Game, attacker: Color, read_replies: bool) -> Result<bool, Error> {
    let Some(royal) = sole_defender_royal(game, attacker)? else {
        return Ok(false);
    };
    for mv in game.legal_moves() {
        if !is_attack(game, mv, attacker, royal) {
            continue;
        }
        let mut next = game.clone();
        match next.play(mv)? {
            GameStatus::Finished(GameResult::Win { winner, .. }) if winner == attacker => {
                return Ok(true);
            }
            GameStatus::Finished(_) => {}
            GameStatus::Ongoing => {
                if read_replies && all_replies_lose(&next, attacker)? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// m2を全合法手から調べ、受けが1つでもあれば打ち切る。
fn all_replies_lose(game: &Game, attacker: Color) -> Result<bool, Error> {
    let mut replies = game.legal_moves();
    // 王駒の先取り、王駒への利きがない手、その他の順にし、同順位の順序は保つ。
    // 順位は探索順にだけ使い、勝敗は各枝のGame::playで裁定する。
    replies.sort_by_cached_key(|&mv| {
        let mut position = game.position().clone();
        position.make_move_unchecked(mv, game.rules().moves);
        if position.royal_pieces(attacker).is_empty() {
            0
        } else if position
            .royal_pieces(attacker.opposite())
            .iter()
            .all(|royal| {
                position
                    .attackers_to_by(attacker, royal, position.occupied())
                    .is_empty()
            })
        {
            1
        } else {
            2
        }
    });
    for mv in replies {
        let mut next = game.clone();
        match next.play(mv)? {
            GameStatus::Finished(GameResult::Win { winner, .. }) if winner == attacker => {}
            GameStatus::Finished(_) => return Ok(false),
            GameStatus::Ongoing => {
                if !winning_attack(&next, attacker, false)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
#[path = "mate_tests.rs"]
mod tests;

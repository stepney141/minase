//! 反復規則R1の履歴と裁定、および不可逆手と攻撃的着手の判定。

use super::super::result::{DrawReason, GameResult, WinReason};
use crate::core::movegen::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::{Position, Undo};
use std::collections::HashMap;

/// 第31条R1の裁定に必要な連続可逆手数。
///
/// scalashogiの「局面ハッシュ履歴が12個を超える」に対応する。
/// 対局開始または不可逆手の直後の局面に可逆手12手分を加えると13局面になる。
const R1_MIN_REVERSIBLE_PLIES: u32 = 12;

/// R1で同一局面を判定するキー。
///
/// 局面本体とP1成り権保留状態のZobrist値を別成分として保持する。
/// 各値の衝突は実用上無視できるものとし、完全な局面署名とは照合しない。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct R1Key {
    /// 盤面・手番・先獅子状態のZobrist値。
    position_zobrist: u64,
    /// P1成り権保留状態のZobrist値。
    rights_zobrist: u64,
}

impl R1Key {
    /// 局面からR1の同一局面キーを作る。
    fn from_position(position: &Position) -> Self {
        Self {
            position_zobrist: position.zobrist(),
            rights_zobrist: position.rights_zobrist(),
        }
    }
}

/// R1で追跡する1局面の出現状態。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct R1PositionState {
    /// この局面の出現回数。
    occurrences: u8,
    /// 最初に出現した時点の手数。
    first_ply: u32,
}

/// R1の局面出現履歴と双方の攻撃連続数。
///
/// 対局開始または直前の不可逆手から可逆手が12手以上続き、同一局面が
/// 4回以上出現した場合に裁定する(第31条R1)。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct R1History {
    /// 出現済み局面ごとの出現状態。
    positions: HashMap<R1Key, R1PositionState>,
    /// 対局者ごとの、直近まで連続した攻撃的着手の数。
    consecutive_attacking_moves: [u32; 2],
    /// 対局開始または直前の不可逆手からの連続可逆手数。
    reversible_plies: u32,
}

impl R1History {
    /// 開始局面を第1回として記録した履歴を作る。
    pub(super) fn new(position: &Position) -> Self {
        Self {
            positions: HashMap::from([(
                R1Key::from_position(position),
                R1PositionState {
                    occurrences: 1,
                    first_ply: 0,
                },
            )]),
            consecutive_attacking_moves: [0; 2],
            reversible_plies: 0,
        }
    }

    /// 着手後の局面を記録し、4回以上出現していれば裁定結果を返す(第31条R1)。
    ///
    /// 対局開始または直前の不可逆手から可逆手が12手以上続いていることを要する。
    pub(crate) fn record_move(
        &mut self,
        position: &Position,
        ply: u32,
        mover: Color,
        is_attacking: bool,
        irreversible: bool,
    ) -> Option<GameResult> {
        self.reversible_plies = if irreversible {
            0
        } else {
            self.reversible_plies
                .checked_add(1)
                .expect("a reversible sequence cannot exceed u32::MAX plies")
        };
        self.consecutive_attacking_moves =
            updated_attacking_counters(self.consecutive_attacking_moves, mover, is_attacking);
        let state = self
            .positions
            .entry(R1Key::from_position(position))
            .or_insert(R1PositionState {
                occurrences: 0,
                first_ply: ply,
            });
        state.occurrences += 1;
        if state.occurrences < 4 || self.reversible_plies < R1_MIN_REVERSIBLE_PLIES {
            return None;
        }

        Some(r1_repetition_result(
            ply,
            state.first_ply,
            self.consecutive_attacking_moves,
        ))
    }

    /// 仮想着手が4回目以降の出現を生じさせる場合の裁定結果を、履歴を変更せずに返す。
    ///
    /// 仮想着手を含めて可逆手が12手以上続くことを要する(第31条R1)。
    /// 詰み判定(第21条第3項c)が回避手の即時勝利を調べるために使う。
    pub(crate) fn candidate_result(
        &self,
        position: &Position,
        ply: u32,
        mover: Color,
        is_attacking: bool,
        irreversible: bool,
    ) -> Option<GameResult> {
        if irreversible || self.reversible_plies < R1_MIN_REVERSIBLE_PLIES - 1 {
            return None;
        }
        let state = self.positions.get(&R1Key::from_position(position))?;
        if u16::from(state.occurrences) + 1 < 4 {
            return None;
        }
        let candidate_ply = ply
            .checked_add(1)
            .expect("a game cannot exceed u32::MAX plies");
        let counters =
            updated_attacking_counters(self.consecutive_attacking_moves, mover, is_attacking);
        Some(r1_repetition_result(
            candidate_ply,
            state.first_ply,
            counters,
        ))
    }
}

/// 4回目以降の同一局面出現に対するR1の裁定結果を返す(第31条R1)。
///
/// 最初の出現から裁定時までの自分の全着手が攻撃的着手であった対局者が
/// 一方だけならその側の負け、それ以外は引き分けとする。
pub(crate) fn r1_repetition_result(
    ply: u32,
    first_ply: u32,
    consecutive_attacking_moves: [u32; 2],
) -> GameResult {
    let attackers = Color::ALL.map(|color| {
        // 最初の出現以降にその対局者が指した手数を、攻撃連続数が覆うかで判定する。
        let distance =
            moves_by_color_through(ply, color) - moves_by_color_through(first_ply, color);
        consecutive_attacking_moves[color.index()] >= distance
    });

    match attackers {
        [true, false] => GameResult::Win {
            winner: Color::White,
            reason: WinReason::Repetition,
        },
        [false, true] => GameResult::Win {
            winner: Color::Black,
            reason: WinReason::Repetition,
        },
        [false, false] | [true, true] => GameResult::Draw {
            reason: DrawReason::Repetition,
        },
    }
}

/// 着手側の攻撃連続数を、攻撃的着手なら1増やし、そうでなければ0へ戻す。
pub(crate) fn updated_attacking_counters(
    mut counters: [u32; 2],
    mover: Color,
    is_attacking: bool,
) -> [u32; 2] {
    let counter = &mut counters[mover.index()];
    if is_attacking {
        *counter = counter
            .checked_add(1)
            .expect("an attack sequence cannot exceed u32::MAX plies");
    } else {
        *counter = 0;
    }
    counters
}

/// 指定手数までに指定対局者が指した着手数を返す。先手が奇数手目を指す。
fn moves_by_color_through(ply: u32, color: Color) -> u32 {
    match color {
        Color::Black => ply / 2 + ply % 2,
        Color::White => ply / 2,
    }
}

/// 成り、捕獲、不成の歩兵・香車の着手を不可逆手と判定する(第31条R1)。
pub(crate) fn move_is_irreversible(mv: Move, undo: &Undo) -> bool {
    mv.promote
        || undo.captured.iter().any(Option::is_some)
        || matches!(
            undo.moved_piece_before.kind(),
            Some(PieceKind::Pawn | PieceKind::Lance)
        )
}

/// 着手後の局面で、着手側の攻撃が継続しているかを返す。
pub(crate) fn move_was_attacking(
    position: &Position,
    generator: &MoveGenerator,
    mover: Color,
    played: Move,
) -> bool {
    let probe = position.clone_with_side_to_move(mover);
    let opponent_royals = probe.royal_pieces(mover.opposite());
    let destination = played.to;
    let mut moves = Vec::new();
    generator.generate_moves(&probe, &mut moves);

    moves.into_iter().any(|candidate| {
        let captures = probe.captured_squares(candidate);
        captures
            .into_iter()
            .flatten()
            .any(|square| opponent_royals.contains(square))
            || (candidate.from == destination
                && captures.into_iter().any(|capture| capture.is_some()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::board::Square;
    use crate::core::piece::PieceCode;
    use crate::core::rules::Rules;
    use crate::test_util::{position_from_codes as position, sq};

    fn piece(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
    }

    fn step(from: Square, to: Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    #[test]
    fn article_31_r1_irreversible_moves_are_promotions_captures_and_unpromoted_pawn_or_lance_steps()
    {
        // D3-031-12: 成り・捕獲・不成の歩兵と香車の着手だけを不可逆手とする
        // (第31条R1、scalashogi variant/Chushogi.scala isIrreversible)。
        let rules = Rules::ENGINE_DEFAULT;
        let generator = MoveGenerator::new(rules.moves);
        let cases = [
            (
                "unpromoted pawn step",
                piece(Color::Black, PieceKind::Pawn),
                step(sq(5, 4), sq(5, 5)),
                false,
                true,
            ),
            (
                "unpromoted lance advance",
                piece(Color::Black, PieceKind::Lance),
                step(sq(5, 4), sq(5, 6)),
                false,
                true,
            ),
            (
                "silver promotion entering enemy camp",
                piece(Color::Black, PieceKind::SilverGeneral),
                Move {
                    promote: true,
                    ..step(sq(5, 7), sq(5, 8))
                },
                false,
                true,
            ),
            (
                "rook capture",
                piece(Color::Black, PieceKind::Rook),
                step(sq(5, 4), sq(5, 5)),
                true,
                true,
            ),
            (
                "promoted pawn noncapture",
                PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
                step(sq(5, 4), sq(5, 5)),
                false,
                false,
            ),
            (
                "promoted lance noncapture",
                PieceCode::new_promoted(Color::Black, PieceKind::WhiteHorse).unwrap(),
                step(sq(5, 4), sq(5, 6)),
                false,
                false,
            ),
            (
                "lion pass",
                piece(Color::Black, PieceKind::Lion),
                step(sq(5, 4), sq(5, 4)),
                false,
                false,
            ),
            (
                "king step",
                piece(Color::Black, PieceKind::King),
                step(sq(5, 4), sq(5, 5)),
                false,
                false,
            ),
        ];
        for (name, mover, mv, captures, irreversible) in cases {
            let mut pieces = vec![
                (mv.from, mover),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ];
            if mover.kind() != Some(PieceKind::King) {
                pieces.push((sq(0, 0), piece(Color::Black, PieceKind::King)));
            }
            if captures {
                pieces.push((mv.to, piece(Color::White, PieceKind::SilverGeneral)));
            }
            let mut position = position(Color::Black, &pieces);
            let undo = position.try_make_move_with_undo(mv, &generator).unwrap();
            assert_eq!(move_is_irreversible(mv, &undo), irreversible, "{name}");
        }
    }

    #[test]
    fn article_31_r1_attacking_moves_extend_the_run_and_others_reset_it() {
        // D3-031-05: 攻撃連続数の更新は共有の純粋関数で行い、攻撃的着手は
        // 手番側の連続数を1増やし、非攻撃的着手は0へ戻す。相手側の値は
        // 変更しない(adjudication-refactor.md「R1の攻撃連続数」)。
        assert_eq!(
            updated_attacking_counters([2, 5], Color::Black, true),
            [3, 5]
        );
        assert_eq!(
            updated_attacking_counters([2, 5], Color::Black, false),
            [0, 5]
        );
        assert_eq!(
            updated_attacking_counters([2, 5], Color::White, true),
            [2, 6]
        );
        assert_eq!(
            updated_attacking_counters([2, 5], Color::White, false),
            [2, 0]
        );
        assert_eq!(
            updated_attacking_counters([0, 0], Color::Black, true),
            [1, 0]
        );
    }
}

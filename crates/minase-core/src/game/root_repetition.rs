//! 根の合法手を反復規則R1の終局裁定で分類する（RULES.md第31条R1）。

use std::collections::{HashMap, HashSet};

use super::{Game, GameResult, GameStatus, WinReason};
use crate::{Move, MoveGenerator, MoveRules, Position, RepetitionRule};

/// 根の合法手を反復規則R1の裁定で分類した結果（RULES.md第31条R1）。
pub struct RepetitionRootMoves {
    winning: Vec<Move>,
    losing: Vec<Move>,
}

impl RepetitionRootMoves {
    /// 反復勝ちの手：その着手の完了時にR1の裁定で自分の勝ちとなる手。
    pub fn winning(&self) -> &[Move] {
        &self.winning
    }

    /// 反復負けに直結する手：その着手の完了時にR1の裁定で自分の負けとなる手、
    /// またはその着手の後に相手がR1の裁定で自分を負けにできる応手を持つ手。
    pub fn losing(&self) -> &[Move] {
        &self.losing
    }
}

impl Game {
    /// 根の合法手を、反復勝ちの手と反復負けに直結する手に分類する。
    ///
    /// RULES.md第31条R1の対局が継続中の場合だけ、根から2手以内の裁定を調べる。
    /// R1以外の規則と終局済みの対局では、両方の一覧を空にする。
    /// 判定は写しに対する[`Self::play`]の結果だけに基づき、元の対局を変更しない。
    /// 各一覧は[`Self::legal_moves`]の順序を保ち、重複を含まない。
    ///
    /// 履歴中に3回以上現れた探索局面キーを使って候補を絞る。
    /// 応手が候補になる場合は、その直前の根の着手も写しへ適用する。
    /// 該当するキーがなければ、写しへの着手の適用も省く。
    pub fn repetition_root_moves(&self) -> RepetitionRootMoves {
        let mut result = RepetitionRootMoves {
            winning: Vec::new(),
            losing: Vec::new(),
        };
        if self.rules().repetition != RepetitionRule::R1 || self.status() != GameStatus::Ongoing {
            return result;
        }

        let mut occurrences = HashMap::<u64, usize>::new();
        for &key in self.repetition_key_history() {
            *occurrences.entry(key).or_insert(0) += 1;
        }
        let repeated: HashSet<_> = occurrences
            .into_iter()
            .filter_map(|(key, count)| (count >= 3).then_some(key))
            .collect();
        if repeated.is_empty() {
            return result;
        }

        let mover = self.position().side_to_move();
        let rules = self.rules().moves;
        let generator = MoveGenerator::new(rules);
        for mv in self.legal_moves() {
            let mut position = self.position().clone();
            position.make_move_unchecked(mv, rules);
            if !repeated.contains(&search_key(&position)) {
                // R1の局面合法手は、対局が継続中なら対局の合法手とも一致する。
                // 根の手が候補でなくても、応手だけが4回目の出現を作る場合を調べる。
                let mut replies = Vec::new();
                generator.generate_moves(&position, &mut replies);
                if !replies.into_iter().any(|reply| {
                    returns_to_repeated_position(&mut position, reply, rules, &repeated)
                }) {
                    continue;
                }
            }

            let after_move = played(self, mv);
            match after_move.status() {
                GameStatus::Finished(GameResult::Win {
                    winner,
                    reason: WinReason::Repetition,
                }) => {
                    if winner == mover {
                        result.winning.push(mv);
                    } else {
                        result.losing.push(mv);
                    }
                }
                GameStatus::Finished(_) => {}
                GameStatus::Ongoing => {
                    for reply in after_move.legal_moves() {
                        if !returns_to_repeated_position(&mut position, reply, rules, &repeated) {
                            continue;
                        }
                        if played(&after_move, reply).status()
                            == GameStatus::Finished(GameResult::Win {
                                winner: mover.opposite(),
                                reason: WinReason::Repetition,
                            })
                        {
                            result.losing.push(mv);
                            break;
                        }
                    }
                }
            }
        }
        result
    }
}

/// 適用と取り消しで候補を絞る。裁定の条件はここでは判定しない。
fn returns_to_repeated_position(
    position: &mut Position,
    mv: Move,
    rules: MoveRules,
    repeated: &HashSet<u64>,
) -> bool {
    let undo = position.make_move_unchecked(mv, rules);
    let repeated = repeated.contains(&search_key(position));
    position.unmake_move(undo);
    repeated
}

/// 同一局面の条件に成り権保留状態を含める（RULES.md第24条第1項d）。
fn search_key(position: &Position) -> u64 {
    #[cfg(test)]
    TEST_PROBES.with(|probes| {
        let (keys, plays) = probes.get();
        probes.set((keys + 1, plays));
    });
    position.zobrist() ^ position.rights_zobrist()
}

/// 合法手を対局の写しへ適用する。生成と受理が食い違えば内部の不整合である。
fn played(game: &Game, mv: Move) -> Game {
    #[cfg(test)]
    TEST_PROBES.with(|probes| {
        let (keys, plays) = probes.get();
        probes.set((keys, plays + 1));
    });
    let mut after_move = game.clone();
    after_move
        .play(mv)
        .expect("a generated legal move must be accepted by an ongoing R1 game");
    after_move
}

#[cfg(test)]
std::thread_local! {
    // 費用の契約を検査するための、キー計算回数とGame::play呼出し回数。
    pub(super) static TEST_PROBES: std::cell::Cell<(usize, usize)> = const {
        std::cell::Cell::new((0, 0))
    };
}

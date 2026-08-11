//! 評価関数を使って着手を選ぶ探索。

#[cfg(test)]
mod tests;
mod tt;

pub use tt::{DEFAULT_SIZE_MB as DEFAULT_TT_SIZE_MB, TranspositionTable};

use core::cmp::Ordering;

use crate::MoveGenerator;
use crate::core::mv::Move;
use crate::core::piece::PieceKind;
use crate::core::position::Position;
use crate::core::rules::Rules;
use crate::eval::{evaluate, piece_value};

use tt::Bound;

/// 詰みを表す評価値。
pub const MATE: i32 = 30_000;
/// 探索が扱う最大ply。
pub const MAX_PLY: u32 = 256;
/// 詰み手数を含む評価値の下限。
pub const MATE_THRESHOLD: i32 = MATE - MAX_PLY as i32;
/// 引き分けを表す評価値。
pub const DRAW_SCORE: i32 = 0;

const INFINITY: i32 = MATE + 1;

/// 1回の探索に適用する制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchConfig {
    /// 反復深化で完了を目指す最大深さ。
    pub depth: u32,
    /// 探索するノード数の上限。`None`なら上限を設けない。
    pub nodes: Option<u64>,
}

/// 完了した探索の結果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchResult {
    /// 選んだ着手。
    pub best_move: Move,
    /// 選んだ着手の評価値。
    pub score: i32,
    /// 最後まで完了した反復深化の深さ。
    pub depth: u32,
    /// 探索開始から訪問したノード数。
    pub nodes: u64,
}

/// 指定局面を反復深化で探索する。
///
/// `root_moves`には、対局管理層がR2・R3を含めて検査したルート合法手を
/// 渡す。`history_keys`の各要素は、対局開始から現局面までの
/// `Position::zobrist() ^ Position::rights_zobrist()`でなければならない。
///
/// ノード上限によって反復深化が中断された場合は、直前に完了した深さの
/// 結果を返す。深さ1の完了前に中断された場合も、`root_moves`の先頭を返す。
///
/// # Panics
///
/// `root_moves`が空、`config.depth`が0、または`config.depth`が
/// [`MAX_PLY`]を超える場合はpanicする。
pub fn search(
    position: &Position,
    rules: Rules,
    root_moves: &[Move],
    history_keys: &[u64],
    config: &SearchConfig,
    tt: &mut TranspositionTable,
) -> SearchResult {
    assert!(!root_moves.is_empty(), "root move list must not be empty");
    assert!(config.depth > 0, "search depth must be at least one");
    assert!(
        config.depth <= MAX_PLY,
        "search depth must not exceed {MAX_PLY}"
    );

    tt.new_search();
    let mut searcher = Searcher {
        rules,
        generator: MoveGenerator::new(rules),
        history_keys,
        path_keys: vec![search_key(position)],
        nodes: 0,
        node_limit: config.nodes,
        tt,
    };
    let mut result = SearchResult {
        best_move: root_moves[0],
        score: evaluate(position),
        depth: 0,
        nodes: 0,
    };

    for depth in 1..=config.depth {
        let Some((best_move, score)) = searcher.search_root(position, root_moves, depth) else {
            break;
        };
        result.best_move = best_move;
        result.score = score;
        result.depth = depth;
    }
    result.nodes = searcher.nodes;
    result
}

struct Searcher<'a> {
    rules: Rules,
    generator: MoveGenerator,
    history_keys: &'a [u64],
    path_keys: Vec<u64>,
    nodes: u64,
    node_limit: Option<u64>,
    tt: &'a mut TranspositionTable,
}

impl Searcher<'_> {
    fn search_root(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        depth: u32,
    ) -> Option<(Move, i32)> {
        if !self.enter_node() {
            return None;
        }

        let mut position = position.clone();
        let mut moves = root_moves.to_vec();
        let key = search_key(&position);
        let tt_move = self.tt.probe(key, 0).map(|hit| hit.best_move);
        order_moves(&position, &mut moves, tt_move);
        let mut alpha = -INFINITY;
        let beta = INFINITY;
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;

        for (index, mv) in moves.into_iter().enumerate() {
            let score = self.search_move(&mut position, mv, depth, alpha, beta, 0, index == 0)?;
            if score > best_score {
                best_score = score;
                best_move = mv;
            }
            alpha = alpha.max(score);
        }
        self.tt
            .store(key, depth, best_score, Bound::Exact, best_move, 0);
        Some((best_move, best_score))
    }

    fn negamax(
        &mut self,
        position: &mut Position,
        depth: u32,
        mut alpha: i32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        if !self.enter_node() {
            return None;
        }

        if depth == 0 {
            return Some(evaluate(position));
        }

        let key = search_key(position);
        let original_alpha = alpha;
        let mut tt_move = None;
        // 深さが足りるヒットは即時カットオフだけに使い、探索窓は狭めない。
        // 窓を狭めると、格納時のバウンド分類が実際に探索した窓と食い違う。
        if let Some(hit) = self.tt.probe(key, ply) {
            tt_move = Some(hit.best_move);
            if u32::from(hit.depth) >= depth {
                let cutoff = match hit.bound {
                    Bound::Exact => true,
                    Bound::Lower => hit.score >= beta,
                    Bound::Upper => hit.score <= alpha,
                };
                if cutoff {
                    return Some(hit.score);
                }
            }
        }
        let mut moves = Vec::new();
        self.generator.generate_moves(position, &mut moves);
        if moves.is_empty() {
            return Some(-MATE + ply as i32);
        }

        order_moves(position, &mut moves, tt_move);
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;
        let mut beta_cutoff = false;
        for (index, mv) in moves.into_iter().enumerate() {
            let score = self.search_move(position, mv, depth, alpha, beta, ply, index == 0)?;
            if score > best_score {
                best_score = score;
                best_move = mv;
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                beta_cutoff = true;
                break;
            }
        }
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if beta_cutoff {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.tt.store(key, depth, best_score, bound, best_move, ply);
        Some(best_score)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_move(
        &mut self,
        position: &mut Position,
        mv: Move,
        depth: u32,
        alpha: i32,
        beta: i32,
        ply: u32,
        first: bool,
    ) -> Option<i32> {
        if captures_last_royal(position, mv) {
            return self.enter_node().then_some(MATE - ply as i32);
        }

        let undo = position.make_move_unchecked(mv, self.rules);
        let key = search_key(position);
        let repeated = self.history_keys.contains(&key) || self.path_keys.contains(&key);
        if repeated {
            let score = self.enter_node().then_some(DRAW_SCORE);
            position.unmake_move(undo);
            return score;
        }

        self.path_keys.push(key);
        let mut score = if first {
            self.negamax(position, depth - 1, -beta, -alpha, ply + 1)
                .map(|value| -value)
        } else {
            self.negamax(position, depth - 1, -alpha - 1, -alpha, ply + 1)
                .map(|value| -value)
        };
        if !first && score.is_some_and(|value| value > alpha && value < beta) {
            score = self
                .negamax(position, depth - 1, -beta, -alpha, ply + 1)
                .map(|value| -value);
        }
        self.path_keys.pop();
        position.unmake_move(undo);
        score
    }

    fn enter_node(&mut self) -> bool {
        if self.node_limit.is_some_and(|limit| self.nodes >= limit) {
            return false;
        }
        self.nodes += 1;
        true
    }
}

fn search_key(position: &Position) -> u64 {
    position.zobrist() ^ position.rights_zobrist()
}

fn captures_last_royal(position: &Position, mv: Move) -> bool {
    let opponent = position.side_to_move().opposite();
    let royals = position.royal_pieces(opponent);
    let royal_count = royals.popcount();
    royal_count > 0
        && position
            .captured_squares(mv)
            .into_iter()
            .flatten()
            .filter(|&square| royals.contains(square))
            .count()
            == royal_count as usize
}

fn order_moves(position: &Position, moves: &mut [Move], tt_move: Option<Move>) {
    moves.sort_by(|left, right| compare_moves(position, *left, *right));
    if let Some(index) = tt_move.and_then(|tt_move| moves.iter().position(|&mv| mv == tt_move)) {
        moves.swap(0, index);
    }
}

fn compare_moves(position: &Position, left: Move, right: Move) -> Ordering {
    let left_key = move_order_key(position, left);
    let right_key = move_order_key(position, right);
    match (left_key, right_key) {
        (Some(left), Some(right)) => right
            .captured_value
            .cmp(&left.captured_value)
            .then_with(|| left.attacker_value.cmp(&right.attacker_value)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[derive(Clone, Copy)]
struct MoveOrderKey {
    captured_value: i32,
    attacker_value: i32,
}

fn move_order_key(position: &Position, mv: Move) -> Option<MoveOrderKey> {
    let captured_value: i32 = position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| piece_value(piece_kind_at(position, square)))
        .sum();
    (captured_value > 0).then(|| MoveOrderKey {
        captured_value,
        attacker_value: piece_value(piece_kind_at(position, mv.from)),
    })
}

fn piece_kind_at(position: &Position, square: crate::Square) -> PieceKind {
    position
        .piece_at(square)
        .and_then(crate::PieceCode::kind)
        .expect("move ordering square must contain a piece")
}

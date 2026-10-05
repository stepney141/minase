//! 根の探索と探索窓の拡大。

use crate::search::MATE_THRESHOLD;
use crate::search::events::StopReason;
use crate::search::snapshot::search_key;
use minase_core::mv::Move;
use minase_core::position::Position;

use super::searcher::Searcher;
use super::tt::Bound;
use super::{INFINITY, params};

/// 反復探索の窓と、次に外れた側を広げる幅。
///
/// `docs/plans/strength-stage6.md`の「窓の適用条件と拡大」節に従う。
pub(super) struct AspirationWindow {
    pub(super) alpha: i32,
    pub(super) beta: i32,
    pub(super) delta: i32,
}

impl AspirationWindow {
    /// 同じワーカーの直前の完了値から窓を作り、詰み帯に掛かる端を正規化する。
    pub(super) fn initial(depth: u32, prev: Option<i32>, delta: i32) -> Self {
        let (alpha, beta) = match prev {
            Some(score) if depth >= 5 && score.abs() < MATE_THRESHOLD => {
                (score - delta, score + delta)
            }
            _ => (-INFINITY, INFINITY),
        };
        let mut window = Self { alpha, beta, delta };
        window.normalize();
        window
    }

    /// 下側だけを広げ、次の拡大量に調整係数を掛ける。
    pub(super) fn widen_low(&mut self) {
        if self.alpha != -INFINITY {
            self.alpha -= self.delta;
        }
        self.normalize();
        self.delta = grow_aspiration_delta(self.delta);
    }

    /// 上側だけを広げ、次の拡大量に調整係数を掛ける。
    pub(super) fn widen_high(&mut self) {
        if self.beta != INFINITY {
            self.beta += self.delta;
        }
        self.normalize();
        self.delta = grow_aspiration_delta(self.delta);
    }

    /// 詰み帯の閾値へ達した端を無限へ置き換える。
    fn normalize(&mut self) {
        if self.alpha <= -MATE_THRESHOLD {
            self.alpha = -INFINITY;
        }
        if self.beta >= MATE_THRESHOLD {
            self.beta = INFINITY;
        }
    }
}

/// 直前の評価値から上下に取る初期窓幅を求める。
pub(super) fn aspiration_delta(pawn: i32) -> i32 {
    pawn * params::aspiration_delta() / 100
}

/// 窓幅の拡大を広い整数型で計算してから飽和させる。
pub(super) fn grow_aspiration_delta(delta: i32) -> i32 {
    let grown = i64::from(delta) * i64::from(params::aspiration_growth()) / 100;
    i32::try_from(grown).unwrap_or(i32::MAX)
}

/// 「途中結果の採用」（byoyomi-time-usage.md）で使う窓内の最大値と主変化。
pub(super) struct PartialResult {
    pub(super) score: i32,
    pub(super) pv: Vec<Move>,
}

/// 「途中結果の採用」（byoyomi-time-usage.md）に必要な主ワーカーの記録。
#[derive(Default)]
pub(super) struct RootResults {
    pub(super) previous_best: Option<Move>,
    previous_completed: bool,
    original_alpha: i32,
    best: Option<PartialResult>,
}

impl RootResults {
    /// 「途中結果の採用」に従い、読み直す窓へ前の窓の結果を持ち越さない。
    pub(super) fn begin_window(&mut self, alpha: i32) {
        self.previous_completed = false;
        self.original_alpha = alpha;
        self.best = None;
    }

    /// 「途中結果の採用」に従い、完了した手のうち上界でない最大値だけを保持する。
    pub(super) fn record(&mut self, mv: Move, score: i32, alpha: i32, pv: &[Move]) {
        self.previous_completed |= self.previous_best == Some(mv);
        if score > alpha && self.best.as_ref().is_none_or(|best| score > best.score) {
            let mut line = Vec::with_capacity(pv.len() + 1);
            line.push(mv);
            line.extend_from_slice(pv);
            self.best = Some(PartialResult { score, pv: line });
        }
    }

    /// 「途中結果の採用」に従い、hardで中断し直前の最善手が完了した窓だけを採る。
    pub(super) fn partial_result(&self, reason: Option<StopReason>) -> Option<&PartialResult> {
        if reason != Some(StopReason::HardLimit) || !self.previous_completed {
            return None;
        }
        self.best
            .as_ref()
            .filter(|best| best.score > self.original_alpha)
    }
}

impl Searcher<'_> {
    /// 「途中結果の採用」（byoyomi-time-usage.md）に従い、直前の最善手だけを先頭へ移す。
    pub(super) fn order_root_moves(&self, position: &Position, moves: &mut [Move]) {
        let tt_move = self
            .tt
            .probe(search_key(position), 0)
            .and_then(|hit| hit.best_move);
        self.order_moves(position, moves, tt_move, 0);
        if let Some(previous) = self.root_results.as_ref().and_then(|r| r.previous_best) {
            let index = moves
                .iter()
                .position(|&mv| mv == previous)
                .expect("previous best must be a legal root move");
            moves[..=index].rotate_right(1);
        }
    }

    /// 窓を広げながら同じ深さを読み直し、窓内で完了した結果だけを返す。
    ///
    /// `docs/plans/strength-stage6.md`の「aspiration windows」節に従い、
    /// 主・補助ワーカーが共有する。読み直し中の中断も`None`を返す。
    pub(super) fn search_iteration(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        depth: u32,
        prev: Option<i32>,
    ) -> Option<(Move, i32)> {
        let mut window =
            AspirationWindow::initial(depth, prev, aspiration_delta(self.pst.pawn_value()));
        loop {
            let (best_move, score) =
                self.search_root(position, root_moves, depth, window.alpha, window.beta)?;
            if score <= window.alpha {
                window.widen_low();
            } else if score >= window.beta {
                window.widen_high();
            } else {
                if let Some(results) = &mut self.root_results {
                    results.previous_best = Some(best_move);
                }
                return Some((best_move, score));
            }
        }
    }

    /// ルート局面を指定深さで探索し、最善手と評価値を返す。
    ///
    /// `docs/plans/strength-stage6.md`の「根の探索の窓化」節に従い、
    /// β以上で打ち切り、入力時の窓に対する上界・下界・正確な値を記録する。
    /// 中断された場合は`None`を返し、停止条件を記録する。
    pub(super) fn search_root(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        depth: u32,
        mut alpha: i32,
        beta: i32,
    ) -> Option<(Move, i32)> {
        if let Some(results) = &mut self.root_results {
            results.begin_window(alpha);
        }
        self.pv[0].clear();

        let mut position = position.clone();
        let mut moves = root_moves.to_vec();
        let key = search_key(&position);
        self.order_root_moves(&position, &mut moves);
        let original_alpha = alpha;
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;

        for (index, mv) in moves.into_iter().enumerate() {
            let score =
                self.search_move(&mut position, mv, depth, alpha, beta, 0, index == 0, 0)?;
            if let Some(results) = &mut self.root_results {
                results.record(mv, score, alpha, &self.pv[1]);
            }
            if score > best_score {
                best_score = score;
                best_move = mv;
                self.update_pv(0, mv);
            }
            alpha = alpha.max(score);
            if best_score >= beta {
                break;
            }
        }
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.tt
            .store(key, depth, best_score, bound, Some(best_move), 0);
        Some((best_move, best_score))
    }
}

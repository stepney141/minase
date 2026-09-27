//! 根の探索と探索窓の拡大。

use crate::core::mv::Move;
use crate::core::position::Position;
use crate::search::MATE_THRESHOLD;
use crate::search::snapshot::search_key;

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

impl Searcher<'_> {
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
        self.pv[0].clear();

        let mut position = position.clone();
        let mut moves = root_moves.to_vec();
        let key = search_key(&position);
        let tt_move = self.tt.probe(key, 0).and_then(|hit| hit.best_move);
        self.order_moves(&position, &mut moves, tt_move, 0);
        let original_alpha = alpha;
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;

        for (index, mv) in moves.into_iter().enumerate() {
            let score =
                self.search_move(&mut position, mv, depth, alpha, beta, 0, index == 0, 0)?;
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

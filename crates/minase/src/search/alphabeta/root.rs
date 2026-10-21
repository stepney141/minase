//! 根の探索と探索窓の拡大。

use crate::search::REPETITION_BAND_START;
use crate::search::events::StopReason;
use crate::search::snapshot::search_key;
use minase_core::mv::Move;
use minase_core::position::Position;

use super::correction::correction_from_weighted_sum;
use super::negamax::{SEARCHED_LIST_CAPACITY, should_record_searched_move};
use super::ordering::{HistoryUpdateCoefficients, move_order_key};
use super::pruning::{
    LmrContext, applies_lmr, corrected_static_eval, is_seek_mate, lmr_depth, lmr_history_value,
    lmr_reduction, pruning_eval,
};
use super::searcher::Searcher;
use super::tt::{Bound, Hit};
use super::{INFINITY, params};

/// 反復探索の窓と、次に外れた側を広げる幅。
///
/// 前の反復の値の周囲から探索を始めるaspiration windowである。
/// `delta`は静的評価と同じ単位で、初期値は歩兵価値×`AspirationDelta`%とする。
/// 外れた側だけを広げる理由は`docs/adr/widen-only-the-failed-side-of-aspiration-windows.md`にある。
pub(super) struct AspirationWindow {
    pub(super) alpha: i32,
    pub(super) beta: i32,
    pub(super) delta: i32,
}

impl AspirationWindow {
    /// 深さ5以上で同じワーカーの直前の完了値が詰み帯と専用の帯の外なら、その値±`delta`を初期窓にする。
    /// それ以外は全窓とし、詰み帯または専用の帯に達した端は無限へ置き換える。
    pub(super) fn initial(depth: u32, prev: Option<i32>, delta: i32) -> Self {
        let (alpha, beta) = match prev {
            Some(score) if depth >= 5 && score.abs() < REPETITION_BAND_START => {
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

    /// 詰み帯または専用の帯へ達した端を無限へ置き換える。
    fn normalize(&mut self) {
        if self.alpha <= -REPETITION_BAND_START {
            self.alpha = -INFINITY;
        }
        if self.beta >= REPETITION_BAND_START {
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

/// 中断時の着手選択に使う、探索窓のαを上回った最大値とその主変化。
pub(super) struct PartialResult {
    pub(super) score: i32,
    pub(super) pv: Vec<Move>,
}

/// 秒読みつき時計だけで探索する主ワーカーが保持する、途中結果の採用材料。
/// 適用対象は[`super::time::byoyomi_clock`]、採用条件は[`Self::partial_result`]に従う。
#[derive(Default)]
pub(super) struct RootResults {
    pub(super) previous_best: Option<Move>,
    previous_completed: bool,
    original_alpha: i32,
    best: Option<PartialResult>,
}

impl RootResults {
    /// 読み直しの窓を開始し、前の窓の結果と直前の最善手の完了記録を消す。
    pub(super) fn begin_window(&mut self, alpha: i32) {
        self.previous_completed = false;
        self.original_alpha = alpha;
        self.best = None;
    }

    /// 完了した手のうち探索時のαを上回る最大値と主変化を保持し、直前の最善手の完了も記録する。
    pub(super) fn record(&mut self, mv: Move, score: i32, alpha: i32, pv: &[Move]) {
        self.previous_completed |= self.previous_best == Some(mv);
        if score > alpha && self.best.as_ref().is_none_or(|best| score > best.score) {
            let mut line = Vec::with_capacity(pv.len() + 1);
            line.push(mv);
            line.extend_from_slice(pv);
            self.best = Some(PartialResult { score, pv: line });
        }
    }

    /// hard limitで中断し、直前の最善手の探索が完了した窓から、開始時のαを上回る途中結果を返す。
    /// 窓全体の完了は要しないが、上界しか得られなかった手と他の停止理由は採用しない。
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
    /// 先頭手を保ち、残りを前回完了反復の部分木ノード数の降順に安定整列する。
    ///
    /// 先頭手は通常の整列で優先した置換表の手であり、`root_results`に直前の最善手があればそちらを優先する。
    /// どちらもなければ[`Self::order_moves`]の先頭を保ち、前回ノード数がなければその整列順のままとする。
    /// 評価値で整列しない理由は`docs/adr/order-root-moves-by-previous-subtree-nodes.md`にある。
    pub(super) fn order_root_moves(&self, position: &Position, moves: &mut [Move]) -> Option<Hit> {
        let tt_hit = self.tt.probe(search_key(position), 0);
        let tt_move = tt_hit.and_then(|hit| hit.best_move);
        self.order_moves(position, moves, tt_move, 0);
        if let Some(previous) = self.root_results.as_ref().and_then(|r| r.previous_best) {
            let index = moves
                .iter()
                .position(|&mv| mv == previous)
                .expect("previous best must be a legal root move");
            moves[..=index].rotate_right(1);
        }
        if !self.completed_root_nodes.is_empty() && moves.len() > 1 {
            moves[1..].sort_by_key(|mv| {
                std::cmp::Reverse(
                    self.completed_root_nodes
                        .iter()
                        .find(|(root_move, _)| root_move == mv)
                        .map_or(0, |&(_, nodes)| nodes),
                )
            });
        }
        tt_hit
    }

    /// 窓を広げながら同じ深さを読み直し、窓内で完了した結果だけを返す。
    ///
    /// 主ワーカーと補助ワーカーが共有し、直前の完了値は各ワーカー自身のものを使う。
    /// 値がα以下なら下側だけ、β以上なら上側だけを広げ、同じ深さで再探索する。
    /// 等号も窓外れとし、fail-lowの最善手は置換表の助言としてだけ残して、完了結果や進捗へは採らない。
    /// 読み直し中の中断も`None`を返す。
    pub(super) fn search_iteration(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        depth: u32,
        prev: Option<i32>,
    ) -> Option<(Move, i32)> {
        self.seek_mate = is_seek_mate(prev, depth, self.nnue.pawn_value());
        self.previous_pv.clone_from(&self.pv[0]);
        let mut window =
            AspirationWindow::initial(depth, prev, aspiration_delta(self.nnue.pawn_value()));
        loop {
            let Some((best_move, score)) =
                self.search_root(position, root_moves, depth, window.alpha, window.beta)
            else {
                self.current_root_nodes.clear();
                return None;
            };
            if score <= window.alpha {
                window.widen_low();
            } else if score >= window.beta {
                window.widen_high();
            } else {
                self.completed_root_nodes = std::mem::take(&mut self.current_root_nodes);
                if let Some(results) = &mut self.root_results {
                    results.previous_best = Some(best_move);
                }
                return Some((best_move, score));
            }
        }
    }

    /// ルート局面を指定深さで探索し、最善手と評価値を返す。
    ///
    /// β以上で打ち切って下界、入力時のα以下なら上界、それ以外は正確な値として置換表に記録する。
    /// 根は常に主変化（PV）ノードとして扱い、前向き枝刈りを行わない。
    /// 根のcutNodeは偽とする（Stockfish src/search.cpp:412）。
    /// 置換表に補正前の静的評価があれば、評価関数の出力を求めずに使う（Stockfish src/search.cpp:854-860）。
    /// 照会に当たらなければ、求めた補正前の静的評価をPVの探索値なしの記録として直ちに保存する
    /// （Stockfish src/search.cpp:870-874）。照合した記録に静的評価がない場合は保存しない。
    /// 根の先頭以外の手は残り深さ2以上でLMRを適用し、子ノードの枝刈りも通常どおり行う。
    /// 完了した探索で最善値が開始時のαを上回れば、根でもhistoryを1回更新する。
    /// 中断された場合は`None`を返し、停止条件を記録する。
    pub(super) fn search_root(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        depth: u32,
        mut alpha: i32,
        beta: i32,
    ) -> Option<(Move, i32)> {
        #[cfg(feature = "invariants")]
        assert!(
            self.nmp_min_ply.is_none(),
            "null move verification state must not reach the root"
        );
        self.root_depth = depth;
        self.follow_pv[0] = true;
        // 根も2手先のβ打ち切り数を初期化する（Stockfish src/search.cpp:829）。
        self.cutoff_counts[2] = 0;
        self.stat_scores[0] = 0;
        // 根が読む子のnull move打ち切り回数を初期化する（Stockfish src/search.cpp:830）。
        self.prior_nmp_fail_high[1] = 0;
        if let Some(results) = &mut self.root_results {
            results.begin_window(alpha);
        }
        self.pv[0].clear();
        self.tt_pv[0] = true;

        let mut position = position.clone();
        let mut moves = root_moves.to_vec();
        let key = search_key(&position);
        let tt_hit = self.order_root_moves(&position, &mut moves);
        self.tt_hits[0] = tt_hit.is_some();
        // 根の静的評価も子孫のimprovingの比較に保持する（Stockfish src/search.cpp:846-884）。
        #[cfg(feature = "invariants")]
        self.nnue
            .assert_accumulator(&position, &self.accumulators[0], 0);
        let side = position.side_to_move();
        let raw = if let Some(static_eval) = tt_hit.and_then(|hit| hit.static_eval) {
            #[cfg(feature = "search-stats")]
            {
                self.stats.tt_static_eval_uses += 1;
            }
            static_eval
        } else {
            let raw = self.nnue.evaluate_accumulator(&self.accumulators[0], side);
            if tt_hit.is_none() && self.tt.store_static_eval(key, raw, true) {
                #[cfg(feature = "search-stats")]
                {
                    self.stats.tt_static_eval_stores += 1;
                }
            }
            raw
        };
        let correction_weighted_sum = self.correction_weighted_sum_at(side, 0);
        let correction =
            correction_from_weighted_sum(correction_weighted_sum, self.nnue.pawn_value());
        let static_eval = corrected_static_eval(raw, correction);
        self.static_evals[0] = static_eval;
        let eval = pruning_eval(
            static_eval,
            tt_hit
                .and_then(|hit| hit.value)
                .map(|value| (value.score, value.bound)),
        );
        let improving = static_eval >= beta;
        let original_alpha = alpha;
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;
        let root_tt_move = moves[0];
        let tt_capture = move_order_key(&position, self.nnue, root_tt_move).is_some();
        let mut best_capture = false;
        let mut searched_captures = Vec::with_capacity(SEARCHED_LIST_CAPACITY);
        let mut searched_quiets = Vec::with_capacity(SEARCHED_LIST_CAPACITY);

        for (index, mv) in moves.into_iter().enumerate() {
            self.move_counts[0] = index + 1;
            // 根も実際に読む各手のstatScoreを子の探索前に記録する（src/search.cpp:1371-1378）。
            let capture = move_order_key(&position, self.nnue, mv).is_some();
            let stat_score = self.move_stat_score(&position, mv, capture, 0);
            self.stat_scores[0] = stat_score;
            // Stockfishは根の並べ替え後の先頭手を置換表の手として扱う
            // （src/search.cpp:839-840）。
            let lmr = applies_lmr(depth, index);
            let first_depth = if lmr {
                let reduction = lmr_reduction(LmrContext {
                    depth,
                    index,
                    improving,
                    pv_node: true,
                    tt_pv: true,
                    cut_node: false,
                    tt_hit: tt_hit
                        .and_then(|hit| hit.value)
                        .map(|value| (value.score, u32::from(value.depth))),
                    tt_capture: Some(tt_capture),
                    is_tt_move: mv == root_tt_move,
                    capture,
                    stat_score,
                    history: lmr_history_value(
                        capture,
                        self.history[side.index()][mv.from.dense_index()][mv.to.dense_index()],
                    ),
                    cutoff_count: self.cutoff_counts[1],
                    alpha,
                    eval,
                    pawn: self.nnue.pawn_value(),
                    correction_weighted_sum,
                    seek_mate_gives_check: self.lmr_seek_mate_gives_check(&position, mv),
                });
                lmr_depth(depth, reduction, true, 0, self.root_depth)
            } else {
                depth - 1
            };
            let alpha_before = alpha;
            let before = self.nodes;
            let score = self.search_move(
                &mut position,
                mv,
                depth,
                alpha,
                beta,
                0,
                index == 0,
                first_depth,
                lmr.then_some(best_score),
                false,
            );
            let nodes = self.nodes - before;
            if let Some((_, total)) = self.current_root_nodes.iter_mut().find(|(m, _)| *m == mv) {
                *total += nodes;
            } else {
                self.current_root_nodes.push((mv, nodes));
            }
            let score = score?;
            if let Some(results) = &mut self.root_results {
                results.record(mv, score, alpha, &self.pv[1]);
            }
            if score > best_score {
                best_score = score;
                best_move = mv;
                best_capture = capture;
                self.update_pv(0, mv);
            }
            alpha = alpha.max(score);
            if best_score >= beta {
                self.cutoff_counts[0] += 1;
                break;
            }
            if should_record_searched_move(index + 1, score, alpha_before) {
                if capture {
                    searched_captures.push(mv);
                } else {
                    searched_quiets.push(mv);
                }
            }
        }
        if best_score > original_alpha {
            let updates = HistoryUpdateCoefficients::current().updates(
                depth,
                best_move == root_tt_move,
                0,
                false,
                searched_quiets.len() + searched_captures.len(),
            );
            self.update_best_move_histories(
                &position,
                best_move,
                best_capture,
                &searched_quiets,
                &searched_captures,
                updates,
            );
        }
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if best_score >= beta {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.update_correction(
            &position,
            depth,
            0,
            best_score,
            bound,
            best_capture,
            static_eval,
            None,
        );
        self.tt.store(
            key,
            depth,
            best_score,
            Some(raw),
            bound,
            Some(best_move),
            0,
            true,
        );
        Some((best_move, best_score))
    }
}

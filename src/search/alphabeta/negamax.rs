//! 内部ノードのαβ探索。

use crate::core::mv::Move;
use crate::core::position::Position;
use crate::search::snapshot::search_key;
use crate::search::{DRAW_SCORE, MATE, MATE_THRESHOLD};

use super::INFINITY;
use super::correction::key_after_move;
use super::pruning::{futility_margin, lmr_reduction, null_move_reduction, see_margin};
use super::royal::{captures_last_royal, royal_under_attack};
use super::searcher::Searcher;
use super::see::see_prunes;
use super::tt::Bound;

impl Searcher<'_> {
    /// ネガマックス形式のアルファベータ探索で局面を評価する。
    ///
    /// 深さ0では静止探索へ移り、合法手のない局面は詰みとして
    /// `-MATE + ply`を返す。中断された場合は`None`を返す。
    pub(super) fn negamax(
        &mut self,
        position: &mut Position,
        depth: u32,
        mut alpha: i32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        self.pv[ply as usize].clear();

        if depth == 0 {
            return self.quiesce(position, alpha, beta, ply);
        }

        let key = search_key(position);
        let original_alpha = alpha;
        let mut tt_move = None;
        // 深さが足りるヒットは即時カットオフだけに使い、探索窓は狭めない。
        // 窓を狭めると、格納時のバウンド分類が実際に探索した窓と食い違う。
        if let Some(hit) = self.tt.probe(key, ply) {
            tt_move = hit.best_move;
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

        // docs/plans/strength-stage6.md「internal iterative reduction」節。
        // 即時打ち切りを要求深さで判定した後、記録手がなければ1だけ浅く読む。
        let depth = if depth >= 3 && tt_move.is_none() {
            depth - 1
        } else {
            depth
        };

        let side = position.side_to_move();
        let has_non_royal_piece =
            !(position.pieces_of(side) & !position.royal_pieces(side)).is_empty();
        if depth >= 3
            && self.null_move_ply != Some(ply)
            && beta.abs() < MATE_THRESHOLD
            && has_non_royal_piece
        {
            let reduction = null_move_reduction(depth);
            let lion_before = position
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square);
            let undo = position.make_null_move();
            self.accumulators[(ply + 1) as usize] = self
                .pst
                .update_accumulator_after_null(self.accumulators[ply as usize], lion_before);
            self.material_keys[(ply + 1) as usize] = self.material_keys[ply as usize];
            let previous_null_move_ply = self.null_move_ply.replace(ply + 1);
            let score = self
                .negamax(
                    position,
                    depth.saturating_sub(1 + reduction),
                    -beta,
                    -beta + 1,
                    ply + 1,
                )
                .map(|value| -value);
            self.null_move_ply = previous_null_move_ply;
            position.unmake_null_move(undo);
            let score = score?;
            if score >= beta {
                return Some(if score.abs() >= MATE_THRESHOLD {
                    beta
                } else {
                    score
                });
            }
        }

        // docs/plans/strength-stage9.md「評価の償却」節。
        // 静的評価は必要時にだけ計算し、補正履歴の更新でも再利用する。
        let mut static_eval = None;
        // docs/plans/strength-stage4.mdの「適用するノード」「futility pruning」節。
        // 静的評価と余裕値の和は対象ノードで1回だけ求める。
        let futility_bound = (depth <= 3
            && beta - alpha == 1
            && alpha.abs() < MATE_THRESHOLD
            && beta.abs() < MATE_THRESHOLD)
            .then(|| {
                let static_eval = *static_eval.get_or_insert_with(|| {
                    self.pst.evaluate_accumulator(
                        self.accumulators[ply as usize],
                        position.side_to_move(),
                    )
                });
                let margin = futility_margin(self.pst.pawn_value(), depth);
                static_eval + self.correction.read(side, self.material_keys[ply as usize]) + margin
            });
        let mut royal_attacked = None;
        self.move_pickers[ply as usize].reset(tt_move, self.killers[ply as usize]);
        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_capture = false;
        let mut beta_cutoff = false;
        let mut index = 0;
        while let Some((mv, capture)) =
            self.move_pickers[ply as usize].next(position, self.pst, &self.generator, &self.history)
        {
            // 同「展開しない手の範囲」。負の詰み帯を脱するまでは安全な手を探す。
            // 王駒への利きは他の条件が揃ったときにだけ調べ、ノード内で再利用する。
            if best_score > -MATE_THRESHOLD
                && futility_bound.is_some_and(|bound| bound <= alpha)
                && Some(mv) != tt_move
                && !mv.promote
                && !capture
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            {
                index += 1;
                continue;
            }
            // docs/plans/strength-stage8.md「SEEによる捕獲手の枝刈り」節。
            // futilityと対象ノードおよび王駒への利きの遅延評価を共有する。
            if futility_bound.is_some()
                && best_score > -MATE_THRESHOLD
                && capture
                && Some(mv) != tt_move
                && !captures_last_royal(position, mv)
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                && see_prunes(
                    position,
                    self.rules,
                    self.pst,
                    mv,
                    see_margin(self.pst.pawn_value(), depth),
                )
            {
                index += 1;
                continue;
            }
            let reduction = if Some(mv) != tt_move
                && !capture
                && !self.killers[ply as usize][..].contains(&Some(mv))
            {
                let history =
                    self.history[side.index()][mv.from.dense_index()][mv.to.dense_index()];
                lmr_reduction(depth, index, history)
            } else {
                0
            };
            let score =
                self.search_move(position, mv, depth, alpha, beta, ply, index == 0, reduction)?;
            if score > best_score {
                best_score = score;
                best_move = Some(mv);
                best_capture = capture;
                self.update_pv(ply, mv);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                beta_cutoff = true;
                if !capture {
                    self.record_quiet_beta_cutoff(position, mv, depth, ply);
                }
                break;
            }
            index += 1;
        }
        let Some(best_move) = best_move else {
            return Some(-MATE + ply as i32);
        };
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if beta_cutoff {
            Bound::Lower
        } else {
            Bound::Exact
        };
        // 段階8の変種B。補正前の評価と保存値が補正の向きを確定するときだけ学習する。
        if best_score.abs() < MATE_THRESHOLD && !best_capture {
            let static_eval = *static_eval.get_or_insert_with(|| {
                self.pst
                    .evaluate_accumulator(self.accumulators[ply as usize], position.side_to_move())
            });
            if (bound == Bound::Exact
                || (bound == Bound::Upper && best_score < static_eval)
                || (bound == Bound::Lower && best_score > static_eval))
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            {
                self.correction.update(
                    side,
                    self.material_keys[ply as usize],
                    best_score - static_eval,
                    depth,
                );
            }
        }
        self.tt
            .store(key, depth, best_score, bound, Some(best_move), ply);
        Some(best_score)
    }

    /// 1手を適用して子局面を探索し、この局面から見た評価値を返す。
    ///
    /// 王駒をすべて取る手は子局面での終局値`MATE - (ply + 1)`を返す。
    /// 対局履歴または探索経路と同一の局面は引き分け値とする。
    /// 2手目以降は零窓で探索する。減深した探索がαを超えた場合は通常深さの零窓、
    /// さらに窓内なら全窓で再探索する。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn search_move(
        &mut self,
        position: &mut Position,
        mv: Move,
        depth: u32,
        alpha: i32,
        beta: i32,
        ply: u32,
        first: bool,
        reduction: u32,
    ) -> Option<i32> {
        self.pv[(ply + 1) as usize].clear();
        if captures_last_royal(position, mv) {
            return Some(MATE - (ply + 1) as i32);
        }

        if !self.enter_node() {
            return None;
        }
        let undo = position.make_move_unchecked(mv, self.rules);
        let key = search_key(position);
        let repeated = self.history_keys.contains(&key) || self.path_keys.contains(&key);
        if repeated {
            position.unmake_move(undo);
            return Some(DRAW_SCORE);
        }

        self.accumulators[(ply + 1) as usize] = self.pst.update_accumulator_after_move(
            self.accumulators[ply as usize],
            position,
            &undo,
        );
        self.material_keys[(ply + 1) as usize] =
            key_after_move(self.material_keys[ply as usize], &undo);
        self.path_keys.push(key);
        let mut score = if first {
            self.negamax(position, depth - 1, -beta, -alpha, ply + 1)
                .map(|value| -value)
        } else {
            self.negamax(position, depth - 1 - reduction, -alpha - 1, -alpha, ply + 1)
                .map(|value| -value)
        };
        if !first && reduction > 0 && score.is_some_and(|value| value > alpha) {
            score = self
                .negamax(position, depth - 1, -alpha - 1, -alpha, ply + 1)
                .map(|value| -value);
        }
        if !first && score.is_some_and(|value| value > alpha && value < beta) {
            score = self
                .negamax(position, depth - 1, -beta, -alpha, ply + 1)
                .map(|value| -value);
        }
        self.path_keys.pop();
        position.unmake_move(undo);
        score
    }
}

//! 内部ノードのαβ探索。

use crate::search::snapshot::search_key;
use crate::search::{
    MATE, REPETITION_BAND_START, REPETITION_WIN, SearchRepetition, in_repetition_band,
};
use minase_core::mv::Move;
use minase_core::piece::Color;
use minase_core::position::Position;
use minase_core::repetition_key;

use super::INFINITY;
use super::correction::key_after_move;
use super::pruning::{
    futility_margin, late_move_limit, lmr_reduction, null_move_reduction, razoring_margin,
    reverse_futility_margin, see_margin,
};
use super::royal::{captures_last_royal, royal_under_attack};
use super::searcher::{PathMove, Searcher};
use super::see::see_prunes;
use super::tt::Bound;

impl Searcher<'_> {
    /// 同じ手番の2手前より補正前の静的評価が上がったかを返す。
    ///
    /// 2手前がない場合と比較区間にnull moveを含む場合は余裕値を縮めない。
    pub(super) fn improving(&self, static_eval: i32, side: Color, ply: u32) -> bool {
        ply < 2
            || self.null_move_ply == Some(ply)
            || self.null_move_ply == Some(ply - 1)
            || static_eval
                > self
                    .nnue
                    .evaluate_accumulator(&self.accumulators[(ply - 2) as usize], side)
    }

    /// ネガマックス形式のアルファベータ探索で局面を評価する。
    ///
    /// 深さ0では[`Self::quiesce`]へ移り、合法手のない局面は第23条により`-MATE + ply`を返す。
    /// R2・R3で全合法手が禁止手なら`-(REPETITION_WIN - ply)`を返し、どちらも置換表へ保存しない。
    /// 禁止は履歴と経路に依存するため、別の経路で負けの値が再利用される詰みの帯を使わない。
    /// 中断された場合は`None`を返す。
    ///
    /// 置換表の専用の帯の値は経路に依存するため打ち切りに使わず、記録手だけを使う。
    /// 保存された詰みの距離と読み出し位置の手数の和が256を超える深い詰みの値も、
    /// 補正後に専用の帯へ入ると打ち切りに使われなくなる。保存形式に伴う既知の制約とする。
    /// 禁止手を除いて求めた通常の評価値は、履歴と経路に依存するまま置換表へ保存する近似を残す。
    ///
    /// 置換表で打ち切れず、要求深さ3以上で記録手がなければ、内部反復減深（IIR）として1だけ浅く読む。
    /// stand-patだけの記録も「記録手なし」に含める。
    /// 判断の理由は`docs/adr/reduce-depth-by-one-without-tt-move.md`にある。
    ///
    /// 前向き枝刈りはreverse futility、razoring、null move、futility、late move pruning、SEE、LMRの順で判定する。
    /// このうちreverse futility、razoring、futility、late move pruning、SEEは、IIR後の残り深さ3以下、零窓、αとβが詰み帯と専用の帯の外という共通条件を持つ。
    /// さらに手番側のどの王駒にも相手の疑似利きが届かない場合に限り、razoringは残り深さ2以下に限る。
    /// null moveとLMRの条件はそれぞれの判定箇所と[`super::pruning::lmr_reduction`]に従う。
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
        #[cfg(feature = "search-stats")]
        {
            self.stats.normal_nodes += 1;
        }

        let key = search_key(position);
        let original_alpha = alpha;
        let mut tt_move = None;
        // 深さが足りるヒットは即時カットオフだけに使い、探索窓は狭めない。
        // 窓を狭めると、格納時のバウンド分類が実際に探索した窓と食い違う。
        #[cfg(feature = "search-stats")]
        {
            self.stats.tt_probes += 1;
        }
        if let Some(hit) = self.tt.probe(key, ply) {
            #[cfg(feature = "search-stats")]
            {
                self.stats.tt_hits += 1;
            }
            tt_move = hit.best_move;
            if !in_repetition_band(hit.score) && u32::from(hit.depth) >= depth {
                let cutoff = match hit.bound {
                    Bound::Exact => true,
                    Bound::Lower => hit.score >= beta,
                    Bound::Upper => hit.score <= alpha,
                };
                if cutoff {
                    #[cfg(feature = "search-stats")]
                    {
                        self.stats.tt_cutoffs += 1;
                    }
                    return Some(hit.score);
                }
            }
        }

        // 置換表の打ち切りは要求深さで判定し、その後にIIRを適用する。
        let depth = if depth >= 3 && tt_move.is_none() {
            depth - 1
        } else {
            depth
        };

        // 浅い枝刈りの共通条件を判定し、累算値から補正前の静的評価を1回だけ求める。
        // 王駒への利きは個々の枝刈りが発動し得る場合に調べる。
        let pruning_node = depth <= 3
            && beta - alpha == 1
            && alpha.abs() < REPETITION_BAND_START
            && beta.abs() < REPETITION_BAND_START;
        let mut static_eval = pruning_node.then(|| {
            #[cfg(feature = "invariants")]
            self.nnue
                .assert_accumulator(position, &self.accumulators[ply as usize], ply);
            self.nnue
                .evaluate_accumulator(&self.accumulators[ply as usize], position.side_to_move())
        });
        let mut royal_attacked = None;
        // reverse futilityは「静的評価 − 余裕値 ≥ β」で静的評価を返し、置換表へ保存しない。
        // razoringは深さ2以下で「静的評価 + 余裕値 ≤ α」なら静止探索へ移り、その値もα以下なら返す。
        // 両者とも王駒に疑似利きが届くノードを除く理由は
        // docs/adr/skip-static-eval-pruning-when-a-royal-is-attacked.md にある。
        if let Some(value) = static_eval {
            if value - reverse_futility_margin(self.nnue.pawn_value()) >= beta
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            {
                return Some(value);
            }
            if depth <= 2
                && value + razoring_margin(self.nnue.pawn_value(), depth) <= alpha
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            {
                let value = self.quiesce(position, alpha, beta, ply)?;
                if value <= alpha {
                    return Some(value);
                }
            }
        }

        let side = position.side_to_move();
        let has_non_royal_piece =
            !(position.pieces_of(side) & !position.royal_pieces(side)).is_empty();
        // 深さ3以上、直前がnull moveでない、βが詰み帯と専用の帯の外、王駒以外の自駒がある場合に試す。
        // 王駒への利き、零窓、静的評価がβ以上という条件は加えない。
        // null_move_reductionで求めた減深量と着手相当の1手を引き、深さ0で飽和させる。
        if depth >= 3
            && self.null_move_ply != Some(ply)
            && beta.abs() < REPETITION_BAND_START
            && has_non_royal_piece
        {
            let value = *static_eval.get_or_insert_with(|| {
                #[cfg(feature = "invariants")]
                self.nnue
                    .assert_accumulator(position, &self.accumulators[ply as usize], ply);
                self.nnue
                    .evaluate_accumulator(&self.accumulators[ply as usize], side)
            });
            let reduction = null_move_reduction(depth, value, beta, self.nnue.pawn_value());
            let score =
                self.search_null_move(position, depth.saturating_sub(1 + reduction), beta, ply)?;
            if score >= beta {
                return Some(null_move_cutoff_score(score, beta));
            }
        }

        // futilityだけにcorrection historyの補正を加え、余裕値との和を1回だけ求める。
        let futility_bound = pruning_node.then(|| {
            let value = static_eval.expect("eligible pruning node has a static evaluation");
            value
                + self.correction.read(side, self.material_keys[ply as usize])
                + futility_margin(
                    self.nnue.pawn_value(),
                    depth,
                    self.improving(value, side, ply),
                )
        });
        let late_move_limit = pruning_node.then(|| late_move_limit(depth));
        // 損な先頭捕獲よりkiller手を先に返す条件はMovePicker::resetの契約に従う。
        self.move_pickers[ply as usize].reset(
            tt_move,
            self.killers[ply as usize],
            tt_move.is_none() && beta - alpha == 1 && depth <= 3,
        );
        let mut best_move = None;
        let mut best_score = -INFINITY;
        let mut best_capture = false;
        let mut beta_cutoff = false;
        let mut index = 0;
        let mut counted_quiets = 0;
        let mut had_forbidden_move = false;
        let mut searched_captures = Vec::new();
        let mut searched_quiets: Vec<Move> = Vec::new();
        // 枝刈りされた候補は探索順位に含めない。
        #[cfg(feature = "search-stats")]
        let mut searched_moves = 0;
        #[cfg(feature = "search-stats")]
        let mut best_move_rank = 0;
        while let Some((mv, capture)) = self.move_pickers[ply as usize].next(
            position,
            self.nnue,
            &self.generator,
            self.history,
            self.capture_history,
        ) {
            #[cfg(test)]
            if self
                .move_loop_probe
                .as_ref()
                .is_some_and(|probe| probe.ply == ply && probe.excluded.contains(&mv))
            {
                continue;
            }
            // 禁止手は探索順位、枝刈りの静かな手数、履歴の更新対象、ノード数に含めない。
            if self.move_is_forbidden(position, mv) {
                had_forbidden_move = true;
                continue;
            }
            #[cfg(test)]
            if let Some(probe) = &mut self.move_loop_probe
                && probe.ply == ply
            {
                if probe.visits.is_empty() {
                    probe.path_keys.clone_from(&self.path_keys);
                }
                probe.visits.push((mv, index, counted_quiets));
            }
            // late_move_limitが定める静かな手だけを数え、現在の手より前の数で判定する。
            // killer手はこの制限から除くが、futilityの対象にはなり得る。
            let counted_quiet = Some(mv) != tt_move
                && !capture
                && !mv.promote
                && !self.killers[ply as usize].contains(&Some(mv));
            let late_move_pruned =
                counted_quiet && late_move_limit.is_some_and(|limit| counted_quiets >= limit);
            counted_quiets += i32::from(counted_quiet);
            // 置換表の記録手、捕獲手、成る手はfutilityとlate move pruningで飛ばさない。
            // 成る手は、麒麟が獅子に成るように非捕獲でも評価が大きく動くため保護する。
            // 最善値が負の詰み帯と専用の帯を脱するまでは探索を続け、全手の枝刈りを合法手なしの詰みと誤認することと、
            // 安全な手を読まずに負けの値を上界として保存することを防ぐ。
            // 飛ばした手もindexを進めるがsearched_quietsには加えず、減点とkiller更新の対象にしない。
            if can_prune_moves(best_score)
                && (futility_bound.is_some_and(|bound| bound <= alpha) || late_move_pruned)
                && Some(mv) != tt_move
                && !mv.promote
                && !capture
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            {
                index += 1;
                continue;
            }
            // SEEの損が深さ別の余裕値を超える捕獲手を飛ばす。
            // 共通条件に加え、負の詰み帯と専用の帯を脱した後に限り、置換表の記録手と最後の王駒の捕獲を除く。
            // 規則依存で判定不能な交換は展開する。理由は
            // docs/adr/prune-losing-captures-by-see-at-shallow-nodes.md にある。
            if futility_bound.is_some()
                && can_prune_moves(best_score)
                && capture
                && Some(mv) != tt_move
                && !captures_last_royal(position, mv)
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                && see_prunes(
                    position,
                    self.rules,
                    self.nnue,
                    mv,
                    see_margin(self.nnue.pawn_value(), depth),
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
            #[cfg(feature = "search-stats")]
            {
                searched_moves += 1;
                if score > alpha {
                    best_move_rank = searched_moves;
                }
            }
            if score > best_score {
                best_score = score;
                best_move = Some(mv);
                best_capture = capture;
                self.update_pv(ply, mv);
            }
            alpha = alpha.max(score);
            if alpha >= beta {
                #[cfg(feature = "search-stats")]
                {
                    self.stats.beta_cutoffs += 1;
                    if searched_moves == 1 {
                        self.stats.first_move_beta_cutoffs += 1;
                    }
                }
                beta_cutoff = true;
                if capture {
                    self.capture_history.record_cutoff(
                        position,
                        self.nnue,
                        mv,
                        &searched_captures,
                        depth,
                    );
                } else {
                    self.capture_history.record_quiet_cutoff(
                        position,
                        self.nnue,
                        &searched_captures,
                        depth,
                    );
                    self.record_quiet_beta_cutoff(position, mv, &searched_quiets, depth, ply);
                }
                break;
            }
            if capture {
                searched_captures.push(mv);
            } else {
                searched_quiets.push(mv);
            }
            index += 1;
        }
        #[cfg(feature = "search-stats")]
        if !beta_cutoff {
            self.stats.record_best_move_rank(best_move_rank);
        }
        let Some(best_move) = best_move else {
            return Some(if had_forbidden_move {
                -(REPETITION_WIN - ply as i32)
            } else {
                -MATE + ply as i32
            });
        };
        let bound = if best_score <= original_alpha {
            Bound::Upper
        } else if beta_cutoff {
            Bound::Lower
        } else {
            Bound::Exact
        };
        self.update_correction(
            position,
            depth,
            ply,
            best_score,
            bound,
            best_capture,
            static_eval,
            royal_attacked,
        );
        self.tt
            .store(key, depth, best_score, bound, Some(best_move), ply);
        Some(best_score)
    }

    /// null moveの部分木を探索し、局面と探索状態を復元してから値を返す。
    pub(super) fn search_null_move(
        &mut self,
        position: &mut Position,
        depth: u32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        let lion_before = position
            .lion_taken_by_non_lion()
            .map(|trigger| trigger.square);
        let undo = position.make_null_move();
        self.previous_capture[(ply + 1) as usize] = None;
        let (parents, children) = self.accumulators.split_at_mut((ply + 1) as usize);
        self.nnue.update_accumulator_after_null(
            &parents[ply as usize],
            &mut children[0],
            lion_before,
        );
        self.material_keys[(ply + 1) as usize] = self.material_keys[ply as usize];
        let previous_null_move_ply = self.null_move_ply.replace(ply + 1);
        let previous_null_move_boundary = self.null_move_boundary.replace(self.path_keys.len());
        // このキー自体では反復を判定せず、部分木内の実着手からの参照点にする。
        self.path_keys
            .push(repetition_key(self.repetition.rule(), position));
        self.path_moves.push(PathMove {
            mv: None,
            attacking: None,
        });
        let score = self
            .negamax(position, depth, -beta, -beta + 1, ply + 1)
            .map(|value| -value);
        self.path_keys.pop();
        self.path_moves.pop();
        self.null_move_boundary = previous_null_move_boundary;
        self.null_move_ply = previous_null_move_ply;
        position.unmake_null_move(undo);
        score
    }

    /// 1手を適用して子局面を探索し、この局面から見た評価値を返す。
    ///
    /// 王駒をすべて取る手は子局面での終局値`MATE - (ply + 1)`を返す。
    /// [`Self::find_repetition`]で再現を認めた子局面は、経路へ積む前に[`Self::repetition_score`]を返す。
    /// 第31条R1では現在の着手を含む1周の攻撃側が負け、双方またはどちらも攻撃側でなければ引き分けとする。
    /// 勝敗の値は`REPETITION_WIN - (ply + 1)`の正負とし、R3の禁止されない再現は引き分けを返す。
    /// R2の禁止手は呼び出し前に除く。null moveの部分木では照合範囲より前の再現を検出しない。
    /// その子局面自体は置換表へ保存しないが、反復の値を含めて求めた親の値は保存する。
    /// null moveの部分木では、直近のnull move直後以降の経路だけを参照する。
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
        self.previous_capture[(ply + 1) as usize] = position
            .captured_squares(mv)
            .iter()
            .any(Option::is_some)
            .then_some(mv.to);
        if captures_last_royal(position, mv) {
            return Some(MATE - (ply + 1) as i32);
        }

        if !self.enter_node() {
            return None;
        }
        let undo = position.make_move_unchecked(mv, self.rules);
        let key = search_key(position);
        let repetition_key = match self.repetition {
            SearchRepetition::R1 { .. } => key,
            SearchRepetition::R2 | SearchRepetition::R3 => {
                repetition_key(self.repetition.rule(), position)
            }
        };
        if let Some(repeated) = self.find_repetition(repetition_key) {
            let score = self.repetition_score(repeated, position, mv, ply);
            position.unmake_move(undo);
            return Some(score);
        }

        let (parents, children) = self.accumulators.split_at_mut((ply + 1) as usize);
        self.nnue.update_accumulator_after_move(
            &parents[ply as usize],
            &mut children[0],
            position,
            &undo,
        );
        self.material_keys[(ply + 1) as usize] =
            key_after_move(self.material_keys[ply as usize], &undo);
        self.path_keys.push(repetition_key);
        self.path_moves.push(PathMove {
            mv: Some(mv),
            attacking: None,
        });
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
        self.path_moves.pop();
        position.unmake_move(undo);
        score
    }
}

/// 最善値が負の詰み帯と専用の帯を脱した後だけ、後続の静かな手と捕獲手の枝刈りを許す。
pub(super) fn can_prune_moves(best_score: i32) -> bool {
    best_score > -REPETITION_BAND_START
}

/// null moveによる打ち切りでは、詰み帯と専用の帯の見込みを確定値として返さず、βを返す。
pub(super) fn null_move_cutoff_score(score: i32, beta: i32) -> i32 {
    if score.abs() >= REPETITION_BAND_START {
        beta
    } else {
        score
    }
}

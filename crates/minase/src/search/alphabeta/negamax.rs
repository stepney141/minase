//! 内部ノードのαβ探索。

use std::cell::OnceCell;

use crate::search::snapshot::search_key;
use crate::search::{
    MATE, MATE_THRESHOLD, MAX_PLY, REPETITION_BAND_START, REPETITION_WIN, SearchRepetition,
    in_repetition_band,
};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::repetition_key;

use super::INFINITY;
use super::correction::{
    CorrectionPieceKeys, correction_from_weighted_sum, correction_piece_keys_after_move,
    key_after_move,
};
use super::history::pawn_key_after_move;
use super::ordering::{
    HistoryUpdateCoefficients, MovePickerContext, QuietOrderingCoefficients, move_order_key,
    piece_at_for_ordering,
};
use super::params;
use super::pruning::{
    CAPTURE_FUTILITY_LMR_DEPTH_LIMIT, LmrContext, NULL_MOVE_VERIFICATION_MIN_DEPTH,
    NULL_MOVE_VERIFICATION_PLY_DENOMINATOR, NULL_MOVE_VERIFICATION_PLY_NUMERATOR,
    NullMoveCoefficients, REVERSE_FUTILITY_MAX_DEPTH, SEEK_MATE_REVERSE_FUTILITY_MAX_DEPTH,
    applies_lmr, capture_futility_value, continuation_history_prunes, corrected_static_eval,
    futility_pruning_value, history_pruning_lmr_depth, is_improving, is_opponent_worsening,
    late_move_limit, lmr_depth, lmr_history_value, lmr_reduction, lmr_research_depth,
    non_lmr_depth, pruning_eval, pruning_lmr_depth, quiet_see_margin, razoring_margin,
    reverse_futility_margin, reverse_futility_value, see_margin, seek_mate_null_move_reduction,
    should_raise_futility_best_score, zero_window_tt_cutoff,
};
use super::royal::{captures_last_royal, royal_under_attack};
use super::searcher::{MoveRecord, PathMove, Searcher};
use super::see::see_prunes;
use super::tt::Bound;

/// 打ち切り前に置換表の手を指した局面を照合する深さの下限。
/// Stockfishの`depth >= 7`に対応する（src/search.cpp:917）。
const TT_CUTOFF_CHILD_MIN_DEPTH: u32 = 7;

/// 探索済みの手の列に保持するノードの手数の上限。
/// Stockfishの`SEARCHEDLIST_CAPACITY`と同じ32を使う（src/search.cpp:75, 1578）。
pub(super) const SEARCHED_LIST_CAPACITY: usize = 32;

/// LMRの減深探索がαを上回った手のcontinuation history加点。
/// Stockfishと同じ1334を使う（src/search.cpp:1423）。
const POST_LMR_CONTINUATION_BONUS: i32 = 1334;

/// 実際に読んだ手をhistoryの減点対象へ加えるかを返す。
pub(super) fn should_record_searched_move(move_count: usize, score: i32, alpha: i32) -> bool {
    move_count <= SEARCHED_LIST_CAPACITY && score <= alpha
}

impl Searcher<'_> {
    /// 保持した2手前の静的評価と比べる（Stockfish src/search.cpp:883）。
    pub(super) fn improving(&self, static_eval: i32, ply: u32) -> bool {
        is_improving(
            static_eval,
            ply.checked_sub(2)
                .map(|previous| self.static_evals[previous as usize]),
        )
    }

    /// ネガマックス形式のアルファベータ探索で局面を評価する。
    ///
    /// 深さ0または最大手数では[`Self::quiesce`]へ移り、合法手のない局面は第23条により`-MATE + ply`を返す。
    /// 静止探索へ移らない場合は、延長後の残り深さを`MAX_PLY - 1`以下に抑える（Stockfish src/search.cpp:752）。
    /// R2・R3で全合法手が禁止手なら`-(REPETITION_WIN - ply)`を返し、どちらも置換表へ保存しない。
    /// 禁止は履歴と経路に依存するため、別の経路で負けの値が再利用される詰みの帯を使わない。
    /// `cut_node`は、このノードがcutNode（親がβ打ち切りを見込んで零窓で読ませる子）かを表す。
    /// 中断された場合は`None`を返す。
    ///
    /// 置換表の専用の帯の値は経路に依存するため打ち切りに使わず、記録手だけを使う。
    /// 保存された詰みの距離と読み出し位置の手数の和が256を超える深い詰みの値も、
    /// 補正後に専用の帯へ入ると打ち切りに使われなくなる。保存形式に伴う既知の制約とする。
    /// 禁止手を除いて求めた通常の評価値は、履歴と経路に依存するまま置換表へ保存する近似を残す。
    ///
    /// 置換表で打ち切れず、要求深さ3以上で記録手がなければ、内部反復減深（IIR）として1だけ浅く読む。
    /// ただしallNodeと前の反復の主変化を辿るノードは除く（Stockfish src/search.cpp:1085）。
    /// stand-patだけの記録も「記録手なし」に含める。
    /// 判断の理由は`docs/adr/reduce-depth-by-one-without-tt-move-except-at-all-nodes-and-on-the-previous-pv.md`にある。
    ///
    /// IIRの後に補正後の静的評価を1回求め、手数ごとに保持する（Stockfish src/search.cpp:832, 846-884）。
    /// 置換表に補正前の静的評価があれば、評価関数の出力を求めずに使う（Stockfish src/search.cpp:854-860）。
    /// 照会に当たらなければ、求めた補正前の静的評価を探索値なしの記録として直ちに保存する
    /// （Stockfish src/search.cpp:870-874）。照合した記録に静的評価がない場合は保存しない。
    /// reverse futilityとrazoringには置換表の値を条件つきで採る判定用の評価値を使い、
    /// null moveの減深量、improving、およびfutilityには静的評価を使う。
    /// 補正表の更新にはノードの入口で求めた補正後の評価を渡す。
    ///
    /// 前向き枝刈りはreverse futility、razoring、null moveの順で判定する。
    /// 手のループではlate move pruningの後、捕獲手のfutility、捕獲手と王手になる手のSEE、静かな手のfutilityの順に判定し、残った手にLMRを適用する。
    /// 捕獲手のfutilityはlmrDepthが8未満で、成る手、相手の最後の王駒を取る手、および王手になる手を除く。
    /// 静かな手のfutilityはlmrDepthが12未満ならPVノードにも適用し、followPVのPVノード、成る手、および王手になる手を除く。
    /// αとβが詰み帯と専用の帯の外にあり、手番側の王駒に相手の疑似利きが届かないことも要求する。
    /// late move pruningはIIR後の残り深さに上限を設けず、零窓とPVノードの両方に適用する。
    /// αとβが詰み帯と専用の帯の外にあり、手番側の王駒に相手の疑似利きが届かないことは要求する。
    /// SEEは深さと零窓の条件を持たず、PVノードにも適用する。
    /// αとβが詰み帯と専用の帯の外にあり、手番側の王駒に相手の疑似利きが届かないことは要求する。
    /// razoringはallNodeで、判定用の評価値がIIR後の残り深さに比例する余裕値を引いたαより小さい場合に行う。
    /// 深さの上限は持たず、同じ帯と王駒への利きの条件を要求する。
    /// reverse futilityは残り深さ18以下でttPvでなく、合法な置換表の手が静かな手でない場合に限る。
    /// 詰みを探す反復では上限を5に下げ、razoringを行わない（Stockfish src/search.cpp:1017, 1022）。
    /// αとβの帯と王駒への利きの条件は、reverse futilityにも適用する。
    /// null moveとLMRの条件はそれぞれの判定箇所と[`super::pruning::lmr_reduction`]に従う。
    pub(super) fn negamax(
        &mut self,
        position: &mut Position,
        depth: u32,
        mut alpha: i32,
        beta: i32,
        ply: u32,
        cut_node: bool,
    ) -> Option<i32> {
        // PVノードはcutNodeにしない（Stockfish src/search.cpp:766）。
        #[cfg(feature = "invariants")]
        assert!(!(beta - alpha > 1 && cut_node));
        #[cfg(feature = "invariants")]
        assert_eq!(self.path_moves.len(), ply as usize);
        self.pv[ply as usize].clear();

        if depth == 0 || ply >= MAX_PLY {
            return self.quiesce(position, alpha, beta, ply);
        }
        let depth = depth.min(MAX_PLY - 1);
        #[cfg(feature = "search-stats")]
        {
            self.stats.normal_nodes += 1;
        }

        // 2手先だけを初期化し、子の手数の数は兄弟の探索と再探索の間で保持する。
        // Stockfish src/search.cpp:829。
        self.cutoff_counts[(ply + 2) as usize] = 0;
        // 手を読む前に書くstatScoreをノードの開始時に0へ戻す（Stockfish src/search.cpp:828）。
        self.stat_scores[ply as usize] = 0;
        // 子のnull move打ち切り回数は、兄弟とその再探索の間だけ保持する。
        // Stockfish src/search.cpp:830。
        self.prior_nmp_fail_high[(ply + 1) as usize] = 0;
        let key = search_key(position);
        let original_alpha = alpha;
        let pv_node = beta - alpha > 1;
        let all_node = !pv_node && !cut_node;
        let mut tt_move = None;
        // 合法性と捕獲の分類を遅延評価し、打ち切り、reverse futility、LMRで共有する。
        let tt_capture = OnceCell::new();
        // 深さが足りるヒットは即時カットオフだけに使い、探索窓は狭めない。
        // 窓を狭めると、格納時のバウンド分類が実際に探索した窓と食い違う。
        #[cfg(feature = "search-stats")]
        {
            self.stats.tt_probes += 1;
        }
        let tt_hit = self.tt.probe(key, ply);
        self.tt_hits[ply as usize] = tt_hit.is_some();
        // Stockfish src/search.cpp:842。PVの印をreverse futilityの適用条件に使う。
        self.tt_pv[ply as usize] = pv_node || tt_hit.is_some_and(|hit| hit.is_pv);
        if let Some(hit) = tt_hit {
            #[cfg(feature = "search-stats")]
            {
                self.stats.tt_hits += 1;
            }
            tt_move = hit.best_move;
            if let Some(value) = hit.value {
                let cutoff = if pv_node {
                    !in_repetition_band(value.score)
                        && u32::from(value.depth) >= depth
                        && match value.bound {
                            Bound::Exact => true,
                            Bound::Lower => value.score >= beta,
                            Bound::Upper => value.score <= alpha,
                        }
                } else {
                    zero_window_tt_cutoff(
                        value.score,
                        value.bound,
                        u32::from(value.depth),
                        depth,
                        beta,
                        cut_node,
                    )
                };
                if cutoff {
                    if value.score >= beta
                        && let Some(mv) = tt_move
                    {
                        let capture =
                            *tt_capture.get_or_init(|| self.legal_tt_capture(position, tt_move));
                        self.update_tt_cutoff_histories(position, ply, depth, mv, capture);
                    }
                    let mut consistent = true;
                    // Stockfish src/search.cpp:917-931。照合の着手は探索ノードに数えない。
                    if !pv_node
                        && depth >= TT_CUTOFF_CHILD_MIN_DEPTH
                        && value.score.abs() < MATE_THRESHOLD
                        && let Some(mv) = tt_move
                        && tt_capture
                            .get_or_init(|| self.legal_tt_capture(position, tt_move))
                            .is_some()
                    {
                        let undo = position.make_move_unchecked(mv, self.rules);
                        let next = self.tt.probe(search_key(position), ply + 1);
                        position.unmake_move(undo);
                        if let Some(next) = next
                            .and_then(|next| next.value)
                            .filter(|next| !in_repetition_band(next.score))
                        {
                            consistent = (value.score >= beta) == (-next.score >= beta);
                        }
                    }
                    if consistent {
                        #[cfg(feature = "search-stats")]
                        {
                            self.stats.tt_cutoffs += 1;
                        }
                        return Some(value.score);
                    }
                }
            }
        }

        // 置換表の打ち切りは要求深さで判定し、その後にIIRを適用する。
        let depth = if depth >= 3 && tt_move.is_none() && !all_node && !self.follow_pv[ply as usize]
        {
            depth - 1
        } else {
            depth
        };

        // 補正はノードの最初に1回読み、すべての静的評価の判定で共有する。
        // Stockfish src/search.cpp:832, 846-884。
        let side = position.side_to_move();
        #[cfg(feature = "invariants")]
        self.nnue
            .assert_accumulator(position, &self.accumulators[ply as usize], ply);
        let raw = if let Some(static_eval) = tt_hit.and_then(|hit| hit.static_eval) {
            #[cfg(feature = "search-stats")]
            {
                self.stats.tt_static_eval_uses += 1;
            }
            static_eval
        } else {
            let raw = self
                .nnue
                .evaluate_accumulator(&self.accumulators[ply as usize], side);
            if tt_hit.is_none()
                && self
                    .tt
                    .store_static_eval(key, raw, self.tt_pv[ply as usize])
            {
                #[cfg(feature = "search-stats")]
                {
                    self.stats.tt_static_eval_stores += 1;
                }
            }
            raw
        };
        let correction_weighted_sum = self.correction_weighted_sum_at(side, ply);
        let correction =
            correction_from_weighted_sum(correction_weighted_sum, self.nnue.pawn_value());
        let static_eval = corrected_static_eval(raw, correction);
        self.static_evals[ply as usize] = static_eval;
        self.update_eval_difference_histories(ply, static_eval, self.tt_hits[ply as usize]);
        let mut improving = self.improving(static_eval, ply);
        let eval = pruning_eval(
            static_eval,
            tt_hit
                .and_then(|hit| hit.value)
                .map(|value| (value.score, value.bound)),
        );

        // 王駒への利きは個々の枝刈りが発動し得る場合に調べる。
        let window_outside_bands =
            alpha.abs() < REPETITION_BAND_START && beta.abs() < REPETITION_BAND_START;
        let razoring_node = all_node
            && !self.seek_mate
            && window_outside_bands
            && eval < alpha - razoring_margin(self.nnue.pawn_value(), depth);
        let late_move_node = window_outside_bands;
        let mut royal_attacked = None;
        // Stockfish src/search.cpp:1022-1033。余裕値が負でもβ未満の値では打ち切らない。
        // 返すβと判定用の評価値の加重平均は置換表へ保存しない。
        let reverse_futility_max_depth = if self.seek_mate {
            SEEK_MATE_REVERSE_FUTILITY_MAX_DEPTH
        } else {
            REVERSE_FUTILITY_MAX_DEPTH
        };
        if depth <= reverse_futility_max_depth
            && !self.tt_pv[ply as usize]
            && window_outside_bands
            && eval >= beta
            && eval
                - reverse_futility_margin(
                    self.nnue.pawn_value(),
                    depth,
                    tt_hit.is_some(),
                    improving,
                    is_opponent_worsening(
                        static_eval,
                        ply.checked_sub(1)
                            .map(|previous| self.static_evals[previous as usize]),
                    ),
                    correction,
                )
                >= beta
            && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            && tt_capture
                .get_or_init(|| self.legal_tt_capture(position, tt_move))
                .is_none_or(|capture| capture)
        {
            return Some(reverse_futility_value(beta, eval));
        }
        // allNodeで判定用の評価値が深さ比例の余裕値を引いたαより小さければ、静止探索の値をそのまま返す。
        // Stockfish src/search.cpp:1015-1018。
        // 王駒に疑似利きが届くノードを除く理由は
        // docs/adr/extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md にある。
        if razoring_node && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position)) {
            let value = self.quiesce(position, alpha, beta, ply)?;
            return Some(value);
        }

        let has_non_royal_piece =
            !(position.pieces_of(side) & !position.royal_pieces(side)).is_empty();
        // Stockfish src/search.cpp:1037-1076。王駒への利きは、ほかの条件がすべて成立した後に調べる。
        let null_move_coefficients = NullMoveCoefficients::current();
        if cut_node
            && self.null_move_ply != Some(ply)
            && window_outside_bands
            && has_non_royal_piece
            && self.nmp_min_ply.is_none_or(|minimum| ply >= minimum)
            && null_move_coefficients.allows_null_move(
                depth,
                static_eval,
                beta,
                self.nnue.pawn_value(),
                improving,
                self.prior_nmp_fail_high[ply as usize],
            )
            && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
        {
            let reduction = seek_mate_null_move_reduction(
                1 + null_move_coefficients.null_move_reduction(
                    depth,
                    static_eval,
                    beta,
                    self.nnue.pawn_value(),
                ),
                self.seek_mate,
            );
            let child_depth = depth.saturating_sub(reduction);
            let score = self.search_null_move(position, child_depth, beta, ply)?;
            if score >= beta {
                if self.nmp_min_ply.is_some() || depth < NULL_MOVE_VERIFICATION_MIN_DEPTH {
                    self.prior_nmp_fail_high[ply as usize] += 1;
                    return Some(null_move_cutoff_score(score, beta));
                }
                let verification =
                    self.search_null_move_verification(position, child_depth, beta, ply)?;
                if verification >= beta {
                    self.prior_nmp_fail_high[ply as usize] += 1;
                    return Some(null_move_cutoff_score(score, beta));
                }
            }
        }

        // null moveを試さなかった場合も更新する（Stockfish src/search.cpp:1080）。
        improving |= static_eval >= beta;
        let late_move_limit = late_move_node.then(|| late_move_limit(depth, improving));
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
        let mut searched_captures = Vec::with_capacity(SEARCHED_LIST_CAPACITY);
        let mut searched_quiets = Vec::with_capacity(SEARCHED_LIST_CAPACITY);
        let quiet_ordering_coefficients = QuietOrderingCoefficients::current();
        // 枝刈りされた候補は探索順位に含めない。
        #[cfg(feature = "search-stats")]
        let mut searched_moves = 0;
        #[cfg(feature = "search-stats")]
        let mut best_move_rank = 0;
        while let Some((mv, capture)) = self.move_pickers[ply as usize].next(
            position,
            self.nnue,
            &self.generator,
            MovePickerContext {
                history: &*self.history,
                capture_history: &*self.capture_history,
                continuation_history: self.continuation_history,
                pawn_history: self.pawn_history,
                low_ply_history: self.low_ply_history.as_ref(),
                path_moves: &self.path_moves,
                pawn_key: self.pawn_keys[ply as usize],
                ply,
                coefficients: quiet_ordering_coefficients,
            },
        ) {
            // 禁止手は探索順位、枝刈りの静かな手数、履歴の更新対象、ノード数に含めない。
            if self.move_is_forbidden(position, mv) {
                had_forbidden_move = true;
                continue;
            }
            self.move_counts[ply as usize] = index + 1;
            // late_move_limitが定める静かな手だけを数え、現在の手より前の数で判定する。
            // killer手はこの制限から除くが、futilityの対象にはなり得る。
            let counted_quiet = Some(mv) != tt_move
                && !capture
                && !mv.promote
                && !self.killers[ply as usize].contains(&Some(mv));
            let late_move_pruned =
                counted_quiet && late_move_limit.is_some_and(|limit| counted_quiets >= limit);
            counted_quiets += i32::from(counted_quiet);
            // 置換表の記録手は、手のループ内の前向き枝刈りで飛ばさない。
            // 最善値が負の詰み帯と専用の帯を脱するまでは探索を続け、全手の枝刈りを合法手なしの詰みと誤認することと、
            // 安全な手を読まずに負けの値を上界として保存することを防ぐ。初期値ではcan_prune_movesが偽なので、
            // 真になる前に少なくとも1手を読み、best_moveを設定する。futilityの見積りでbest_scoreだけを上げても、
            // best_moveがNoneのままになることはない。
            // 飛ばした手もindexを進めるが、探索済みの手には加えず、減点とkiller更新の対象にしない。
            if can_prune_moves(best_score) && Some(mv) != tt_move {
                // late move pruningは、捕獲手と成る手を対象にしない。
                if late_move_node
                    && late_move_pruned
                    && !mv.promote
                    && !capture
                    && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                {
                    index += 1;
                    continue;
                }

                if capture {
                    // Stockfish src/search.cpp:1208-1217。捕獲価値は整列値を流用せず、
                    // 2枚取りを含む実際の捕獲升から求める。
                    if window_outside_bands && !mv.promote && !captures_last_royal(position, mv) {
                        let lmr_depth = pruning_lmr_depth(
                            depth,
                            index,
                            improving,
                            pv_node,
                            self.tt_pv[ply as usize],
                        );
                        if lmr_depth < CAPTURE_FUTILITY_LMR_DEPTH_LIMIT {
                            let captured_value = position
                                .captured_squares(mv)
                                .into_iter()
                                .flatten()
                                .map(|square| {
                                    self.nnue.piece_value(
                                        position
                                            .piece_at(square)
                                            .expect("capture square must contain a piece"),
                                    )
                                })
                                .sum();
                            let history = self.capture_history.read(position, self.nnue, mv);
                            if capture_futility_value(
                                self.nnue.pawn_value(),
                                static_eval,
                                lmr_depth,
                                captured_value,
                                history,
                            ) <= alpha
                                // 王手と王駒への利きは、安い条件をすべて満たした後に調べる。
                                && !position.gives_check(&self.generator, mv)
                                && !*royal_attacked
                                    .get_or_insert_with(|| royal_under_attack(position))
                            {
                                index += 1;
                                continue;
                            }
                        }
                    }
                    // SEEの損が深さとcapture historyの余裕値を超える捕獲手を飛ばす。
                    // 相手の最後の王駒を取る手は除き、規則依存で判定不能な交換は展開する。理由は
                    // docs/adr/prune-captures-and-quiet-moves-by-see-and-capture-futility-with-depth-scaled-margins.md にある。
                    if window_outside_bands
                        && !captures_last_royal(position, mv)
                        && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                        && see_prunes(
                            position,
                            self.rules,
                            self.nnue,
                            mv,
                            -see_margin(
                                self.nnue.pawn_value(),
                                depth,
                                self.capture_history.read(position, self.nnue, mv),
                            ),
                        )
                    {
                        index += 1;
                        continue;
                    }
                } else if !mv.promote && window_outside_bands {
                    // Stockfishの「捕獲手か王手ならS3、それ以外ならS1、S5」と同じ結果になるよう、
                    // S1で王手と判明した手はS5を通さずS3へ進める（src/search.cpp:1205, 1227-1259）。
                    let mut gives_check = None;
                    if !(self.follow_pv[ply as usize] && pv_node) {
                        let piece = piece_at_for_ordering(position, mv.from);
                        let history = self.continuation_value(ply, piece, mv, 1).unwrap_or(0)
                            + self.continuation_value(ply, piece, mv, 2).unwrap_or(0)
                            + self
                                .pawn_history
                                .read(self.pawn_keys[ply as usize], piece, mv.to);
                        // 王手と王駒への利きは、安いhistoryの条件を満たした後に調べる。
                        if continuation_history_prunes(
                            history,
                            depth,
                            params::continuation_pruning(),
                        ) && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                            && !*gives_check
                                .get_or_insert_with(|| position.gives_check(&self.generator, mv))
                        {
                            index += 1;
                            continue;
                        }
                        // 枝刈り用のlmrDepthは、この判定に到達した手についてだけ求める。
                        let lmr_depth = history_pruning_lmr_depth(
                            pruning_lmr_depth(
                                depth,
                                index,
                                improving,
                                pv_node,
                                self.tt_pv[ply as usize],
                            ),
                            depth,
                            history,
                            self.history[side.index()][mv.from.dense_index()][mv.to.dense_index()],
                        );
                        if let Some(futility_value) = futility_pruning_value(
                            self.nnue.pawn_value(),
                            static_eval,
                            alpha,
                            lmr_depth,
                        ) {
                            // 王手と王駒への利きは、安い条件をすべて満たした後に調べる。
                            if !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                                && !*gives_check.get_or_insert_with(|| {
                                    position.gives_check(&self.generator, mv)
                                })
                            {
                                if should_raise_futility_best_score(best_score, futility_value) {
                                    best_score = futility_value;
                                }
                                index += 1;
                                continue;
                            }
                        }
                        // S1で飛ばさなかった王手でない静かな手は、lmrDepthの2乗に応じた
                        // SEEの損がある場合に飛ばす。成る手と2段階移動は対象外である。
                        if gives_check != Some(true)
                            && mv.mid.is_none()
                            && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                            && see_prunes(
                                position,
                                self.rules,
                                self.nnue,
                                mv,
                                -quiet_see_margin(self.nnue.pawn_value(), lmr_depth),
                            )
                            && !*gives_check
                                .get_or_insert_with(|| position.gives_check(&self.generator, mv))
                        {
                            index += 1;
                            continue;
                        }
                    }
                    // 王手になる静かな手にはS3の余裕値を使う。王手か未判定なら、
                    // S3のSEEで枝刈りできる場合にだけ判定する（Stockfish src/search.cpp:1205, 1220-1225）。
                    if !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
                        && see_prunes(
                            position,
                            self.rules,
                            self.nnue,
                            mv,
                            -see_margin(self.nnue.pawn_value(), depth, 0),
                        )
                        && *gives_check
                            .get_or_insert_with(|| position.gives_check(&self.generator, mv))
                    {
                        index += 1;
                        continue;
                    }
                }
            }
            // 子がこのノードへ至った手の値として読み、LMRの連続量にも使う。
            // 枝刈りを通った後、減深量を求める前に1回だけ計算する（src/search.cpp:1371-1381）。
            let stat_score = self.move_stat_score(position, mv, capture, ply);
            self.stat_scores[ply as usize] = stat_score;
            // 残り深さ2以上、2手目以降に適用する（Stockfish src/search.cpp:1395）。
            let lmr = applies_lmr(depth, index);
            let first_depth = if lmr || (index == 0 && !pv_node) {
                let tt_capture =
                    *tt_capture.get_or_init(|| self.legal_tt_capture(position, tt_move));
                let reduction = lmr_reduction(LmrContext {
                    depth,
                    index,
                    improving,
                    pv_node,
                    tt_pv: self.tt_pv[ply as usize],
                    cut_node,
                    tt_hit: tt_hit
                        .and_then(|hit| hit.value)
                        .map(|value| (value.score, u32::from(value.depth))),
                    tt_capture,
                    is_tt_move: tt_capture.is_some() && Some(mv) == tt_move,
                    capture,
                    stat_score,
                    history: lmr_history_value(
                        capture,
                        self.history[side.index()][mv.from.dense_index()][mv.to.dense_index()],
                    ),
                    cutoff_count: self.cutoff_counts[(ply + 1) as usize],
                    alpha,
                    eval,
                    pawn: self.nnue.pawn_value(),
                    correction_weighted_sum,
                    seek_mate_gives_check: self.lmr_seek_mate_gives_check(position, mv),
                });
                if lmr {
                    lmr_depth(depth, reduction, pv_node, ply, self.root_depth)
                } else {
                    non_lmr_depth(depth - 1, reduction, tt_capture.is_some())
                }
            } else {
                depth - 1
            };
            let alpha_before = alpha;
            let score = self.search_move(
                position,
                mv,
                depth,
                alpha,
                beta,
                ply,
                index == 0,
                first_depth,
                lmr.then_some(best_score),
                cut_node,
            )?;
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
                // 延長の有無による重み付けは行わない（Stockfish src/search.cpp:1562）。
                self.cutoff_counts[ply as usize] += 1;
                beta_cutoff = true;
                if !capture {
                    self.record_killer(mv, ply);
                }
                break;
            }
            // αを上回った手は、後で最善手でなくなっても減点対象へ加えない。
            if should_record_searched_move(index + 1, score, alpha_before) {
                if capture {
                    searched_captures.push(mv);
                } else {
                    searched_quiets.push(mv);
                }
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
        if best_score > original_alpha {
            let parent_stat_score = ply
                .checked_sub(1)
                .map_or(0, |parent| self.stat_scores[parent as usize]);
            let updates = HistoryUpdateCoefficients::current().updates(
                depth,
                Some(best_move) == tt_move,
                parent_stat_score,
                !pv_node,
                searched_quiets.len() + searched_captures.len(),
            );
            self.update_best_move_histories(
                position,
                best_move,
                best_capture,
                &searched_quiets,
                &searched_captures,
                updates,
            );
            self.update_refutation_history(ply, updates.malus);
        } else {
            self.update_fail_low_histories(ply, depth, best_score, static_eval);
        }
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
        // Stockfish src/search.cpp:1651-1657。α以下なら、直前のノードの印も継ぐ。
        if best_score <= original_alpha {
            self.tt_pv[ply as usize] |= ply > 0 && self.tt_pv[(ply - 1) as usize];
        }
        self.tt.store(
            key,
            depth,
            best_score,
            Some(raw),
            bound,
            Some(best_move),
            ply,
            self.tt_pv[ply as usize],
        );
        Some(best_score)
    }

    /// 置換表の手が合法な場合だけ捕獲かを返す。ノード内の遅延評価で1回だけ呼ぶ。
    fn legal_tt_capture(&self, position: &Position, tt_move: Option<Move>) -> Option<bool> {
        tt_move
            .filter(|&mv| {
                self.generator
                    .is_legal_move(position, mv, &mut Vec::new(), &mut Vec::new())
            })
            .map(|mv| move_order_key(position, self.nnue, mv).is_some())
    }

    /// LMRの式を使う手について、詰みを探す局面でだけ王手になるかを返す。
    pub(super) fn lmr_seek_mate_gives_check(&self, position: &Position, mv: Move) -> Option<bool> {
        self.seek_mate
            .then(|| position.gives_check(&self.generator, mv))
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
        self.follow_pv[(ply + 1) as usize] = false;
        self.previous_move_to[(ply + 1) as usize] = None;
        let (parents, children) = self.accumulators.split_at_mut((ply + 1) as usize);
        self.nnue.update_accumulator_after_null(
            &parents[ply as usize],
            &mut children[0],
            lion_before,
        );
        self.material_keys[(ply + 1) as usize] = self.material_keys[ply as usize];
        self.pawn_keys[(ply + 1) as usize] = self.pawn_keys[ply as usize];
        self.minor_keys[(ply + 1) as usize] = self.minor_keys[ply as usize];
        self.non_pawn_keys[(ply + 1) as usize] = self.non_pawn_keys[ply as usize];
        let previous_null_move_ply = self.null_move_ply.replace(ply + 1);
        let previous_null_move_boundary = self.null_move_boundary.replace(self.path_keys.len());
        // このキー自体では反復を判定せず、部分木内の実着手からの参照点にする。
        self.path_keys
            .push(repetition_key(self.repetition.rule(), position));
        self.path_moves.push(PathMove {
            record: None,
            attacking: None,
        });
        let score = self
            .negamax(
                position,
                depth,
                -beta,
                -beta + 1,
                ply + 1,
                child_cut_node(ChildSearch::NullMove, false),
            )
            .map(|value| -value);
        self.path_keys.pop();
        self.path_moves.pop();
        self.null_move_boundary = previous_null_move_boundary;
        self.null_move_ply = previous_null_move_ply;
        position.unmake_null_move(undo);
        score
    }

    /// 同じ局面を零窓で検証し、同一plyの外側の状態を復元してから値を返す。
    ///
    /// 検証中は`ply + 3 * depth / 4`へ達するまでnull moveを禁じる
    /// （Stockfish src/search.cpp:1061-1070）。中断時も状態を復元し、完了した探索だけを統計へ数える。
    pub(super) fn search_null_move_verification(
        &mut self,
        position: &mut Position,
        depth: u32,
        beta: i32,
        ply: u32,
    ) -> Option<i32> {
        let previous_static_eval = self.static_evals[ply as usize];
        let previous_tt_pv = self.tt_pv[ply as usize];
        let previous_stat_score = self.stat_scores[ply as usize];
        let previous_move_count = self.move_counts[ply as usize];
        let previous_tt_hit = self.tt_hits[ply as usize];
        let previous_nmp_min_ply = self.nmp_min_ply.replace(
            ply + NULL_MOVE_VERIFICATION_PLY_NUMERATOR * depth
                / NULL_MOVE_VERIFICATION_PLY_DENOMINATOR,
        );
        let score = self.negamax(position, depth, beta - 1, beta, ply, false);
        self.nmp_min_ply = previous_nmp_min_ply;
        self.static_evals[ply as usize] = previous_static_eval;
        self.tt_pv[ply as usize] = previous_tt_pv;
        self.stat_scores[ply as usize] = previous_stat_score;
        self.move_counts[ply as usize] = previous_move_count;
        self.tt_hits[ply as usize] = previous_tt_hit;
        self.pv[ply as usize].clear();
        let score = score?;
        #[cfg(feature = "search-stats")]
        {
            self.stats.null_move_verifications += 1;
            self.stats.null_move_verification_failures += u64::from(score < beta);
        }
        Some(score)
    }

    /// 1手を適用して子局面を探索し、この局面から見た評価値を返す。
    ///
    /// 王駒をすべて取る手は子局面での終局値`MATE - (ply + 1)`を返す。
    /// [`Self::find_repetition`]で再現を認めた子局面は、経路へ積む前に[`Self::repetition_score`]を返す。
    /// 第31条R1では現在の着手を含む1周の攻撃側が負け、双方またはどちらも攻撃側でなければ引き分けとする。
    /// 引き分けは現在の着手側が根の手番と同じなら`draw_value`、異なれば`-draw_value`を返す。
    /// 勝敗の値は`REPETITION_WIN - (ply + 1)`の正負とする。
    /// R3の禁止されない再現は、周回の損得を表す近似値として設定にかかわらず`DRAW_SCORE`を返す。
    /// R2の禁止手は呼び出し前に除く。null moveの部分木では照合範囲より前の再現を検出しない。
    /// その子局面自体は置換表へ保存しないが、反復の値を含めて求めた親の値は保存する。
    /// null moveの部分木では、直近のnull move直後以降の経路だけを参照する。
    /// 初回の子の深さは`first_depth`とし、2手目以降は零窓で探索する。
    /// `lmr_best`はLMR対象の手を読む直前の最善値であり、対象外ではNoneとする。
    /// LMRの値がαを超えたら通常深さを調整し、初回より深い場合だけ零窓で再探索する。
    /// さらに窓内なら、調整した深さで全窓の再探索を行う（Stockfish src/search.cpp:1410-1455）。
    /// LMR対象外の全窓の再探索は通常深さのままとする。
    /// `cut_node`は親ノードの値であり、子へは[`child_cut_node`]の規則で渡す。
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
        first_depth: u32,
        lmr_best: Option<i32>,
        cut_node: bool,
    ) -> Option<i32> {
        self.pv[(ply + 1) as usize].clear();
        // 親から一致し続ける手順だけを辿る（Stockfish src/search.cpp:791-794）。
        self.follow_pv[(ply + 1) as usize] =
            self.follow_pv[ply as usize] && self.previous_pv.get(ply as usize) == Some(&mv);
        self.previous_move_to[(ply + 1) as usize] = Some(mv.to);
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
        self.pawn_keys[(ply + 1) as usize] =
            pawn_key_after_move(self.pawn_keys[ply as usize], &undo);
        let correction_keys = correction_piece_keys_after_move(
            CorrectionPieceKeys {
                minor: self.minor_keys[ply as usize],
                non_pawn: self.non_pawn_keys[ply as usize],
            },
            &undo,
        );
        self.minor_keys[(ply + 1) as usize] = correction_keys.minor;
        self.non_pawn_keys[(ply + 1) as usize] = correction_keys.non_pawn;
        self.path_keys.push(repetition_key);
        self.path_moves.push(PathMove {
            record: Some(MoveRecord::from_undo(self.nnue, &undo)),
            attacking: None,
        });
        let mut score = if first {
            self.negamax(
                position,
                first_depth,
                -beta,
                -alpha,
                ply + 1,
                child_cut_node(
                    ChildSearch::First {
                        pv_node: beta - alpha > 1,
                    },
                    cut_node,
                ),
            )
            .map(|value| -value)
        } else {
            self.negamax(
                position,
                first_depth,
                -alpha - 1,
                -alpha,
                ply + 1,
                child_cut_node(ChildSearch::Later { depth }, cut_node),
            )
            .map(|value| -value)
        };
        let mut research_depth = depth - 1;
        let mut update_post_lmr = false;
        if let Some(best) = lmr_best
            && let Some(value) = score
            && value > alpha
        {
            research_depth = lmr_research_depth(
                research_depth,
                first_depth,
                value,
                best,
                self.nnue.pawn_value(),
            );
            if research_depth > first_depth {
                score = self
                    .negamax(
                        position,
                        research_depth,
                        -alpha - 1,
                        -alpha,
                        ply + 1,
                        child_cut_node(ChildSearch::ZeroWindowResearch, cut_node),
                    )
                    .map(|value| -value);
            }
            update_post_lmr = score.is_some();
        }
        if update_post_lmr {
            let record = self.path_moves[ply as usize]
                .record
                .expect("LMR move must have a path record");
            self.update_continuation(ply, record.piece, record.mv, POST_LMR_CONTINUATION_BONUS);
        }
        if !first && score.is_some_and(|value| value > alpha && value < beta) {
            score = self
                .negamax(
                    position,
                    research_depth,
                    -beta,
                    -alpha,
                    ply + 1,
                    child_cut_node(ChildSearch::FullWindowResearch, cut_node),
                )
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

/// 子のcutNodeを決める探索の呼び出し箇所。
///
/// 行番号はStockfishのsrc/search.cppに対応する。
pub(super) enum ChildSearch {
    /// null moveの子は偽にする（1048行）。
    NullMove,
    /// 先頭手の子は親がPVなら偽、それ以外なら親の値の否定にする（1455行、1428〜1436行）。
    First { pv_node: bool },
    /// 後続手の最初の零窓は親の残り深さ2以上で真、1で親の値の否定にする（1395行、1405行、1428〜1436行）。
    /// `depth`はIIR適用後の親の残り深さであり、1以上とする。
    Later { depth: u32 },
    /// 減深後の通常深さでの零窓の再探索は親の値の否定にする（1410〜1420行）。
    ZeroWindowResearch,
    /// 全窓の再探索は偽にする（1442〜1455行）。
    FullWindowResearch,
}

/// 子に渡すcutNodeを、親の値と呼び出し箇所から決める。
///
/// Stockfishのsrc/search.cppの受け渡しに従う。
/// null moveは偽（1048行）、先頭手は親がPVなら偽（1455行）、零窓なら親の値の否定（1428〜1436行）とする。
/// 後続手の最初の零窓は親の残り深さ2以上なら真（1395行、1405行）、1なら親の値の否定（1428〜1436行）とする。
/// 通常深さでの零窓の再探索は親の値の否定（1410〜1420行）、全窓の再探索は偽（1442〜1455行）とする。
pub(super) fn child_cut_node(search: ChildSearch, parent_cut_node: bool) -> bool {
    match search {
        ChildSearch::NullMove => false,
        ChildSearch::First { pv_node: true } => false,
        ChildSearch::First { pv_node: false } => !parent_cut_node,
        ChildSearch::Later { depth } if depth >= 2 => true,
        ChildSearch::Later { .. } => !parent_cut_node,
        ChildSearch::ZeroWindowResearch => !parent_cut_node,
        ChildSearch::FullWindowResearch => false,
    }
}

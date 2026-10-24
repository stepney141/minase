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

/// 根の手の平均を更新する尺度（Stockfish `src/search.cpp:1480`）。
const ROOT_MOVE_AVERAGE_SCALE: u32 = 32;
/// 根の手の重みで、更新前の累計ノード数に掛ける比3/2の分子
/// （Stockfish `src/search.cpp:1481`）。
const ROOT_MOVE_EFFORT_RATIO_NUMERATOR: u32 = 3;
/// 根の手の重みで、今回のノード数に掛ける比3/2の分母
/// （Stockfish `src/search.cpp:1482`）。
const ROOT_MOVE_EFFORT_RATIO_DENOMINATOR: u32 = 2;
/// 根の手の平均に使う重みの下限（Stockfish `src/search.cpp:1483`）。
const ROOT_MOVE_WEIGHT_MIN: u32 = 12;
/// 根の手の平均に使う重みの上限（Stockfish `src/search.cpp:1484`）。
const ROOT_MOVE_WEIGHT_MAX: u32 = 24;
/// 根の手の符号つき2乗の平均に使う重みの上限（Stockfish `src/search.cpp:1489`）。
const ROOT_MOVE_SIGNED_SQUARE_WEIGHT_MAX: u32 = 16;
/// 反復を始めるたびにfail highの持ち越しから引く量
/// （Stockfish `src/search.cpp:405`）。
const FAIL_HIGH_RECOVERY_DECAY: u32 = 2;
/// fail highによって減らした窓の深さの下限
/// （Stockfish `src/search.cpp:410`）。
const MIN_ASPIRATION_WINDOW_DEPTH: u32 = 1;
/// 深さを増やさない反復の回数から減深を求める分子と分母
/// （Stockfish `src/search.cpp:410-411`）。
const SEARCH_AGAIN_REDUCTION_NUMERATOR: u32 = 3;
const SEARCH_AGAIN_REDUCTION_DENOMINATOR: u32 = 4;

/// 1回の反復で数えるfail highと、反復をまたいで持ち越す減深。
/// 窓の深さでは、これらとincreaseDepthのワーカー固有の回数を合成する。
#[derive(Clone, Copy)]
pub(super) struct FailHighReduction {
    failed_high_count: u32,
    recovery: u32,
}

impl FailHighReduction {
    /// 持ち越しを2減らして0で止め、fail highの回数を0にして反復を始める。
    pub(super) fn begin(recovery: u32) -> Self {
        Self {
            failed_high_count: 0,
            recovery: recovery.saturating_sub(FAIL_HIGH_RECOVERY_DECAY),
        }
    }

    /// 反復の深さからfail high 1回につき1手、持ち越し、およびincreaseDepthの
    /// 減深を引き、1手を下限とする（Stockfish `src/search.cpp:410-411, 450`）。
    pub(super) fn window_depth(self, iteration_depth: u32, search_again_counter: u32) -> u32 {
        iteration_depth
            .saturating_sub(self.failed_high_count)
            .saturating_sub(self.recovery)
            .saturating_sub(increase_depth_reduction(search_again_counter))
            .max(MIN_ASPIRATION_WINDOW_DEPTH)
    }

    /// fail highの回数を1増やす。
    pub(super) fn after_fail_high(mut self) -> Self {
        self.failed_high_count += 1;
        self
    }

    /// fail lowの後はfail highの回数を0へ戻す。
    pub(super) fn after_fail_low(mut self) -> Self {
        self.failed_high_count = 0;
        self
    }

    /// 窓内で完了した反復の後に持ち越す減深を返す。
    ///
    /// fail highがあれば`(h + 1) / 2 + 2`、なければ開始時に減らした値を保つ
    /// （Stockfish `src/search.cpp:461-462`）。
    pub(super) fn recovery_after_completion(self) -> u32 {
        if self.failed_high_count == 0 {
            self.recovery
        } else {
            self.failed_high_count.div_ceil(2) + 2
        }
    }
}

/// 深さを増やさずに始めた反復の回数から、窓の減深を返す。
pub(super) fn increase_depth_reduction(search_again_counter: u32) -> u32 {
    SEARCH_AGAIN_REDUCTION_NUMERATOR * (search_again_counter + 1)
        / SEARCH_AGAIN_REDUCTION_DENOMINATOR
}

/// 根の手について、`go`の間に累積する探索量と評価値の平均。
#[derive(Default)]
pub(super) struct RootMoveStats {
    /// 累計の探索ノード数。
    pub(super) effort: u64,
    /// 評価値の平均と、符号つき2乗`v × |v|`の平均。
    pub(super) averages: Option<(i32, i64)>,
}

impl RootMoveStats {
    /// 完了した1回の根の手の探索を、今回のノード数と最終値で反映する
    /// （Stockfish `src/search.cpp:1474-1501`）。
    pub(super) fn update(&mut self, nodes: u64, value: i32) {
        let weight = i64::from(root_move_weight(nodes, self.effort));
        self.effort += nodes;

        let signed_value = i64::from(value);
        let signed_square = signed_value * signed_value.abs();
        if let Some((average, signed_square_average)) = &mut self.averages {
            let scale = i64::from(ROOT_MOVE_AVERAGE_SCALE);
            *average = i32::try_from(
                (weight * signed_value + (scale - weight) * i64::from(*average)).div_euclid(scale),
            )
            .expect("root move average remains an i32 score");
            let signed_square_weight = weight.min(i64::from(ROOT_MOVE_SIGNED_SQUARE_WEIGHT_MAX));
            *signed_square_average = (signed_square_weight * signed_square
                + (scale - signed_square_weight) * *signed_square_average)
                .div_euclid(scale);
        } else {
            self.averages = Some((value, signed_square));
        }
    }
}

/// 今回と更新前の累計ノード数から、根の手の平均に使う重みを求める
/// （Stockfish `src/search.cpp:1486-1488`）。
pub(super) fn root_move_weight(nodes: u64, previous_effort: u64) -> u32 {
    let previous_effort = previous_effort.max(1);
    let numerator = u128::from(ROOT_MOVE_AVERAGE_SCALE)
        * u128::from(ROOT_MOVE_EFFORT_RATIO_DENOMINATOR)
        * u128::from(nodes);
    let denominator = u128::from(ROOT_MOVE_EFFORT_RATIO_DENOMINATOR) * u128::from(nodes)
        + u128::from(ROOT_MOVE_EFFORT_RATIO_NUMERATOR) * u128::from(previous_effort);
    let weight = (numerator / denominator).clamp(
        u128::from(ROOT_MOVE_WEIGHT_MIN),
        u128::from(ROOT_MOVE_WEIGHT_MAX),
    );
    u32::try_from(weight).expect("root move weight is bounded by its u32 constants")
}

/// 反復探索の窓と、窓外れ後の移動に使う半幅。
///
/// 直前の完了反復の最善手について、評価値の平均の周囲から探索を始めるaspiration windowである。
/// `delta`は静的評価と同じ単位で、初期値は定数項、符号つき2乗の平均の項、およびワーカー番号の項から求める。
/// 窓外れ時は返った値から外れた側の端を移し、反対側の端も元の窓の端を基準に寄せる。
/// 判断の理由は`docs/adr/center-aspiration-windows-on-the-best-move-average-and-move-both-edges-on-failure.md`にある。
pub(super) struct AspirationWindow {
    pub(super) alpha: i32,
    pub(super) beta: i32,
    pub(super) delta: Option<i32>,
}

impl AspirationWindow {
    /// 最善手の平均が詰み帯と専用の帯の外なら、平均±半幅を初期窓にする。
    /// 平均と半幅の組がない場合は全窓とし、半幅は保持せず読まない。
    /// 詰み帯または専用の帯に達した端は無限へ置き換える。
    pub(super) fn initial(average_and_delta: Option<(i32, i32)>) -> Self {
        let Some((average, delta)) = average_and_delta else {
            return Self {
                alpha: -INFINITY,
                beta: INFINITY,
                delta: None,
            };
        };
        if i64::from(average).abs() >= i64::from(REPETITION_BAND_START) {
            return Self {
                alpha: -INFINITY,
                beta: INFINITY,
                delta: None,
            };
        }
        debug_assert!(delta >= 1);
        let mut window = Self {
            alpha: -INFINITY,
            beta: INFINITY,
            delta: Some(delta),
        };
        window.set_bounds(
            i64::from(average) - i64::from(delta),
            i64::from(average) + i64::from(delta),
        );
        window
    }

    /// fail lowの値から下端を移し、上端を元の下端へ寄せてから半幅を広げる。
    pub(super) fn widen_low(&mut self, score: i32) {
        debug_assert!(score <= self.alpha);
        debug_assert!(self.alpha != -INFINITY);
        let delta = self
            .delta
            .expect("an aspiration retry must have a half-width");
        self.set_bounds(i64::from(score) - i64::from(delta), i64::from(self.alpha));
        self.delta = Some(grow_aspiration_delta(delta));
    }

    /// fail highの値から上端を移し、下端を元の上端を基準に寄せてから半幅を広げる。
    pub(super) fn widen_high(&mut self, score: i32) {
        debug_assert!(score >= self.beta);
        debug_assert!(self.beta != INFINITY);
        let delta = self
            .delta
            .expect("an aspiration retry must have a half-width");
        self.set_bounds(
            (i64::from(self.beta) - i64::from(delta)).max(i64::from(self.alpha)),
            i64::from(score) + i64::from(delta),
        );
        self.delta = Some(grow_aspiration_delta(delta));
    }

    /// 64ビット整数で求めた端を正規化して保持する。
    fn set_bounds(&mut self, alpha: i64, beta: i64) {
        let band_start = i64::from(REPETITION_BAND_START);
        self.alpha = if alpha <= -band_start {
            -INFINITY
        } else {
            i32::try_from(alpha).expect("finite aspiration alpha remains in the ordinary band")
        };
        self.beta = if beta >= band_start {
            INFINITY
        } else {
            i32::try_from(beta).expect("finite aspiration beta remains in the ordinary band")
        };
    }
}

/// 歩兵価値、調整係数、符号つき2乗の平均、およびワーカー番号から初期の半幅を求める。
/// 3つの項は64ビット整数で個別に切り捨ててから足し、結果を1以上にする。
pub(super) fn initial_aspiration_delta(
    pawn: i32,
    aspiration_delta: i32,
    aspiration_score_width: i32,
    signed_square_average: i64,
    worker_index: usize,
) -> i32 {
    let pawn = i64::from(pawn);
    let constant_term = pawn * i64::from(aspiration_delta) / 100;
    let score_term =
        i64::from(aspiration_score_width) * signed_square_average.abs() / (10_000 * pawn);
    let worker_term = (worker_index % 8) as i64 * pawn / 208;
    i32::try_from((constant_term + score_term + worker_term).max(1))
        .expect("initial aspiration delta remains an i32 score")
}

/// 半幅を1以上増やす項と調整係数を掛けた項の大きい方へ広げる。
/// 加算と乗算は64ビット整数で計算し、32ビット整数の上限で飽和させる。
pub(super) fn grow_aspiration_delta(delta: i32) -> i32 {
    let delta = i64::from(delta);
    let grown = (delta + 1).max(delta * i64::from(params::aspiration_growth()) / 100);
    i32::try_from(grown).unwrap_or(i32::MAX)
}

/// 秒読みつき時計の探索と固定時間だけを指定した探索で、中断時の着手選択に使う値と主変化。
/// 探索窓のαを上回った最大値を保持する。
pub(super) struct PartialResult {
    pub(super) score: i32,
    pub(super) pv: Vec<Move>,
}

/// 秒読みつき時計の探索と固定時間だけを指定した探索で、主ワーカーが保持する途中結果の採用材料。
/// 適用条件は[`super::time::uses_partial_results`]、採用条件は[`Self::partial_result`]に従う。
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

    /// 窓を移しながら読み直し、窓内で完了した反復だけを返す。
    ///
    /// 主ワーカーと補助ワーカーが共有し、直前の完了反復の最善手と、その手について各ワーカーが持つ
    /// 評価値の平均と符号つき2乗の平均を使う。深さ1は直前の完了反復がないため全窓で読む。
    /// 深さ2以上は平均があれば窓を置く。
    /// 直前の完了値は詰みを探す印の判定だけに使う。
    /// 反復の開始時にfail highの持ち越しを減らし、各窓はfail highの回数、持ち越し、
    /// およびincreaseDepthの回数`k`に応じて反復の深さ以下で読む。
    /// 窓の深さは`max(1, d − h − h_carry − floor(3 × (k + 1) / 4))`とし、
    /// 読み直しの間は`k`を変えない。
    /// fail lowではfail highの回数を0へ戻し、fail highでは1増やす。
    /// 値がα以下ならβを元のαへ寄せて下端を返り値から移し、
    /// β以上ならαを元のβを基準に寄せて上端を返り値から移す。
    /// どちらも窓を移した後に半幅を広げて再探索する。
    /// 等号も窓外れとし、fail-lowの最善手は置換表の助言としてだけ残して、完了結果や進捗へは採らない。
    /// 完了結果は、実際に読んだ窓の深さによらず、引数の反復の深さの結果として扱う。
    /// 根の置換表へは窓の深さで保存を要求するため、既存の置換規則により、浅い完了窓が
    /// 先に読んだ深い窓の記録を上書きしないことがある。
    /// 読み直し中の中断も`None`を返す。
    /// 窓の置き方と移し方の判断の理由は`docs/adr/center-aspiration-windows-on-the-best-move-average-and-move-both-edges-on-failure.md`に、
    /// fail highの減深と反復の深さの扱いの判断の理由は`docs/adr/reduce-aspiration-depth-after-fail-highs-and-late-iterations.md`にある。
    pub(super) fn search_iteration(
        &mut self,
        position: &Position,
        root_moves: &[Move],
        iteration_depth: u32,
        prev: Option<(Move, i32)>,
    ) -> Option<(Move, i32)> {
        self.seek_mate = is_seek_mate(
            prev.map(|(_, score)| score),
            iteration_depth,
            self.nnue.pawn_value(),
        );
        self.previous_pv.clone_from(&self.pv[0]);
        let mut reduction = FailHighReduction::begin(self.fail_high_recovery);
        self.fail_high_recovery = reduction.recovery;
        let averages = prev.and_then(|(best_move, _)| {
            self.root_move_stats
                .iter()
                .find(|(mv, _)| *mv == best_move)
                .and_then(|(_, stats)| stats.averages)
        });
        let mut window =
            AspirationWindow::initial(averages.map(|(average, signed_square_average)| {
                (
                    average,
                    initial_aspiration_delta(
                        self.nnue.pawn_value(),
                        params::aspiration_delta(),
                        params::aspiration_score_width(),
                        signed_square_average,
                        self.worker_index,
                    ),
                )
            }));
        loop {
            let window_depth = reduction.window_depth(iteration_depth, self.search_again_counter);
            let Some((best_move, score)) = self.search_root(
                position,
                root_moves,
                iteration_depth,
                window_depth,
                window.alpha,
                window.beta,
            ) else {
                self.current_root_nodes.clear();
                return None;
            };
            if score <= window.alpha {
                reduction = reduction.after_fail_low();
                window.widen_low(score);
            } else if score >= window.beta {
                reduction = reduction.after_fail_high();
                window.widen_high(score);
            } else {
                self.fail_high_recovery = reduction.recovery_after_completion();
                self.completed_root_nodes = std::mem::take(&mut self.current_root_nodes);
                if let Some(results) = &mut self.root_results {
                    results.previous_best = Some(best_move);
                }
                return Some((best_move, score));
            }
        }
    }

    /// ルート局面を反復の深さと窓の深さを分けて探索し、最善手と評価値を返す。
    ///
    /// `root_depth`には反復の深さを保持し、根のLMR、子の探索、historyとcorrectionの更新、
    /// および置換表への保存には窓の深さを使う。
    /// β以上で打ち切って下界、入力時のα以下なら上界、それ以外は正確な値として置換表への保存を要求する。
    /// 置換表への保存は窓の深さで要求し、既存の置換規則に従う。このため、浅い窓の結果が
    /// 先に深く読んだ窓の記録を上書きしないことがある。
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
        iteration_depth: u32,
        window_depth: u32,
        mut alpha: i32,
        beta: i32,
    ) -> Option<(Move, i32)> {
        #[cfg(feature = "invariants")]
        assert!(
            self.nmp_min_ply.is_none(),
            "null move verification state must not reach the root"
        );
        self.root_depth = iteration_depth;
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
            let lmr = applies_lmr(window_depth, index);
            let first_depth = if lmr {
                let reduction = lmr_reduction(LmrContext {
                    depth: window_depth,
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
                lmr_depth(window_depth, reduction, true, 0, self.root_depth)
            } else {
                window_depth - 1
            };
            let alpha_before = alpha;
            let before = self.nodes;
            let score = self.search_move(
                &mut position,
                mv,
                window_depth,
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
            if index > 0 && score > alpha_before {
                self.shared.record_best_move_change();
            }
            if let Some((_, stats)) = self.root_move_stats.iter_mut().find(|(m, _)| *m == mv) {
                stats.update(nodes, score);
            } else {
                let mut stats = RootMoveStats::default();
                stats.update(nodes, score);
                self.root_move_stats.push((mv, stats));
            }
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
                window_depth,
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
            window_depth,
            0,
            best_score,
            bound,
            best_capture,
            static_eval,
            None,
        );
        self.tt.store(
            key,
            window_depth,
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

//! 枝刈りと探索深さの削減の式。

use std::sync::OnceLock;

use crate::eval::Nnue;
use crate::eval::nnue::EVALUATION_LIMIT;
use crate::search::{MAX_PLY, REPETITION_BAND_START, in_repetition_band};
use minase_core::mv::Move;
use minase_core::position::Position;
use minase_core::rules::MoveRules;

use super::params;
use super::see::see_prunes;
use super::tt::Bound;

/// 詰みを探す局面の閾値の切片。歩兵価値に対する百分率。
/// Stockfishの750を歩兵価値208で換算した固定値（src/search.cpp:394-395）。
const SEEK_MATE_BASE: i64 = 361;

/// 詰みを探す局面の閾値で反復深さの2乗で割る項。歩兵価値に対する百分率。
/// Stockfishの220000を歩兵価値208で換算した固定値（src/search.cpp:394-395）。
const SEEK_MATE_DEPTH_FACTOR: i64 = 105769;

/// 前の反復の確定値から詰みを探す局面かを判定する。反復深さは1以上とする。
/// 深さの2乗による除算を先に切り捨て、歩兵価値で換算した閾値も切り捨てる。
pub(super) fn is_seek_mate(previous: Option<i32>, depth: u32, pawn: i32) -> bool {
    let depth = i64::from(depth);
    previous.is_some_and(|score| {
        let threshold =
            i64::from(pawn) * (SEEK_MATE_BASE + SEEK_MATE_DEPTH_FACTOR / (depth * depth)) / 100;
        i64::from(score).abs() >= threshold
    })
}

/// 着手1手分を含むnull moveの減深量から、詰みを探す場合は半分を切り捨てて引く。
/// Stockfishの`R -= seekMate * R / 2`に対応する（src/search.cpp:1045）。
pub(super) fn seek_mate_null_move_reduction(reduction: u32, seek_mate: bool) -> u32 {
    reduction - u32::from(seek_mate) * (reduction / 2)
}

/// 置換表の打ち切りでcutNodeと値の方向の一致を要求する深さの上限。
/// Stockfishの`depth > 4`に対応する（src/search.cpp:899）。
const TT_CUTOFF_NODE_TYPE_MAX_DEPTH: u32 = 4;

/// 零窓で置換表の値、深さ、種別、およびノードの種別が打ち切りを許すかを返す。
/// 深さはIIR適用前とし、`d_tt > d - [v <= beta]`を判定する（Stockfish src/search.cpp:894-899）。
pub(super) fn zero_window_tt_cutoff(
    value: i32,
    bound: Bound,
    tt_depth: u32,
    depth: u32,
    beta: i32,
    cut_node: bool,
) -> bool {
    let deep_enough = if value <= beta {
        tt_depth >= depth
    } else {
        tt_depth > depth
    };
    let fail_high = value >= beta;
    let required_bound = if fail_high {
        Bound::Lower
    } else {
        Bound::Upper
    };
    !in_repetition_band(value)
        && deep_enough
        && (bound == Bound::Exact || bound == required_bound)
        && (cut_node == fail_high || depth > TT_CUTOFF_NODE_TYPE_MAX_DEPTH)
}

/// 補正後の静的評価を評価関数の出力範囲に収める（Stockfish src/search.cpp:113-118）。
pub(super) fn corrected_static_eval(raw: i32, correction: i32) -> i32 {
    (raw + correction).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT)
}

/// 同じ手番の2手前より静的評価が上がったかを返す（Stockfish src/search.cpp:883）。
/// 2手前がなければ偽とし、null moveの区間も通常どおり比較する。
pub(super) fn is_improving(static_eval: i32, previous: Option<i32>) -> bool {
    previous.is_some_and(|value| static_eval > value)
}

/// 相手から見た静的評価が直前の手で悪化したかを返す（Stockfish src/search.cpp:884）。
/// 根では1手前の番兵値の符号反転との比較に相当する真を返す。
pub(super) fn is_opponent_worsening(static_eval: i32, previous: Option<i32>) -> bool {
    previous.is_none_or(|value| static_eval > -value)
}

/// 静的評価を、方向の合う置換表の値で置き換える（Stockfish src/search.cpp:862-865）。
/// 経路に依存する専用の帯と詰み帯の値は、判定用の評価値に使わない。
pub(super) fn pruning_eval(static_eval: i32, tt_hit: Option<(i32, Bound)>) -> i32 {
    if let Some((value, bound)) = tt_hit
        && value.abs() < REPETITION_BAND_START
        && (bound == Bound::Exact
            || (value > static_eval && bound == Bound::Lower)
            || (value <= static_eval && bound == Bound::Upper))
    {
        return value;
    }
    static_eval
}

/// LMRの減深量表に保持する手番号の列数。
/// 256手を超える局面は稀で、後方の減深量は上限で抑えるため、それ以降は最終列を使う。
const LMR_MOVE_COUNT: usize = 256;

/// 残り深さと手番号から引く、上限適用前のLMR減深量表を1/100手単位で1回だけ生成する。
pub(super) fn lmr_table() -> &'static [[u16; LMR_MOVE_COUNT]; MAX_PLY as usize + 1] {
    static TABLE: OnceLock<[[u16; LMR_MOVE_COUNT]; MAX_PLY as usize + 1]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let divisor = params::lmr_divisor();
        let mut table = [[0; LMR_MOVE_COUNT]; MAX_PLY as usize + 1];
        for (depth, row) in table.iter_mut().enumerate().skip(1) {
            for (index, value) in row.iter_mut().enumerate().skip(1) {
                *value = lmr_base(depth, index, divisor);
            }
        }
        table
    })
}

/// LMR表の1要素を、百分率の除数から1/100手単位で計算し、端数を切り捨てる。
pub(super) fn lmr_base(depth: usize, index: usize, divisor: i32) -> u16 {
    let reduction = (depth as f64).ln() * (index as f64).ln() / (f64::from(divisor) / 100.0);
    (100.0 * reduction).floor() as u16
}

/// 静かな手のfutility pruningを行うlmrDepthの上限。
/// Stockfishの`lmrDepth < 12`に対応する（src/search.cpp:1247）。
pub(super) const QUIET_FUTILITY_LMR_DEPTH_LIMIT: i32 = 12;

/// 静かな手のfutility見積りの式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct FutilityCoefficients {
    pub(super) futility_base: i32,
    pub(super) futility_lmr_depth: i32,
    pub(super) futility_above_alpha: i32,
}

impl FutilityCoefficients {
    pub(super) fn current() -> Self {
        Self {
            futility_base: params::futility_base(),
            futility_lmr_depth: params::futility_lmr_depth(),
            futility_above_alpha: params::futility_above_alpha(),
        }
    }

    /// 補正後の静的評価と枝刈り用のlmrDepthから、静かな手の見積りを求める。
    /// Stockfishの式に対応する（src/search.cpp:1242-1243）。
    pub(super) fn futility_value(
        &self,
        pawn: i32,
        static_eval: i32,
        alpha: i32,
        lmr_depth: i32,
    ) -> i32 {
        let percent = i64::from(self.futility_base)
            + i64::from(self.futility_lmr_depth) * i64::from(lmr_depth)
            + i64::from(self.futility_above_alpha) * i64::from(static_eval > alpha);
        (i64::from(static_eval) + i64::from(pawn) * percent / 100) as i32
    }

    /// lmrDepthと見積りが静かな手のfutility pruningを許す場合、その見積りを返す。
    pub(super) fn futility_pruning_value(
        &self,
        pawn: i32,
        static_eval: i32,
        alpha: i32,
        lmr_depth: i32,
    ) -> Option<i32> {
        if lmr_depth >= QUIET_FUTILITY_LMR_DEPTH_LIMIT {
            return None;
        }
        let value = self.futility_value(pawn, static_eval, alpha, lmr_depth);
        (value <= alpha).then_some(value)
    }
}

/// 静かな手のfutility見積りを求める（Stockfish src/search.cpp:1242-1243）。
#[cfg(test)]
pub(super) fn futility_value(pawn: i32, static_eval: i32, alpha: i32, lmr_depth: i32) -> i32 {
    FutilityCoefficients::current().futility_value(pawn, static_eval, alpha, lmr_depth)
}

/// lmrDepthと見積りから、静かな手のfutility pruningに使える見積りを返す。
pub(super) fn futility_pruning_value(
    pawn: i32,
    static_eval: i32,
    alpha: i32,
    lmr_depth: i32,
) -> Option<i32> {
    FutilityCoefficients::current().futility_pruning_value(pawn, static_eval, alpha, lmr_depth)
}

/// 枝刈りした手の見積りで最善値だけを上げてよいかを返す。
/// Stockfishの最善値の更新に対応する（src/search.cpp:1249-1251）。
pub(super) fn should_raise_futility_best_score(best_score: i32, futility_value: i32) -> bool {
    best_score < futility_value
        && best_score > -REPETITION_BAND_START
        && best_score < REPETITION_BAND_START
        && futility_value > -REPETITION_BAND_START
        && futility_value < REPETITION_BAND_START
}

/// 残り深さによらず適用するlate move pruningで、静かな手の上限の式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct LateMoveCoefficients {
    pub(super) lmp_non_improving: i32,
    pub(super) lmp_base: i32,
    pub(super) lmp_slope: i32,
}

impl LateMoveCoefficients {
    pub(super) fn current() -> Self {
        Self {
            lmp_non_improving: params::lmp_non_improving(),
            lmp_base: params::lmp_base(),
            lmp_slope: params::lmp_slope(),
        }
    }

    pub(super) fn late_move_limit(&self, depth: u32, improving: bool) -> i32 {
        let depth = i64::from(depth);
        let scale = if improving {
            100
        } else {
            self.lmp_non_improving
        };
        ((i64::from(self.lmp_base) + i64::from(self.lmp_slope) * depth * depth) * i64::from(scale)
            / 10000) as i32
    }
}

/// late move pruningで数える静かな手の数の上限を、残り深さによらず整数除算で求める。
///
/// improvingなら `(LmpBase + LmpSlope × 深さ²) / 100`、
/// それ以外は `(LmpBase + LmpSlope × 深さ²) × LmpNonImproving / 10000` とする。
/// 深さはIIR適用後、improvingはnull move後の更新後の値を使い、端数は最後に1回だけ切り捨てる。
/// 手番号ではなく、現在の手より前に返った非捕獲手のうち、置換表の手、成る手、killer手を除いた数と比べる。
/// 枝刈りで展開しなかった手もこの数に含める。
/// 捕獲手に静かな手の枠を消費させない理由は`docs/adr/count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md`にある。
pub(super) fn late_move_limit(depth: u32, improving: bool) -> i32 {
    LateMoveCoefficients::current().late_move_limit(depth, improving)
}

/// 深さ1〜3のSEE余裕値を求める。
pub(super) fn see_margin(pawn: i32, depth: u32) -> i32 {
    let percent = match depth {
        1 => params::see_margin1(),
        2 => params::see_margin2(),
        3 => params::see_margin3(),
        _ => unreachable!("SEE pruning applies only at depths 1 to 3"),
    };
    pawn * percent / 100
}

/// reverse futilityの残り深さの上限。Stockfishの`depth < (seekMate ? 6 : 19)`に対応する。
/// 詰みを探す場合を除く18手を使う（Stockfish src/search.cpp:1022）。
pub(super) const REVERSE_FUTILITY_MAX_DEPTH: u32 = 18;

/// 詰みを探す局面でのreverse futilityの残り深さの上限。
/// Stockfishの`depth < (seekMate ? 6 : 19)`に対応する固定値（src/search.cpp:1021-1022）。
pub(super) const SEEK_MATE_REVERSE_FUTILITY_MAX_DEPTH: u32 = 5;

/// reverse futilityの`mult`の上限。単位は歩兵価値に対する百分率である。
/// Stockfishの85を歩兵価値208で換算した41%（Stockfish src/search.cpp:1025）。
const REVERSE_FUTILITY_MULT_MAX: i64 = 41;

/// reverse futilityの余裕値の式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct ReverseFutilityCoefficients {
    pub(super) reverse_futility_mult_base: i32,
    pub(super) reverse_futility_mult_slope: i32,
    pub(super) reverse_futility_no_tt_hit: i32,
    pub(super) reverse_futility_improving: i32,
    pub(super) reverse_futility_opponent_worsening: i32,
    pub(super) reverse_futility_correction: i32,
}

impl ReverseFutilityCoefficients {
    pub(super) fn current() -> Self {
        Self {
            reverse_futility_mult_base: params::reverse_futility_mult_base(),
            reverse_futility_mult_slope: params::reverse_futility_mult_slope(),
            reverse_futility_no_tt_hit: params::reverse_futility_no_tt_hit(),
            reverse_futility_improving: params::reverse_futility_improving(),
            reverse_futility_opponent_worsening: params::reverse_futility_opponent_worsening(),
            reverse_futility_correction: params::reverse_futility_correction(),
        }
    }

    pub(super) fn reverse_futility_margin(
        &self,
        pawn: i32,
        depth: u32,
        tt_hit: bool,
        improving: bool,
        opponent_worsening: bool,
        correction: i32,
    ) -> i32 {
        let depth = i64::from(depth);
        let mult = (i64::from(self.reverse_futility_mult_base)
            + i64::from(self.reverse_futility_mult_slope) * depth)
            .min(REVERSE_FUTILITY_MULT_MAX)
            - i64::from(self.reverse_futility_no_tt_hit) * i64::from(!tt_hit);
        let scaled_depth = 100 * depth
            - i64::from(self.reverse_futility_improving) * i64::from(improving)
            - i64::from(self.reverse_futility_opponent_worsening) * i64::from(opponent_worsening);
        (i64::from(pawn) * mult * scaled_depth / 10000
            + i64::from(self.reverse_futility_correction) * i64::from(correction).abs() / 100)
            as i32
    }
}

/// reverse futilityの余裕値を求める（Stockfish src/search.cpp:1025-1030）。
/// 深さはIIR適用後、improvingはnull move後の更新前、補正はノードの最初に読んだ値とする。
/// 深さと良化による項、補正の絶対値による項を別々に0方向へ丸めて足す。
pub(super) fn reverse_futility_margin(
    pawn: i32,
    depth: u32,
    tt_hit: bool,
    improving: bool,
    opponent_worsening: bool,
    correction: i32,
) -> i32 {
    ReverseFutilityCoefficients::current().reverse_futility_margin(
        pawn,
        depth,
        tt_hit,
        improving,
        opponent_worsening,
        correction,
    )
}

/// 深さ1〜2のrazoring余裕値を求める。
pub(super) fn razoring_margin(pawn: i32, depth: u32) -> i32 {
    let percent = match depth {
        1 => params::razoring_margin1(),
        2 => params::razoring_margin2(),
        _ => unreachable!("razoring applies only at depths 1 to 2"),
    };
    pawn * percent / 100
}

/// null moveの減深量の式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct NullMoveCoefficients {
    pub(super) null_move_eval_scale: i32,
    pub(super) null_move_base: i32,
    pub(super) null_move_slope: i32,
}

impl NullMoveCoefficients {
    pub(super) fn current() -> Self {
        Self {
            null_move_eval_scale: params::null_move_eval_scale(),
            null_move_base: params::null_move_base(),
            null_move_slope: params::null_move_slope(),
        }
    }

    pub(super) fn null_move_reduction(
        &self,
        depth: u32,
        static_eval: i32,
        beta: i32,
        pawn: i32,
    ) -> u32 {
        let extra = (((i64::from(static_eval) - i64::from(beta))
            * i64::from(self.null_move_eval_scale)
            / 100)
            / (2 * i64::from(pawn)))
        .clamp(0, 3) as u32;
        (self.null_move_base as u32 + depth * self.null_move_slope as u32) / 1200 + extra
    }
}

/// 深さと静的評価のβに対する余裕からnull moveの減深量を求める。
///
/// 基本量は `(NullMoveBase + 深さ × NullMoveSlope) / 1200` 手である。
/// `(静的評価 − β) × NullMoveEvalScale` を100、歩兵価値の2倍で順に整数除算し、0〜3手に制限して加える。
/// 縮めた深さが0になり静止探索だけで打ち切りを判定する頻度を抑えるため、加算に上限を設ける。
/// 尺度0では基本量と一致する。
pub(super) fn null_move_reduction(depth: u32, static_eval: i32, beta: i32, pawn: i32) -> u32 {
    NullMoveCoefficients::current().null_move_reduction(depth, static_eval, beta, pawn)
}

/// LMRの式へ渡す、その手を読む直前のノードと手の情報。
#[derive(Clone, Copy)]
pub(super) struct LmrContext {
    pub(super) depth: u32,
    pub(super) index: usize,
    pub(super) improving: bool,
    pub(super) pv_node: bool,
    pub(super) tt_pv: bool,
    pub(super) cut_node: bool,
    /// 置換表で照合した値と深さ。記録手の合法性とは独立である。
    pub(super) tt_hit: Option<(i32, u32)>,
    /// 合法性を確認した置換表の手の捕獲判定。手がなければNone。
    pub(super) tt_capture: Option<bool>,
    pub(super) is_tt_move: bool,
    pub(super) capture: bool,
    /// 置換表の手、捕獲手、killer手以外の非捕獲手だけが持つ履歴値。
    pub(super) history: Option<i32>,
    pub(super) cutoff_count: u32,
    pub(super) alpha: i32,
    pub(super) eval: i32,
    pub(super) pawn: i32,
}

/// LMRの符号つき減深量の式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct LmrCoefficients {
    pub(super) lmr_non_improving: i32,
    pub(super) lmr_base: i32,
    pub(super) lmr_pv_reduction: i32,
    pub(super) lmr_tt_pv: i32,
    pub(super) lmr_history_threshold: i32,
    pub(super) lmr_offset: i32,
    pub(super) lmr_move_count: i32,
    pub(super) lmr_tt_pv_base: i32,
    pub(super) lmr_tt_pv_pv: i32,
    pub(super) lmr_tt_pv_value: i32,
    pub(super) lmr_tt_pv_depth: i32,
    pub(super) lmr_tt_pv_depth_cut_node: i32,
    pub(super) lmr_cut_node: i32,
    pub(super) lmr_cut_node_no_tt_move: i32,
    pub(super) lmr_tt_capture: i32,
    pub(super) lmr_cutoff_count: i32,
    pub(super) lmr_cutoff_count_many: i32,
    pub(super) lmr_cutoff_count_all_node: i32,
    pub(super) lmr_tt_move: i32,
    pub(super) lmr_alpha_eval: i32,
    pub(super) lmr_all_node: i32,
}

impl LmrCoefficients {
    pub(super) fn current() -> Self {
        Self {
            lmr_non_improving: params::lmr_non_improving(),
            lmr_base: params::lmr_base(),
            lmr_pv_reduction: params::lmr_pv_reduction(),
            lmr_tt_pv: params::lmr_tt_pv(),
            lmr_history_threshold: params::lmr_history_threshold(),
            lmr_offset: params::lmr_offset(),
            lmr_move_count: params::lmr_move_count(),
            lmr_tt_pv_base: params::lmr_tt_pv_base(),
            lmr_tt_pv_pv: params::lmr_tt_pv_pv(),
            lmr_tt_pv_value: params::lmr_tt_pv_value(),
            lmr_tt_pv_depth: params::lmr_tt_pv_depth(),
            lmr_tt_pv_depth_cut_node: params::lmr_tt_pv_depth_cut_node(),
            lmr_cut_node: params::lmr_cut_node(),
            lmr_cut_node_no_tt_move: params::lmr_cut_node_no_tt_move(),
            lmr_tt_capture: params::lmr_tt_capture(),
            lmr_cutoff_count: params::lmr_cutoff_count(),
            lmr_cutoff_count_many: params::lmr_cutoff_count_many(),
            lmr_cutoff_count_all_node: params::lmr_cutoff_count_all_node(),
            lmr_tt_move: params::lmr_tt_move(),
            lmr_alpha_eval: params::lmr_alpha_eval(),
            lmr_all_node: params::lmr_all_node(),
        }
    }

    /// 枝刈りより前に決まるLMRの減深量を1/100手単位で返す。
    /// Stockfishの枝刈り前の減深量に対応する（src/search.cpp:1183-1192, 1917-1920）。
    pub(super) fn lmr_reduction_before_pruning(
        &self,
        table: u16,
        improving: bool,
        pv_node: bool,
        tt_pv: bool,
    ) -> i32 {
        let table = i64::from(table);
        (table * (100 + i64::from(self.lmr_non_improving) * i64::from(!improving)) / 100
            + i64::from(self.lmr_base)
            - i64::from(self.lmr_pv_reduction) * i64::from(pv_node)
            + i64::from(self.lmr_tt_pv) * i64::from(tt_pv)) as i32
    }

    /// 手を読む深さから枝刈り前の減深量を引き、枝刈り用のlmrDepthを返す。
    pub(super) fn pruning_lmr_depth(
        &self,
        table: u16,
        depth: u32,
        improving: bool,
        pv_node: bool,
        tt_pv: bool,
    ) -> i32 {
        depth as i32 - 1 - self.lmr_reduction_before_pruning(table, improving, pv_node, tt_pv) / 100
    }

    pub(super) fn lmr_reduction(&self, table: u16, context: LmrContext) -> i32 {
        let c = context;
        let all_node = !c.pv_node && !c.cut_node;
        let r_pre =
            i64::from(self.lmr_reduction_before_pruning(table, c.improving, c.pv_node, c.tt_pv));
        let threshold = self.lmr_history_threshold;
        let history = match c.history {
            Some(value) if value >= threshold => -100,
            Some(value) if value <= -threshold => 100,
            _ => 0,
        };
        let mut r = r_pre + i64::from(self.lmr_offset)
            - i64::from(self.lmr_move_count) * (c.index as i64 + 1)
            + history;
        if c.tt_pv {
            r -= i64::from(self.lmr_tt_pv_base)
                + i64::from(self.lmr_tt_pv_pv) * i64::from(c.pv_node)
                + i64::from(self.lmr_tt_pv_value)
                    * i64::from(c.tt_hit.is_some_and(|(value, _)| value > c.alpha))
                + (i64::from(self.lmr_tt_pv_depth)
                    + i64::from(self.lmr_tt_pv_depth_cut_node) * i64::from(c.cut_node))
                    * i64::from(c.tt_hit.is_some_and(|(_, depth)| depth >= c.depth));
        }
        if c.cut_node {
            r += i64::from(self.lmr_cut_node)
                + i64::from(self.lmr_cut_node_no_tt_move) * i64::from(c.tt_capture.is_none());
        }
        r += i64::from(self.lmr_tt_capture) * i64::from(c.tt_capture == Some(true));
        // 子のβ打ち切り数の閾値1と2はStockfish src/search.cpp:1364-1369と同じ。
        if c.cutoff_count > 1 {
            r += i64::from(self.lmr_cutoff_count)
                + i64::from(self.lmr_cutoff_count_many) * i64::from(c.cutoff_count > 2)
                + i64::from(self.lmr_cutoff_count_all_node) * i64::from(all_node);
        } else if c.is_tt_move {
            r -= i64::from(self.lmr_tt_move);
        }
        if !c.capture && c.alpha.abs() < REPETITION_BAND_START {
            let pawn = i64::from(c.pawn);
            // −64と96を歩兵価値208で換算した−31%と46%（Stockfish src/search.cpp:1383-1384）。
            let difference =
                (i64::from(c.alpha) - i64::from(c.eval)).clamp(-31 * pawn / 100, 46 * pawn / 100);
            r += i64::from(self.lmr_alpha_eval) * difference / pawn;
        }
        if all_node {
            // 分母の定数105は268/256の換算値（Stockfish src/search.cpp:1387-1388）。
            r += r * i64::from(self.lmr_all_node) / (100 * i64::from(c.depth) + 105);
        }
        r as i32
    }
}

/// ノードと手番号から、枝刈り用の符号つきlmrDepthを返す。
///
/// `depth - 1 - r_pre / 100`を0方向へ丸める。表の手番号は0始まりで、
/// 256以上なら最終列を使う。history以降のLMRの項と減深・延長の上限は含めない。
/// Stockfishの枝刈り用の深さに対応する（src/search.cpp:1203）。
pub(super) fn pruning_lmr_depth(
    depth: u32,
    index: usize,
    improving: bool,
    pv_node: bool,
    tt_pv: bool,
) -> i32 {
    LmrCoefficients::current().pruning_lmr_depth(
        lmr_table()[depth as usize][index.min(LMR_MOVE_COUNT - 1)],
        depth,
        improving,
        pv_node,
        tt_pv,
    )
}

/// ノードと手の情報から、符号つきのLMR減深量を1/100手単位で返す。
///
/// Stockfish src/search.cpp:1183-1192, 1345-1388, 1917-1920の各項を個別に0方向へ丸める。
/// 表の手番号は0始まりで、256以上なら表だけ最終列を使う。手番号の減算項は実際の番号を使う。
/// historyの補正は対象の静かな手だけに掛け、閾値以上なら−100、負の閾値以下なら+100とする。
/// 負の減深量は延長に使うので、ここでは上限も下限も掛けない。
pub(super) fn lmr_reduction(context: LmrContext) -> i32 {
    LmrCoefficients::current().lmr_reduction(
        lmr_table()[context.depth as usize][context.index.min(LMR_MOVE_COUNT - 1)],
        context,
    )
}

/// 減深量を0方向へ丸め、正の減深だけを係数の上限で抑えて子の深さを求める。
/// 延長の上限はplyが反復深さの2倍未満なら2、それ以外は0である。
/// 最低1手を確保した後、PVノードでは1手足す（Stockfish src/search.cpp:1402）。
pub(super) fn lmr_depth(
    depth: u32,
    reduction: i32,
    pv_node: bool,
    ply: u32,
    root_depth: u32,
) -> u32 {
    lmr_depth_with_coefficient(
        depth,
        reduction,
        pv_node,
        ply,
        root_depth,
        params::lmr_max_reduction(),
    )
}

pub(super) fn lmr_depth_with_coefficient(
    depth: u32,
    reduction: i32,
    pv_node: bool,
    ply: u32,
    root_depth: u32,
    lmr_max_reduction: i32,
) -> u32 {
    let extension = if ply < 2 * root_depth { 2 } else { 0 };
    let reduction = (reduction / 100).clamp(-extension, lmr_max_reduction);
    (depth as i32 - 1 - reduction).max(1) as u32 + u32::from(pv_node)
}

/// αを超えたLMRの結果に応じて、再探索の通常深さを調整する。
/// 深い探索と浅い探索の条件は独立に判定する（Stockfish src/search.cpp:1414-1417）。
pub(super) fn lmr_research_depth(
    normal: u32,
    reduced: u32,
    value: i32,
    best: i32,
    pawn: i32,
) -> u32 {
    lmr_research_depth_with_margins(
        normal,
        reduced,
        value,
        best,
        pawn,
        params::lmr_deeper_margin(),
        params::lmr_shallower_margin(),
    )
}

pub(super) fn lmr_research_depth_with_margins(
    normal: u32,
    reduced: u32,
    value: i32,
    best: i32,
    pawn: i32,
    deeper_margin: i32,
    shallower_margin: i32,
) -> u32 {
    let deeper = reduced < normal
        && i64::from(value) > i64::from(best) + i64::from(deeper_margin) * i64::from(pawn) / 100;
    let shallower =
        i64::from(value) < i64::from(best) + i64::from(shallower_margin) * i64::from(pawn) / 100;
    normal + u32::from(deeper) - u32::from(shallower)
}

/// LMRの分岐を通らない零窓の先頭手の深さを求める。
/// 置換表の手がなければ加算した後、511と536で1手ずつ減らす。2手目は通常深さが2より深い場合だけとする。
/// 閾値は5234/1024と5487/1024の換算値である（Stockfish src/search.cpp:1428-1436）。
pub(super) fn non_lmr_depth(normal: u32, reduction: i32, has_tt_move: bool) -> u32 {
    non_lmr_depth_with_coefficient(normal, reduction, has_tt_move, params::lmr_no_tt_move())
}

pub(super) fn non_lmr_depth_with_coefficient(
    normal: u32,
    reduction: i32,
    has_tt_move: bool,
    lmr_no_tt_move: i32,
) -> u32 {
    let reduction = reduction + if has_tt_move { 0 } else { lmr_no_tt_move };
    normal.saturating_sub(u32::from(reduction > 511) + u32::from(reduction > 536 && normal > 2))
}

/// 静的交換評価で損と判定できる捕獲手かを返す。
pub(super) fn capture_is_pruned_by_see(
    position: &Position,
    rules: MoveRules,
    nnue: &Nnue,
    mv: Move,
) -> bool {
    see_prunes(position, rules, nnue, mv, 0)
}

//! 着手の順序付けと手の成績の更新。
//!
//! 最善値が探索開始時のαを上回るノードでは、historyの加点を
//! `min(傾き × 深さ − 切片, 上限)`へ置換表の手と親のstatScoreの項を加えて求める。
//! 減点も同じ形の1次式で求め、αを上回らなかった探索済みの手へ与える。
//! Butterfly historyと捕獲履歴は、それぞれ上限7183と10692の重力式で更新する。

use core::cmp::Reverse;

use crate::eval::Nnue;
use minase_core::MoveGenerator;
use minase_core::board::{BOARD_SQUARE_COUNT, Bitboard, Square};
use minase_core::mv::Move;
use minase_core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use minase_core::position::Position;

use super::capture_history::CaptureHistory;
use super::history::{
    ContinuationHistory, HISTORY_LIMIT, HistoryTable, LOW_PLY_HISTORY_LIMIT,
    LOW_PLY_HISTORY_PLY_COUNT, LowPlyHistory, PawnHistory, history_gravity,
};
use super::params;
use super::royal::captures_last_royal;
use super::searcher::{KILLER_COUNT, MoveRecord, PathMove, Searcher};
use super::see::see_prunes;

/// Continuation historyを更新する文脈の距離と重み。
/// Stockfishの値を使う（src/search.cpp:2049-2051）。
const CONTINUATION_WEIGHTS: [(usize, i32); 5] = [(1, 520), (2, 390), (3, 145), (4, 251), (6, 209)];

/// 更新前の正値の数に応じてcontinuation historyへ掛ける倍率。
/// Stockfishの値を使う（src/search.cpp:2053-2070）。
const CONTINUATION_MULTIPLIERS: [i32; 6] = [94, 103, 110, 106, 119, 126];

/// 静かな手の並べ替えで使う調整係数。
#[derive(Clone, Copy)]
pub(super) struct QuietOrderingCoefficients {
    pub(super) check_bonus: i32,
    pub(super) threat_weight: i32,
}

impl QuietOrderingCoefficients {
    pub(super) fn current() -> Self {
        Self {
            check_bonus: params::ordering_check_bonus(),
            threat_weight: params::ordering_threat_weight(),
        }
    }
}

/// 静かな手の並べ替え値へ渡す、履歴と局面から求めた値。
#[derive(Clone, Copy)]
pub(super) struct QuietOrderingInput {
    pub(super) butterfly: i32,
    pub(super) pawn: i32,
    pub(super) continuation: i32,
    pub(super) ply: u32,
    pub(super) from: minase_core::Square,
    pub(super) to: minase_core::Square,
    pub(super) check: bool,
    pub(super) moving_piece_value: i32,
    pub(super) pawn_value: i32,
    pub(super) threat_delta: i32,
}

/// 静かな手の並べ替え値を求める。
///
/// Low ply historyはply 5未満でだけ加え、すべての積と合計は`i64`で計算する。
/// 整数除算は0方向へ丸める（src/movepick.cpp:233-251）。
pub(super) fn quiet_ordering_value(
    input: QuietOrderingInput,
    low_ply_history: &LowPlyHistory,
    coefficients: QuietOrderingCoefficients,
) -> i64 {
    assert!(input.pawn_value > 0, "pawn value must be positive");
    let mut value =
        2 * i64::from(input.butterfly) + 2 * i64::from(input.pawn) + i64::from(input.continuation);
    if input.ply < LOW_PLY_HISTORY_PLY_COUNT as u32 {
        let low_ply =
            low_ply_history[input.ply as usize][input.from.dense_index()][input.to.dense_index()];
        value += 8 * i64::from(low_ply) / (1 + i64::from(input.ply));
    }
    value += i64::from(coefficients.check_bonus) * i64::from(input.check);
    let threat = i64::from(input.moving_piece_value) * i64::from(coefficients.threat_weight)
        / i64::from(input.pawn_value);
    value + threat * i64::from(input.threat_delta)
}

/// 相手の同価値の駒をまとめた、価値順の累積疑似利き。
pub(super) struct LesserPieceThreats {
    /// 各要素は駒価値と、その価値以下の相手駒の疑似利きの和集合である。
    pub(super) groups: Vec<(i32, Bitboard)>,
}

impl LesserPieceThreats {
    /// 相手の盤上の各駒を実際の駒コードで評価し、同価値の群ごとに累積疑似利きを作る。
    pub(super) fn with_piece_values(
        position: &Position,
        mut piece_value: impl FnMut(PieceCode) -> i32,
    ) -> Self {
        let enemy = position.side_to_move().opposite();
        let mut controls: Vec<_> = position
            .pieces_of(enemy)
            .into_iter()
            .map(|square| {
                let piece = position
                    .piece_at(square)
                    .expect("enemy piece square must contain a piece");
                (piece_value(piece), position.piece_control(square))
            })
            .collect();
        controls.sort_unstable_by_key(|&(value, _)| value);

        let mut groups = Vec::with_capacity(controls.len());
        let mut cumulative = Bitboard::EMPTY;
        let mut index = 0;
        while index < controls.len() {
            let value = controls[index].0;
            while index < controls.len() && controls[index].0 == value {
                cumulative |= controls[index].1;
                index += 1;
            }
            groups.push((value, cumulative));
        }
        Self { groups }
    }

    /// `piece_value`より価値が厳密に小さい相手駒の疑似利きの和集合を返す。
    pub(super) fn below(&self, piece_value: i32) -> Bitboard {
        let count = self
            .groups
            .partition_point(|&(value, _)| value < piece_value);
        if count == 0 {
            Bitboard::EMPTY
        } else {
            self.groups[count - 1].1
        }
    }
}

/// 王駒を除き、価値の低い駒の利きから出る手を1、入る手を−1として返す。
pub(super) fn lesser_threat_delta(moving_kind: PieceKind, threats: Bitboard, mv: Move) -> i32 {
    if matches!(moving_kind, PieceKind::King | PieceKind::CrownPrince) {
        return 0;
    }
    i32::from(threats.contains(mv.from)) - i32::from(threats.contains(mv.to))
}

/// 駒種ごとに、その駒を置くと相手の王駒へ直接の王手になる升を保持する。
pub(super) struct DirectCheckSquares {
    occupied: Bitboard,
    opponent: Color,
    royal: Option<Square>,
    by_kind: [Option<Bitboard>; PIECE_KIND_COUNT],
}

impl DirectCheckSquares {
    /// 相手の王駒がちょうど1枚ならその升を保持し、それ以外なら全駒種を空集合とする。
    pub(super) fn new(position: &Position) -> Self {
        let opponent = position.side_to_move().opposite();
        let royals = position.royal_pieces(opponent);
        let royal = if royals.popcount() == 1 {
            royals.lsb()
        } else {
            None
        };
        Self {
            occupied: position.occupied(),
            opponent,
            royal,
            by_kind: [None; PIECE_KIND_COUNT],
        }
    }

    /// `kind`の駒を`to`へ置く手が、現在の占有で直接の王手になるかを返す。
    pub(super) fn contains(&mut self, kind: PieceKind, to: Square) -> bool {
        let Some(royal) = self.royal else {
            return false;
        };
        let occupied = self.occupied;
        let opponent = self.opponent;
        let squares = self.by_kind[kind.index()].get_or_insert_with(|| {
            // 中将棋の動きは180度回転と先後の入れ替えについて対称なので、
            // 相手色の同じ駒を王駒升に置いた疑似利きが、直接王手になる升の集合に等しい。
            Position::piece_control_with_occupancy(occupied, opponent, kind, royal)
        });
        squares.contains(to)
    }
}

/// 直接の王手であり、SEEがStockfishの−75以上か判定不能なら真を返す。
/// 直接の王手でない手ではSEEを求めない（src/movepick.cpp:241-242）。
pub(super) fn qualifies_for_check_bonus(
    position: &Position,
    generator: &MoveGenerator,
    nnue: &Nnue,
    direct_checks: &mut DirectCheckSquares,
    moving_kind: PieceKind,
    mv: Move,
) -> bool {
    let kind_after_move = if mv.promote {
        moving_kind
            .promoted()
            .expect("promoting move must have a promotable piece")
    } else {
        moving_kind
    };
    direct_checks.contains(kind_after_move, mv.to)
        && !see_prunes(
            position,
            generator.rules(),
            nnue,
            mv,
            i32::try_from(i64::from(nnue.pawn_value()) * 75 / 208)
                .expect("ordering check SEE margin must fit i32"),
        )
}

/// Historyの加点と減点の式に渡す調整係数。
#[derive(Clone, Copy)]
pub(super) struct HistoryUpdateCoefficients {
    pub(super) bonus_slope: i32,
    pub(super) bonus_base: i32,
    pub(super) bonus_cap: i32,
    pub(super) malus_slope: i32,
    pub(super) malus_base: i32,
    pub(super) malus_cap: i32,
    pub(super) malus_decay: i32,
    pub(super) tt_move_bonus: i32,
}

impl HistoryUpdateCoefficients {
    pub(super) fn current() -> Self {
        Self {
            bonus_slope: params::history_bonus_slope(),
            bonus_base: params::history_bonus_base(),
            bonus_cap: params::history_bonus_cap(),
            malus_slope: params::history_malus_slope(),
            malus_base: params::history_malus_base(),
            malus_cap: params::history_malus_cap(),
            malus_decay: params::history_malus_decay(),
            tt_move_bonus: params::history_tt_move_bonus(),
        }
    }

    /// 残り深さ、置換表の手、親のstatScore、および探索済みの手数から加点と減点を求める。
    /// Stockfishのhistory bonusとmalusに対応する（src/search.cpp:2004-2011）。
    pub(super) fn updates(
        &self,
        depth: u32,
        tt_move: bool,
        parent_stat_score: i32,
        zero_window: bool,
        searched_moves: usize,
    ) -> HistoryUpdates {
        let depth = i64::from(depth);
        let mut bonus = (i64::from(self.bonus_slope) * depth - i64::from(self.bonus_base))
            .min(i64::from(self.bonus_cap));
        bonus +=
            i64::from(self.tt_move_bonus) * i64::from(tt_move) + i64::from(parent_stat_score / 28);
        if zero_window {
            let searched_moves =
                i64::try_from(searched_moves).expect("searched move count must fit i64");
            bonus += bonus * searched_moves / 256;
        }
        let malus = (i64::from(self.malus_slope) * depth - i64::from(self.malus_base))
            .min(i64::from(self.malus_cap));
        HistoryUpdates {
            bonus: i32::try_from(bonus).expect("history bonus must fit i32"),
            malus: i32::try_from(malus).expect("history malus must fit i32"),
            malus_decay: self.malus_decay,
        }
    }
}

/// 1回のhistory更新で使う符号つきの加点、減点、および静かな手の減点の逓減率。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HistoryUpdates {
    pub(super) bonus: i32,
    pub(super) malus: i32,
    malus_decay: i32,
}

impl HistoryUpdates {
    /// 静かな最善手へ与える加点を返す（Stockfish src/search.cpp:2013-2015）。
    pub(super) fn quiet_bonus(self) -> i32 {
        scaled_history_value(self.bonus, 899)
    }

    /// 捕獲する最善手へ与える加点を返す（Stockfish src/search.cpp:2024-2029）。
    pub(super) fn capture_bonus(self) -> i32 {
        scaled_history_value(self.bonus, 1427)
    }

    /// 探索済みの捕獲手へ与える減点の絶対値を返す（Stockfish src/search.cpp:2037-2042）。
    pub(super) fn capture_malus(self) -> i32 {
        scaled_history_value(self.malus, 1489)
    }

    /// 探索済みの静かな手へ与える減点の逓減前の値を返す。
    pub(super) fn quiet_malus(self) -> i32 {
        scaled_history_value(self.malus, 1159)
    }

    /// 探索済みの静かな手1つ分を逓減した値を返す（Stockfish src/search.cpp:2018-2022）。
    pub(super) fn decay_quiet_malus(self, malus: i32) -> i32 {
        scaled_history_value(malus, self.malus_decay)
    }
}

fn scaled_history_value(value: i32, numerator: i32) -> i32 {
    i32::try_from(i64::from(value) * i64::from(numerator) / 1024)
        .expect("scaled history value must fit i32")
}

/// 置換表の静かな打ち切り手へ与える加点を求める。
/// 積は`i64`で計算する（src/search.cpp:902-906）。
pub(super) fn tt_cutoff_history_bonus(depth: u32, coefficient: i32) -> i32 {
    i32::try_from(i64::from(depth) * i64::from(coefficient))
        .expect("TT cutoff history bonus must fit i32")
}

/// 前の最善手が反駁されたときのcontinuation historyの減点を求める。
/// 積は`i64`で計算し、整数除算は0方向へ丸める（src/search.cpp:2031-2035）。
pub(super) fn refutation_continuation_update(malus: i32) -> i32 {
    i32::try_from(-(i64::from(malus) * 713 / 1024))
        .expect("refutation continuation update must fit i32")
}

/// Fail lowで直前の静かな手へ与える加点の入力。
#[derive(Clone, Copy)]
pub(super) struct FailLowHistoryInput {
    pub(super) depth: u32,
    pub(super) best_score: i32,
    pub(super) static_eval: i32,
    pub(super) previous_static_eval: i32,
    pub(super) previous_stat_score: i32,
    pub(super) previous_move_count: usize,
    pub(super) pawn_value: i32,
}

/// Fail lowで直前の静かな手へ与える各表の更新量。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FailLowHistoryUpdates {
    pub(super) bonus_scale: i32,
    pub(super) scaled_bonus: i32,
    pub(super) continuation: i32,
    pub(super) butterfly: i32,
    pub(super) pawn: i32,
}

/// Fail lowで直前の静かな手へ与える各表の更新量を求める。
///
/// 評価値はStockfishの尺度へ換算し、積は`i64`で計算する。
/// 整数除算は0方向へ丸める（src/search.cpp:1611-1633）。
pub(super) fn fail_low_history_updates(
    input: FailLowHistoryInput,
    history_prior_bonus: i32,
) -> FailLowHistoryUpdates {
    let value = stockfish_value(input.best_score, input.pawn_value);
    let static_eval = stockfish_value(input.static_eval, input.pawn_value);
    let previous_static_eval = stockfish_value(input.previous_static_eval, input.pawn_value);
    let depth = i64::from(input.depth);
    let bonus_scale = (-241_i64 - i64::from(input.previous_stat_score / 98)
        + (59 * depth).min(420)
        + 186 * i64::from(input.previous_move_count > 9)
        + 142 * i64::from(value <= static_eval - 106)
        + 159 * i64::from(value <= -previous_static_eval - 68))
    .max(0);
    let scaled_bonus = (150 * depth - 85).min(1337) * bonus_scale;
    let update = |numerator, denominator| {
        i32::try_from(scaled_bonus * i64::from(numerator) / denominator)
            .expect("fail-low history update must fit i32")
    };
    FailLowHistoryUpdates {
        bonus_scale: i32::try_from(bonus_scale).expect("fail-low bonus scale must fit i32"),
        scaled_bonus: i32::try_from(scaled_bonus).expect("fail-low scaled bonus must fit i32"),
        continuation: update(263, 16_384),
        butterfly: update(history_prior_bonus, 32_768),
        pawn: update(324, 8192),
    }
}

/// 静的評価の変化によるhistoryの更新量。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct EvalDifferenceHistoryUpdates {
    pub(super) delta: i32,
    pub(super) butterfly: i32,
    pub(super) pawn: i32,
}

/// 直前の手の前後の静的評価からhistoryの更新量を求める。
///
/// 評価値はStockfishの尺度へ換算し、積は`i64`で計算する
/// （src/search.cpp:1004-1011）。
pub(super) fn eval_difference_history_updates(
    previous_static_eval: i32,
    static_eval: i32,
    pawn_value: i32,
    history_eval_diff: i32,
) -> EvalDifferenceHistoryUpdates {
    let previous = stockfish_value(previous_static_eval, pawn_value);
    let current = stockfish_value(static_eval, pawn_value);
    let delta = (-(previous + current)).clamp(-189, 194) + 60;
    EvalDifferenceHistoryUpdates {
        delta: i32::try_from(delta).expect("static evaluation difference must fit i32"),
        butterfly: i32::try_from(delta * i64::from(history_eval_diff))
            .expect("evaluation-difference butterfly update must fit i32"),
        pawn: i32::try_from(delta * 13).expect("evaluation-difference pawn update must fit i32"),
    }
}

fn stockfish_value(value: i32, pawn_value: i32) -> i64 {
    i64::from(value) * 208 / i64::from(pawn_value)
}

fn records_pawn_history(record: MoveRecord) -> bool {
    !record.mv.promote
        && !matches!(
            record.piece.kind(),
            Some(PieceKind::Pawn | PieceKind::GoBetween)
        )
}

/// 静かな手のbutterfly historyと1手前・2手前のcontinuation historyからstatScoreを求める。
/// 積は`i64`で計算し、整数除算は0方向へ丸める（src/search.cpp:1375-1378）。
pub(super) fn quiet_stat_score(
    butterfly: i32,
    continuation_one: i32,
    continuation_two: i32,
) -> i32 {
    let score = 2252_i64 * i64::from(butterfly)
        + 1126_i64 * i64::from(continuation_one)
        + 1093_i64 * i64::from(continuation_two);
    i32::try_from(score / 1024).expect("quiet statScore must fit i32")
}

/// 捕獲価値をStockfishの尺度へ1回だけ換算し、捕獲履歴を加えてstatScoreを求める。
/// 積は`i64`で計算し、整数除算は0方向へ丸める（src/search.cpp:1371-1373）。
pub(super) fn capture_stat_score(captured_value: i32, pawn: i32, capture_history: i32) -> i32 {
    let stockfish_value = i64::from(captured_value) * 208 / i64::from(pawn);
    let score = 873 * stockfish_value / 128 + i64::from(capture_history);
    i32::try_from(score).expect("capture statScore must fit i32")
}

/// Continuation historyの6次元の添字を平坦な配列の添字へ写す。
/// 両方の到達升は現在の手の所有者が後手なら180度回転する。
pub(super) fn continuation_index(
    context: MoveRecord,
    current_piece: PieceCode,
    current_move: Move,
    distance: usize,
) -> usize {
    let color = current_piece.color().expect("continuation mover color");
    let context_to = oriented_square(context.mv.to.dense_index(), color);
    let current_to = oriented_square(current_move.to.dense_index(), color);
    let context_kind = context
        .piece
        .kind()
        .expect("continuation context piece kind")
        .index();
    let current_kind = current_piece
        .kind()
        .expect("continuation mover piece kind")
        .index();
    (((((usize::from(context.captured.is_some()) * COLOR_COUNT
        + usize::from(distance.is_multiple_of(2)))
        * PIECE_KIND_COUNT
        + context_kind)
        * BOARD_SQUARE_COUNT
        + context_to)
        * PIECE_KIND_COUNT
        + current_kind)
        * BOARD_SQUARE_COUNT)
        + current_to
}

fn oriented_square(square: usize, color: Color) -> usize {
    if color == Color::White {
        BOARD_SQUARE_COUNT - 1 - square
    } else {
        square
    }
}

fn continuation_value(
    continuation_history: &ContinuationHistory,
    path_moves: &[PathMove],
    ply: u32,
    piece: PieceCode,
    mv: Move,
    distance: usize,
) -> Option<i32> {
    let context_ply = ply.checked_sub(distance as u32)? as usize;
    let path = path_moves
        .get(context_ply)
        .expect("continuation context ply must exist in the path");
    path.record
        .map(|context| continuation_history.read(continuation_index(context, piece, mv, distance)))
}

impl Searcher<'_> {
    /// 捕獲手、killer手、history値の順で着手を整列し、置換表の手を先頭へ置く。
    pub(super) fn order_moves(
        &self,
        position: &Position,
        moves: &mut [Move],
        tt_move: Option<Move>,
        ply: u32,
    ) {
        let killers = self.killers[ply as usize];
        let color = position.side_to_move().index();
        moves.sort_by_cached_key(|&mv| {
            if let Some(key) = move_order_key(position, self.nnue, mv) {
                OrderedMoveKey::Capture {
                    captured_value: Reverse(key.captured_value),
                    attacker_value: key.attacker_value,
                }
            } else if Some(mv) == killers[0] {
                OrderedMoveKey::Killer(0)
            } else if Some(mv) == killers[1] {
                OrderedMoveKey::Killer(1)
            } else {
                OrderedMoveKey::Quiet(Reverse(
                    self.history[color][mv.from.dense_index()][mv.to.dense_index()],
                ))
            }
        });
        if let Some(index) = tt_move.and_then(|tt_move| moves.iter().position(|&mv| mv == tt_move))
        {
            moves.swap(0, index);
        }
    }

    /// βカットを起こした静かな手をkiller表へ記録する。
    pub(super) fn record_killer(&mut self, mv: Move, ply: u32) {
        let killers = &mut self.killers[ply as usize];
        if killers[0] != Some(mv) {
            killers[1] = killers[0];
            killers[0] = Some(mv);
        }
    }

    /// 指定plyの手を、指定した距離の文脈で引いたcontinuation historyの値を返す。
    /// 根より前またはnull moveの文脈は`None`とする。
    pub(super) fn continuation_value(
        &self,
        ply: u32,
        piece: PieceCode,
        mv: Move,
        distance: usize,
    ) -> Option<i32> {
        continuation_value(
            self.continuation_history,
            &self.path_moves,
            ply,
            piece,
            mv,
            distance,
        )
    }

    /// 指定plyで指す手の5文脈へcontinuation historyの更新量を与える。
    ///
    /// 更新前の正値の数に応じて倍率を選び、1手前には73を加える。
    /// 文脈を持たない距離は参照も更新もしない（src/search.cpp:2049-2073）。
    pub(super) fn update_continuation(&self, ply: u32, piece: PieceCode, mv: Move, bonus: i32) {
        let mut positive_count = 0;
        for (distance, weight) in CONTINUATION_WEIGHTS {
            let Some(context_ply) = ply.checked_sub(distance as u32) else {
                continue;
            };
            let path = self
                .path_moves
                .get(context_ply as usize)
                .expect("continuation context ply must exist in the path");
            let Some(context) = path.record else {
                continue;
            };
            let index = continuation_index(context, piece, mv, distance);
            if self.continuation_history.read(index) > 0 {
                positive_count += 1;
            }
            let scaled = i64::from(bonus)
                * i64::from(weight)
                * i64::from(CONTINUATION_MULTIPLIERS[positive_count])
                / 65_536;
            let update = i32::try_from(scaled).expect("continuation update must fit i32")
                + 73 * i32::from(distance == 1);
            self.continuation_history.update(index, update);
        }
    }

    /// Butterfly、low ply、continuation、およびpawn historyへ静かな手の更新量を与える。
    /// 各倍率はStockfishの値を使う（src/search.cpp:2077-2089）。
    pub(super) fn update_quiet_histories(
        &mut self,
        ply: u32,
        piece: PieceCode,
        mv: Move,
        bonus: i32,
    ) {
        let color = piece.color().expect("quiet history mover color").index();
        let from = mv.from.dense_index();
        let to = mv.to.dense_index();
        let butterfly = &mut self.history[color][from][to];
        *butterfly = history_gravity(*butterfly, bonus, HISTORY_LIMIT);

        if (ply as usize) < LOW_PLY_HISTORY_PLY_COUNT {
            let low_ply = &mut self.low_ply_history[ply as usize][from][to];
            let update = scaled_history_value(bonus, 712);
            *low_ply = history_gravity(i32::from(*low_ply), update, LOW_PLY_HISTORY_LIMIT) as i16;
        }

        self.update_continuation(ply, piece, mv, scaled_history_value(bonus, 750));

        let pawn_update = scaled_history_value(bonus, if bonus > -4 { 1104 } else { 459 });
        self.pawn_history
            .update(self.pawn_keys[ply as usize], piece, mv.to, pawn_update);
    }

    /// 置換表の値による打ち切りのhistoryを更新する。
    ///
    /// 置換表の手が合法な静かな手ならその手へ加点する。直前の手への減点は、
    /// 置換表の手の合法性と捕獲の種別によらない（src/search.cpp:902-910）。
    pub(super) fn update_tt_cutoff_histories(
        &mut self,
        position: &Position,
        ply: u32,
        requested_depth: u32,
        tt_move: Move,
        tt_capture: Option<bool>,
    ) {
        if tt_capture == Some(false) {
            self.update_quiet_histories(
                ply,
                piece_at_for_ordering(position, tt_move.from),
                tt_move,
                tt_cutoff_history_bonus(requested_depth, params::history_tt_cutoff_bonus()),
            );
        }

        let Some(previous_ply) = ply.checked_sub(1) else {
            return;
        };
        let Some(previous) = self.previous_move_record(ply) else {
            return;
        };
        if previous.captured.is_none() && self.move_counts[previous_ply as usize] < 5 {
            self.update_continuation(previous_ply, previous.piece, previous.mv, -2210);
        }
    }

    /// 直前の静かな手の前後の静的評価からhistoryを更新する。
    /// Stockfishの王手の条件は、minaseがすべてのノードで静的評価を求めるため置かない。
    pub(super) fn update_eval_difference_histories(
        &mut self,
        ply: u32,
        static_eval: i32,
        tt_hit: bool,
    ) {
        let Some(previous_ply) = ply.checked_sub(1) else {
            return;
        };
        let Some(previous) = self.previous_move_record(ply) else {
            return;
        };
        if previous.captured.is_some() {
            return;
        }
        let updates = eval_difference_history_updates(
            self.static_evals[previous_ply as usize],
            static_eval,
            self.nnue.pawn_value(),
            params::history_eval_diff(),
        );
        self.update_butterfly(previous, updates.butterfly);
        if !tt_hit && records_pawn_history(previous) {
            self.pawn_history.update(
                self.pawn_keys[ply as usize],
                previous.piece,
                previous.mv.to,
                updates.pawn,
            );
        }
    }

    /// 最善値がαを上回ったノードで、反駁された直前の静かな手を減点する。
    pub(super) fn update_refutation_history(&self, ply: u32, malus: i32) {
        let Some(previous_ply) = ply.checked_sub(1) else {
            return;
        };
        let Some(previous) = self.previous_move_record(ply) else {
            return;
        };
        if previous.captured.is_none()
            && self.move_counts[previous_ply as usize]
                == 1 + usize::from(self.tt_hits[previous_ply as usize])
        {
            self.update_continuation(
                previous_ply,
                previous.piece,
                previous.mv,
                refutation_continuation_update(malus),
            );
        }
    }

    /// Fail lowのノードで、このノードへ至った手のhistoryを更新する。
    pub(super) fn update_fail_low_histories(
        &mut self,
        ply: u32,
        depth: u32,
        best_score: i32,
        static_eval: i32,
    ) {
        let Some(previous_ply) = ply.checked_sub(1) else {
            return;
        };
        let Some(previous) = self.previous_move_record(ply) else {
            return;
        };
        if previous.captured.is_some() {
            self.capture_history.update_record(previous, 892);
            return;
        }

        let updates = fail_low_history_updates(
            FailLowHistoryInput {
                depth,
                best_score,
                static_eval,
                previous_static_eval: self.static_evals[previous_ply as usize],
                previous_stat_score: self.stat_scores[previous_ply as usize],
                previous_move_count: self.move_counts[previous_ply as usize],
                pawn_value: self.nnue.pawn_value(),
            },
            params::history_prior_bonus(),
        );
        self.update_continuation(
            previous_ply,
            previous.piece,
            previous.mv,
            updates.continuation,
        );
        self.update_butterfly(previous, updates.butterfly);
        if records_pawn_history(previous) {
            self.pawn_history.update(
                self.pawn_keys[ply as usize],
                previous.piece,
                previous.mv.to,
                updates.pawn,
            );
        }
    }

    fn previous_move_record(&self, ply: u32) -> Option<MoveRecord> {
        let previous = ply.checked_sub(1)? as usize;
        self.path_moves.get(previous)?.record
    }

    fn update_butterfly(&mut self, record: MoveRecord, bonus: i32) {
        let color = record
            .piece
            .color()
            .expect("butterfly history mover color")
            .index();
        let value =
            &mut self.history[color][record.mv.from.dense_index()][record.mv.to.dense_index()];
        *value = history_gravity(*value, bonus, HISTORY_LIMIT);
    }

    /// 実際に読む手のstatScoreを、着手前の局面と履歴表から求める。
    pub(super) fn move_stat_score(
        &self,
        position: &Position,
        mv: Move,
        capture: bool,
        ply: u32,
    ) -> i32 {
        if capture {
            capture_stat_score(
                captured_value(position, self.nnue, mv),
                self.nnue.pawn_value(),
                self.capture_history.read(position, self.nnue, mv),
            )
        } else {
            let piece = piece_at_for_ordering(position, mv.from);
            quiet_stat_score(
                self.history[piece.color().expect("quiet statScore mover color").index()]
                    [mv.from.dense_index()][mv.to.dense_index()],
                self.continuation_value(ply, piece, mv, 1).unwrap_or(0),
                self.continuation_value(ply, piece, mv, 2).unwrap_or(0),
            )
        }
    }

    /// αを上回った最善手へ加点し、探索済みの手へ減点する。
    ///
    /// Butterfly historyの上限は7183とし、捕獲履歴の上限は10692とする。
    /// 静かな手の減点は読んだ順に`HistoryMalusDecay / 1024`倍してから与える。
    /// 両表とも更新量を表の上限へ切り詰めた重力式を使う。
    /// 方式の理由は`docs/adr/order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md`にある。
    pub(super) fn update_best_move_histories(
        &mut self,
        position: &Position,
        best: Move,
        best_capture: bool,
        searched_quiets: &[Move],
        searched_captures: &[Move],
        updates: HistoryUpdates,
    ) {
        let ply = u32::try_from(self.path_moves.len()).expect("search ply must fit u32");
        if best_capture {
            self.capture_history
                .update(position, self.nnue, best, updates.capture_bonus());
        } else {
            self.update_quiet_histories(
                ply,
                piece_at_for_ordering(position, best.from),
                best,
                updates.quiet_bonus(),
            );
            let mut malus = updates.quiet_malus();
            for &mv in searched_quiets {
                malus = updates.decay_quiet_malus(malus);
                self.update_quiet_histories(
                    ply,
                    piece_at_for_ordering(position, mv.from),
                    mv,
                    -malus,
                );
            }
        }

        let capture_malus = updates.capture_malus();
        for &mv in searched_captures {
            self.capture_history
                .update(position, self.nnue, mv, -capture_malus);
        }
    }
}

/// 捕獲手と整列キーのペアをMVV-LVA順で安定に整列する参照実装。
#[cfg(test)]
pub(super) fn order_captures(captures: &mut [(Move, MoveOrderKey)]) {
    captures.sort_by_key(|&(_, key)| (Reverse(key.captured_value), key.attacker_value));
}

/// 通常探索で使う着手の整列キー。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OrderedMoveKey {
    /// 捕獲価値の降順、攻撃駒価値の昇順で並べる捕獲手。
    Capture {
        /// 根の[`Searcher::order_moves`]で使う、取る駒の価値の合計の降順キー。
        /// 捕獲履歴は加えない。内部ノードでの加算は[`MoveOrderKey::captured_value`]に従う。
        captured_value: Reverse<i32>,
        /// 動かす駒の駒価値。
        attacker_value: i32,
    },
    /// 添字が小さいほど新しいkiller手。
    Killer(usize),
    /// History値の降順で並べる残りの非捕獲手。
    Quiet(Reverse<i32>),
}

/// 通常探索で手を返す段階。
#[derive(Clone, Copy)]
pub(super) enum MovePickerStage {
    Tt,
    Captures,
    EarlyKiller0,
    EarlyKiller1,
    Killer0,
    Killer1,
    Quiets,
    Done,
}

/// 手選択器が静かな手の並べ替えと捕獲履歴の換算に使う探索状態。
#[derive(Clone, Copy)]
pub(super) struct MovePickerContext<'a> {
    pub(super) history: &'a HistoryTable,
    pub(super) capture_history: &'a CaptureHistory,
    pub(super) continuation_history: &'a ContinuationHistory,
    pub(super) pawn_history: &'a PawnHistory,
    pub(super) low_ply_history: &'a LowPlyHistory,
    pub(super) path_moves: &'a [PathMove],
    pub(super) pawn_key: u64,
    pub(super) ply: u32,
    pub(super) coefficients: QuietOrderingCoefficients,
}

fn quiet_move_ordering_value(
    position: &Position,
    nnue: &Nnue,
    generator: &MoveGenerator,
    context: MovePickerContext<'_>,
    lesser_piece_threats: &LesserPieceThreats,
    direct_checks: &mut DirectCheckSquares,
    mv: Move,
) -> i64 {
    let piece = piece_at_for_ordering(position, mv.from);
    let piece_value = nnue.piece_value(piece);
    let moving_kind = piece.kind().expect("quiet ordering mover kind");
    let continuation = CONTINUATION_WEIGHTS
        .iter()
        .filter_map(|&(distance, _)| {
            continuation_value(
                context.continuation_history,
                context.path_moves,
                context.ply,
                piece,
                mv,
                distance,
            )
        })
        .sum();
    let threats = lesser_piece_threats.below(piece_value);
    quiet_ordering_value(
        QuietOrderingInput {
            butterfly: context.history[piece.color().expect("quiet ordering mover color").index()]
                [mv.from.dense_index()][mv.to.dense_index()],
            pawn: context.pawn_history.read(context.pawn_key, piece, mv.to),
            continuation,
            ply: context.ply,
            from: mv.from,
            to: mv.to,
            check: qualifies_for_check_bonus(
                position,
                generator,
                nnue,
                direct_checks,
                moving_kind,
                mv,
            ),
            moving_piece_value: piece_value,
            pawn_value: nnue.pawn_value(),
            threat_delta: lesser_threat_delta(moving_kind, threats, mv),
        },
        context.low_ply_history,
        context.coefficients,
    )
}

/// 置換表の手、捕獲手、killer手、残りの静かな手の順に合法手を1回ずつ返す。
///
/// [`Self::reset`]で指定したノードでは、整列後の先頭捕獲手だけを静的交換評価（SEE）で判定する。
/// 最後の王駒を取る手は判定せず、損と判定した場合は合法なkiller手を捕獲列より先に返す。
/// 追加のSEEは1ノードに最大1回で、元の捕獲列もその後に返す。
/// 方式の理由は`docs/adr/try-killers-before-a-losing-first-capture.md`にある。
pub(super) struct MovePicker {
    stage: MovePickerStage,
    tt_move: Option<Move>,
    killers: [Option<Move>; KILLER_COUNT],
    killers_before_losing_capture: bool,
    captures: Vec<(Move, MoveOrderKey)>,
    capture_index: usize,
    captures_generated: bool,
    quiets: Vec<Move>,
    quiet_index: usize,
    base_moves: Vec<Move>,
    /// 静かな手の並べ替え値の降順、線形経路の移動差分の降順、生成順の整列キー。
    /// 差分は[`Nnue::quiet_move_pst_delta`]で求め、成る手は到達升を成り後の駒で引く。
    /// 同点破りの理由は`docs/adr/order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md`にある。
    quiet_order: Vec<(Reverse<i64>, Reverse<i64>, usize)>,
    used_quiets: [Option<usize>; KILLER_COUNT],
    quiets_generated: bool,
}

impl MovePicker {
    /// 助言手を保持した空の手選択器を作る。
    pub(super) fn new(tt_move: Option<Move>, killers: [Option<Move>; KILLER_COUNT]) -> Self {
        Self {
            stage: MovePickerStage::Tt,
            tt_move,
            killers,
            killers_before_losing_capture: false,
            captures: Vec::new(),
            capture_index: 0,
            captures_generated: false,
            quiets: Vec::new(),
            quiet_index: 0,
            base_moves: Vec::new(),
            quiet_order: Vec::new(),
            used_quiets: [None; KILLER_COUNT],
            quiets_generated: false,
        }
    }

    /// 確保した領域を保って候補と整列状態を消し、次のノードへ進む。
    /// `killers_before_losing_capture`は、置換表の記録手がなく、零窓で、残り深さ3以下の場合だけ真にする。
    pub(super) fn reset(
        &mut self,
        tt_move: Option<Move>,
        killers: [Option<Move>; KILLER_COUNT],
        killers_before_losing_capture: bool,
    ) {
        self.stage = MovePickerStage::Tt;
        self.tt_move = tt_move;
        self.killers = killers;
        self.killers_before_losing_capture = killers_before_losing_capture;
        self.captures.clear();
        self.capture_index = 0;
        self.captures_generated = false;
        self.quiets.clear();
        self.quiet_index = 0;
        self.quiets_generated = false;
        self.quiet_order.clear();
        self.used_quiets = [None; KILLER_COUNT];
    }

    /// 現在の段階で次に探索する合法手と、捕獲手かどうかの組を返す。
    pub(super) fn next(
        &mut self,
        position: &Position,
        nnue: &Nnue,
        generator: &MoveGenerator,
        context: MovePickerContext<'_>,
    ) -> Option<(Move, bool)> {
        loop {
            match self.stage {
                MovePickerStage::Tt => {
                    self.stage = MovePickerStage::Captures;
                    if let Some(tt_move) = self.tt_move
                        && generator.is_legal_move(
                            position,
                            tt_move,
                            &mut self.base_moves,
                            &mut self.quiets,
                        )
                    {
                        return Some((tt_move, move_order_key(position, nnue, tt_move).is_some()));
                    }
                }
                MovePickerStage::Captures => {
                    if !self.captures_generated {
                        self.quiets.clear();
                        generator.generate_captures_with_scratch(
                            position,
                            &mut self.base_moves,
                            &mut self.quiets,
                        );
                        self.captures.extend(
                            self.quiets
                                .drain(..)
                                .filter(|&mv| Some(mv) != self.tt_move)
                                .map(|mv| {
                                    let mut key = move_order_key(position, nnue, mv)
                                        .expect("capture generator must not return a quiet move");
                                    key.captured_value +=
                                        context.capture_history.adjustment(position, nnue, mv);
                                    (mv, key)
                                }),
                        );
                        self.captures.sort_by_key(|&(_, key)| {
                            (Reverse(key.captured_value), key.attacker_value)
                        });
                        self.captures_generated = true;
                        // 生成時だけ判定し、killer段から戻ったときは再判定しない。
                        if self.killers_before_losing_capture
                            && let Some(&(mv, _)) = self.captures.first()
                            && !captures_last_royal(position, mv)
                            && see_prunes(position, generator.rules(), nnue, mv, 0)
                        {
                            self.stage = MovePickerStage::EarlyKiller0;
                            continue;
                        }
                    }
                    if let Some(&(mv, _)) = self.captures.get(self.capture_index) {
                        self.capture_index += 1;
                        return Some((mv, true));
                    }
                    self.stage = if self.quiets_generated {
                        MovePickerStage::Quiets
                    } else {
                        MovePickerStage::Killer0
                    };
                }
                MovePickerStage::EarlyKiller0
                | MovePickerStage::EarlyKiller1
                | MovePickerStage::Killer0
                | MovePickerStage::Killer1 => {
                    if !self.quiets_generated {
                        generator.generate_quiets(position, &mut self.base_moves, &mut self.quiets);
                        self.quiets.retain(|&mv| Some(mv) != self.tt_move);
                        self.quiets_generated = true;
                    }
                    let (killer_index, next_stage) = match self.stage {
                        MovePickerStage::EarlyKiller0 => (0, MovePickerStage::EarlyKiller1),
                        MovePickerStage::EarlyKiller1 => (1, MovePickerStage::Captures),
                        MovePickerStage::Killer0 => (0, MovePickerStage::Killer1),
                        MovePickerStage::Killer1 => (1, MovePickerStage::Quiets),
                        _ => unreachable!("only killer stages select a killer"),
                    };
                    self.stage = next_stage;
                    if let Some(killer) = self.killers[killer_index]
                        && let Some(index) =
                            self.quiets.iter().enumerate().find_map(|(index, &mv)| {
                                (mv == killer && !self.used_quiets.contains(&Some(index)))
                                    .then_some(index)
                            })
                    {
                        self.used_quiets[killer_index] = Some(index);
                        return Some((self.quiets[index], false));
                    }
                }
                // killer手を除き、quiet_orderの3つのキーで順序を確定する。
                MovePickerStage::Quiets => {
                    let threats = LesserPieceThreats::with_piece_values(position, |piece| {
                        nnue.piece_value(piece)
                    });
                    let mut direct_checks = DirectCheckSquares::new(position);
                    for (index, &mv) in self.quiets.iter().enumerate() {
                        if self.used_quiets.contains(&Some(index)) {
                            continue;
                        }
                        self.quiet_order.push((
                            Reverse(quiet_move_ordering_value(
                                position,
                                nnue,
                                generator,
                                context,
                                &threats,
                                &mut direct_checks,
                                mv,
                            )),
                            Reverse(nnue.quiet_move_pst_delta(position, mv)),
                            index,
                        ));
                    }
                    self.quiet_order.sort_unstable();
                    self.stage = MovePickerStage::Done;
                }
                MovePickerStage::Done => {
                    if let Some(&(_, _, index)) = self.quiet_order.get(self.quiet_index) {
                        self.quiet_index += 1;
                        return Some((self.quiets[index], false));
                    }
                    return None;
                }
            }
        }
    }
}

/// 捕獲手の整列キー。
#[derive(Clone, Copy)]
pub(super) struct MoveOrderKey {
    /// 取る駒の価値の合計を基礎とする整列値。
    /// [`move_order_key`]の返却時点と根の[`Searcher::order_moves`]では合計だけである。
    /// [`MovePicker`]の捕獲段では[`CaptureHistory::adjustment`]の換算値を加えてから整列する。
    pub(super) captured_value: i32,
    /// 動かす駒の駒価値。
    pub(super) attacker_value: i32,
}

/// 捕獲手なら駒価値だけの整列キーを返し、非捕獲手なら`None`を返す。
///
/// 捕獲価値の合計が正であることを判定に使うので、この同値は[`Nnue::piece_value`]の正値性に依存する。
/// [`MovePicker::next`]が捕獲手かどうかを分類するためにこの関数を使うのは置換表の記録手だけである。
/// 捕獲段では捕獲専用生成が分類を保証し、この関数は整列キーの取得に使う。
pub(super) fn move_order_key(position: &Position, nnue: &Nnue, mv: Move) -> Option<MoveOrderKey> {
    let captured_value = captured_value(position, nnue, mv);
    (captured_value > 0).then(|| MoveOrderKey {
        captured_value,
        attacker_value: nnue.piece_value(piece_at_for_ordering(position, mv.from)),
    })
}

/// 2枚取りを含む、手が取る駒の価値の合計を返す。
fn captured_value(position: &Position, nnue: &Nnue, mv: Move) -> i32 {
    position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| nnue.piece_value(piece_at_for_ordering(position, square)))
        .sum()
}

/// 指定升の駒コードを返す。
///
/// # Panics
///
/// 升に駒がない場合にパニックする。
pub(super) fn piece_at_for_ordering(position: &Position, square: minase_core::Square) -> PieceCode {
    position
        .piece_at(square)
        .expect("move ordering square must contain a piece")
}

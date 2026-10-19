//! 着手の順序付けと手の成績の更新。
//!
//! β打ち切りでは、historyの加点を`min(傾き × 深さ − 切片, 上限)`へ置換表の手と親のstatScoreの項を加えて求める。
//! 減点も同じ形の1次式で求め、αを上回らなかった探索済みの手へ与える。
//! Butterfly historyと捕獲履歴は、それぞれ上限7183と10692の重力式で更新する。

use core::cmp::Reverse;

use crate::eval::Nnue;
use minase_core::MoveGenerator;
use minase_core::mv::Move;
use minase_core::piece::PieceCode;
use minase_core::position::Position;

use super::capture_history::CaptureHistory;
use super::history::{HISTORY_LIMIT, HistoryTable, history_gravity};
use super::params;
use super::royal::captures_last_royal;
use super::searcher::{KILLER_COUNT, Searcher};
use super::see::see_prunes;

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

    /// βカットを起こした最善手へ加点し、探索済みの手へ減点する。
    ///
    /// Butterfly historyの上限は7183とし、捕獲履歴の上限は10692とする。
    /// 静かな手の減点は読んだ順に`HistoryMalusDecay / 1024`倍してから与える。
    /// 両表とも更新量を表の上限へ切り詰めた重力式を使う。
    /// 方式の理由は`docs/adr/order-quiets-with-malus-and-pst-tiebreak.md`にある。
    pub(super) fn update_beta_cutoff_histories(
        &mut self,
        position: &Position,
        best: Move,
        best_capture: bool,
        searched_quiets: &[Move],
        searched_captures: &[Move],
        updates: HistoryUpdates,
    ) {
        let color = position.side_to_move().index();
        let mut update_quiet = |mv: Move, bonus: i32| {
            let value = &mut self.history[color][mv.from.dense_index()][mv.to.dense_index()];
            *value = history_gravity(*value, bonus, HISTORY_LIMIT);
        };

        if best_capture {
            self.capture_history
                .update(position, self.nnue, best, updates.capture_bonus());
        } else {
            update_quiet(best, updates.quiet_bonus());
            let mut malus = updates.quiet_malus();
            for &mv in searched_quiets {
                malus = updates.decay_quiet_malus(malus);
                update_quiet(mv, -malus);
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
    /// butterfly historyの降順、線形経路の移動差分の降順、生成順の整列キー。
    /// 差分は[`Nnue::quiet_move_pst_delta`]で求め、成る手は到達升を成り後の駒で引く。
    /// 同点破りの理由は`docs/adr/order-quiets-with-malus-and-pst-tiebreak.md`にある。
    quiet_order: Vec<(Reverse<i32>, Reverse<i64>, usize)>,
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
        history: &HistoryTable,
        capture_history: &CaptureHistory,
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
                                        capture_history.adjustment(position, nnue, mv);
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
                    let color = position.side_to_move().index();
                    self.quiet_order.extend(
                        self.quiets
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| !self.used_quiets.contains(&Some(*index)))
                            .map(|(index, mv)| {
                                (
                                    Reverse(
                                        history[color][mv.from.dense_index()][mv.to.dense_index()],
                                    ),
                                    Reverse(nnue.quiet_move_pst_delta(position, *mv)),
                                    index,
                                )
                            }),
                    );
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
    let captured_value: i32 = position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| nnue.piece_value(piece_at_for_ordering(position, square)))
        .sum();
    (captured_value > 0).then(|| MoveOrderKey {
        captured_value,
        attacker_value: nnue.piece_value(piece_at_for_ordering(position, mv.from)),
    })
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

//! 捕獲手の順序付けに使うワーカー別の捕獲履歴（capture history）。
//!
//! βカットを起こした捕獲手へ加点し、打ち切り手の種別によらず探索済みの捕獲手を減点する。
//! 更新量を±10692へ切り詰めてから、butterfly historyと同じ重力式で更新する。
//! 値は常に±10692に収まり、通常探索では[`CaptureHistory::adjustment`]で駒価値へ換算する。
//! 静止探索では生の履歴値で同じ捕獲価値の群内を並べる。
//! 上限と適用範囲の理由は`docs/adr/update-capture-history-in-stockfish-form-and-use-it-in-ordering-qsearch-and-lmr.md`にある。

use crate::eval::Nnue;
use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::mv::Move;
use minase_core::piece::{COLOR_COUNT, PIECE_KIND_COUNT, PieceCode, PieceKind};
use minase_core::position::Position;

use super::history::history_gravity;
use super::ordering::piece_at_for_ordering;
use super::params;
use super::searcher::MoveRecord;

/// 捕獲履歴の絶対値の上限。
/// Stockfishの`CapturePieceToHistory`と同じ10692を使う（src/history.h:135）。
pub(super) const CAPTURE_HISTORY_LIMIT: i32 = 10692;

/// 新しい対局での捕獲履歴の初期値。
/// Stockfishの初期化と同じ−742を使う（src/search.cpp:715）。
const CAPTURE_HISTORY_INITIAL: i32 = -742;

/// 着手前の駒コード、到達升、被捕獲駒種で参照するワーカー専用の表。
pub(crate) struct CaptureHistory {
    values: Box<[i32]>,
}

impl CaptureHistory {
    /// ワーカーが探索間で持ち越す表をStockfishと同じ−742で初期化する。
    pub(super) fn new() -> Self {
        Self {
            values: vec![
                CAPTURE_HISTORY_INITIAL;
                COLOR_COUNT * PIECE_KIND_COUNT * BOARD_SQUARE_COUNT * PIECE_KIND_COUNT
            ]
            .into_boxed_slice(),
        }
    }

    /// 全要素を新しい対局と同じ初期値へ戻す。
    pub(super) fn clear(&mut self) {
        self.values.fill(CAPTURE_HISTORY_INITIAL);
    }

    /// 探索間に残す百分率で全要素を減衰させる。整数除算は0方向へ丸める。
    pub(super) fn decay(&mut self, percent: i32) {
        for value in &mut self.values {
            *value = *value * percent / 100;
        }
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> &[i32] {
        &self.values
    }

    #[cfg(test)]
    pub(crate) fn values_mut(&mut self) -> &mut [i32] {
        &mut self.values
    }

    /// 捕獲手の履歴値を返す。
    pub(super) fn read(&self, position: &Position, nnue: &Nnue, mv: Move) -> i32 {
        self.values[index(position, nnue, mv)]
    }

    /// 履歴値を駒価値へ換算し、捕獲価値に加える値を返す。
    ///
    /// `尺度 = 歩兵価値 × CaptureHistoryScale / 100` を先に整数で求め、`履歴値 × 尺度 / 10692` を返す。
    /// 整数除算は0方向へ丸め、換算値の絶対値は尺度以下に保つ。
    /// 捕獲価値の差が大きい手の順序を履歴だけで覆さないための制限である。
    pub(super) fn adjustment(&self, position: &Position, nnue: &Nnue, mv: Move) -> i32 {
        let scale = i64::from(nnue.pawn_value()) * i64::from(params::capture_history_scale()) / 100;
        (i64::from(self.read(position, nnue, mv)) * scale / i64::from(CAPTURE_HISTORY_LIMIT)) as i32
    }

    /// 指定した捕獲手の要素へ、上限10692の重力式で符号つきの更新量を与える。
    pub(super) fn update(&mut self, position: &Position, nnue: &Nnue, mv: Move, bonus: i32) {
        let value = &mut self.values[index(position, nnue, mv)];
        *value = history_gravity(*value, bonus, CAPTURE_HISTORY_LIMIT);
    }

    /// 探索経路に記録した捕獲手の要素へ更新量を与える。
    pub(super) fn update_record(&mut self, record: MoveRecord, bonus: i32) {
        let value = &mut self.values[index_from_record(record)];
        *value = history_gravity(*value, bonus, CAPTURE_HISTORY_LIMIT);
    }

    /// 探索経路に記録した捕獲手の履歴値を返す。
    #[cfg(test)]
    pub(super) fn read_record(&self, record: MoveRecord) -> i32 {
        self.values[index_from_record(record)]
    }
}

/// 着手前の所有者と駒種、到達升、被捕獲駒種の順で添字を構成する。
/// 2枚取りでは価値の高い被捕獲駒を選び、同値なら捕獲升の列の後方、すなわち到達升を選ぶ。
fn index(position: &Position, nnue: &Nnue, mv: Move) -> usize {
    let mover = piece_at_for_ordering(position, mv.from);
    let victim = captured_kind_for_history(
        nnue,
        position
            .captured_squares(mv)
            .into_iter()
            .flatten()
            .map(|square| piece_at_for_ordering(position, square)),
    )
    .expect("capture history requires a capture");
    index_from_parts(mover, mv.to.dense_index(), victim)
}

/// 探索経路の記録から捕獲履歴の添字を構成する。
fn index_from_record(record: MoveRecord) -> usize {
    index_from_parts(
        record.piece,
        record.mv.to.dense_index(),
        record
            .captured
            .expect("capture history record requires a captured piece"),
    )
}

fn index_from_parts(mover: PieceCode, to: usize, victim: PieceKind) -> usize {
    let code = mover.color().expect("mover color").index() * PIECE_KIND_COUNT
        + mover.kind().expect("mover kind").index();
    (code * BOARD_SQUARE_COUNT + to) * PIECE_KIND_COUNT + victim.index()
}

/// 捕獲履歴の添字に使う被捕獲駒種を返す。
/// 2枚取りでは価値の高い駒を選び、同値なら列の後方にある到達升の駒を選ぶ。
pub(super) fn captured_kind_for_history(
    nnue: &Nnue,
    pieces: impl IntoIterator<Item = PieceCode>,
) -> Option<PieceKind> {
    pieces
        .into_iter()
        // `Iterator::max_by_key`は最大値が複数あれば最後の要素を返す。
        .max_by_key(|&piece| nnue.piece_value(piece))
        .and_then(PieceCode::kind)
}

//! 捕獲手の順序付けに使うワーカー別の捕獲履歴（capture history）。
//!
//! βカットを起こした捕獲手へ加点し、打ち切り手の種別によらず先に探索済みの捕獲手を減点する。
//! 加減点の絶対値は `min(深さ², CaptureHistoryLimit)` とし、重力式 `v += b − v × |b| / CaptureHistoryLimit` で更新する。
//! 値は常に±`CaptureHistoryLimit`に収まり、通常探索では[`CaptureHistory::adjustment`]で駒価値へ換算する。
//! 静止探索では生の履歴値で同じ捕獲価値の群内を並べる。
//! 上限と適用範囲の理由は`docs/adr/bound-capture-history-and-use-it-in-qsearch.md`にある。

use crate::eval::Nnue;
use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::mv::Move;
use minase_core::piece::{COLOR_COUNT, PIECE_KIND_COUNT};
use minase_core::position::Position;

use super::ordering::piece_at_for_ordering;
use super::params;

/// 着手前の駒コード、到達升、被捕獲駒種で参照するワーカー専用の表。
pub(crate) struct CaptureHistory {
    values: Box<[i32]>,
}

impl CaptureHistory {
    /// ワーカーが探索間で持ち越す表を0で初期化する。
    pub(super) fn new() -> Self {
        Self {
            values: vec![0; COLOR_COUNT * PIECE_KIND_COUNT * BOARD_SQUARE_COUNT * PIECE_KIND_COUNT]
                .into_boxed_slice(),
        }
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
    /// `尺度 = 歩兵価値 × CaptureHistoryScale / 100` を先に整数で求め、`履歴値 × 尺度 / CaptureHistoryLimit` を返す。
    /// 整数除算は0方向へ丸め、換算値の絶対値は尺度以下に保つ。
    /// 捕獲価値の差が大きい手の順序を履歴だけで覆さないための制限である。
    pub(super) fn adjustment(&self, position: &Position, nnue: &Nnue, mv: Move) -> i32 {
        let scale = i64::from(nnue.pawn_value()) * i64::from(params::capture_history_scale()) / 100;
        (i64::from(self.read(position, nnue, mv)) * scale
            / i64::from(params::capture_history_limit())) as i32
    }

    /// 打ち切り手に加点し、同じノードで探索済みの捕獲手に減点する。
    pub(super) fn record_cutoff(
        &mut self,
        position: &Position,
        nnue: &Nnue,
        mv: Move,
        earlier: &[Move],
        depth: u32,
    ) {
        let limit = params::capture_history_limit();
        let bonus = (depth * depth).min(limit as u32) as i32;
        for (mv, bonus) in
            core::iter::once((mv, bonus)).chain(earlier.iter().map(|&mv| (mv, -bonus)))
        {
            update(&mut self.values[index(position, nnue, mv)], bonus, limit);
        }
    }

    /// 静かな手による打ち切り時に、同じノードで探索済みの捕獲手に減点する。
    pub(super) fn record_quiet_cutoff(
        &mut self,
        position: &Position,
        nnue: &Nnue,
        earlier: &[Move],
        depth: u32,
    ) {
        let limit = params::capture_history_limit();
        let bonus = (depth * depth).min(limit as u32) as i32;
        for &mv in earlier {
            update(&mut self.values[index(position, nnue, mv)], -bonus, limit);
        }
    }
}

/// 着手前の所有者と駒種、到達升、被捕獲駒種の順で添字を構成する。
/// 2枚取りでは価値の高い被捕獲駒を選び、同値なら捕獲升の列の後方、すなわち到達升を選ぶ。
fn index(position: &Position, nnue: &Nnue, mv: Move) -> usize {
    let mover = piece_at_for_ordering(position, mv.from);
    let victim = position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| piece_at_for_ordering(position, square))
        // 同値なら後の捕獲升を選ぶ。2枚取りでは経由升より到達升を優先する。
        .max_by_key(|&piece| nnue.piece_value(piece))
        .expect("capture history requires a capture");
    let code = mover.color().expect("mover color").index() * PIECE_KIND_COUNT
        + mover.kind().expect("mover kind").index();
    (code * BOARD_SQUARE_COUNT + mv.to.dense_index()) * PIECE_KIND_COUNT
        + victim.kind().expect("victim kind").index()
}

fn update(value: &mut i32, bonus: i32, limit: i32) {
    let old = i64::from(*value);
    let bonus = i64::from(bonus);
    *value = (old + bonus - old * bonus.abs() / i64::from(limit)) as i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 重力式は上限の許容範囲の両端でも履歴値を±上限以内に保つ。
    /// 加減点が上限に等しい場合は、その符号側の上限へ達する。
    #[test]
    fn capture_history_gravity_stays_bounded_at_limit_extremes() {
        for limit in [4_096, 65_536] {
            for old in -limit..=limit {
                for bonus in [-limit, -limit + 1, -1, 0, 1, limit - 1, limit] {
                    let mut value = old;
                    update(&mut value, bonus, limit);
                    assert!((-limit..=limit).contains(&value));
                    if bonus == limit {
                        assert_eq!(value, limit);
                    } else if bonus == -limit {
                        assert_eq!(value, -limit);
                    }
                }
            }
        }
        let mut value = -10;
        update(&mut value, 3, 100);
        assert_eq!(value, -7, "負の積も0方向へ丸める");
    }
}

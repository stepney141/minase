//! docs/plans/search-revival-spsa.md「戻す8項目」の捕獲履歴。
//! 更新と添字はstrength-stage12.md「項目3」に従い、上限65,536をi32で保持する。

use crate::eval::Pst;
use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::mv::Move;
use minase_core::piece::{COLOR_COUNT, PIECE_KIND_COUNT};
use minase_core::position::Position;

use super::ordering::piece_at_for_ordering;
use super::params;

/// 着手前の駒コード、到達升、被捕獲駒種で参照するワーカー専用の表。
pub(super) struct CaptureHistory {
    values: Box<[i32]>,
}

impl CaptureHistory {
    /// 1回の探索で使う表を0で初期化する。
    pub(super) fn new() -> Self {
        Self {
            values: vec![0; COLOR_COUNT * PIECE_KIND_COUNT * BOARD_SQUARE_COUNT * PIECE_KIND_COUNT]
                .into_boxed_slice(),
        }
    }

    /// 捕獲手の履歴値を返す。
    pub(super) fn read(&self, position: &Position, pst: &Pst, mv: Move) -> i32 {
        self.values[index(position, pst, mv)]
    }

    /// 履歴値を歩兵価値で換算する。整数除算は0方向へ丸める。
    pub(super) fn adjustment(&self, position: &Position, pst: &Pst, mv: Move) -> i32 {
        let scale = i64::from(pst.pawn_value()) * i64::from(params::capture_history_scale()) / 100;
        (i64::from(self.read(position, pst, mv)) * scale
            / i64::from(params::capture_history_limit())) as i32
    }

    /// 打ち切り手に加点し、同じノードで探索済みの捕獲手に減点する。
    pub(super) fn record_cutoff(
        &mut self,
        position: &Position,
        pst: &Pst,
        mv: Move,
        earlier: &[Move],
        depth: u32,
    ) {
        let limit = params::capture_history_limit();
        let bonus = (depth * depth).min(limit as u32) as i32;
        for (mv, bonus) in
            core::iter::once((mv, bonus)).chain(earlier.iter().map(|&mv| (mv, -bonus)))
        {
            update(&mut self.values[index(position, pst, mv)], bonus, limit);
        }
    }
}

fn index(position: &Position, pst: &Pst, mv: Move) -> usize {
    let mover = piece_at_for_ordering(position, mv.from);
    let victim = position
        .captured_squares(mv)
        .into_iter()
        .flatten()
        .map(|square| piece_at_for_ordering(position, square))
        // 同値なら後の捕獲升を選ぶ。2枚取りでは経由升より到達升を優先する。
        .max_by_key(|&piece| pst.piece_value(piece))
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

    /// 設計書「検証」: Dの両端で全履歴値と境界の加減点を検査する。
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

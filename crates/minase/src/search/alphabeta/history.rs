//! 対局内で持ち越すワーカー別のbutterfly history。

use core::num::NonZeroUsize;

use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::piece::COLOR_COUNT;

/// 手番側・移動元・移動先で参照するhistory表。
pub(crate) type HistoryTable = [[[i32; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT];

/// 各ワーカーが次の根の探索へ持ち越す表。
/// `docs/plans/strength-stage12.md`「項目5」「並列探索」に従う。
pub struct HistoryTables {
    pub(crate) workers: Vec<Box<HistoryTable>>,
}

impl HistoryTables {
    /// 指定したワーカー数の表を0で初期化する。
    pub fn new(threads: NonZeroUsize) -> Self {
        Self {
            workers: (0..threads.get())
                .map(|_| Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]))
                .collect(),
        }
    }

    /// ワーカー数を保ったまま全ワーカーの表を0にする。
    pub fn clear(&mut self) {
        for history in &mut self.workers {
            for value in history.iter_mut().flatten().flatten() {
                *value = 0;
            }
        }
    }
}

//! 対局内で持ち越すワーカー別のbutterfly historyと捕獲履歴。

use core::num::NonZeroUsize;

use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::piece::COLOR_COUNT;

use super::capture_history::CaptureHistory;

/// 手番側・移動元・移動先で参照するhistory表。
pub(crate) type HistoryTable = [[[i32; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT];

/// 各ワーカーが対局内で次の根の探索へ持ち越すbutterfly historyと捕獲履歴。
///
/// 探索チームは根の探索ごとに、両表をそれぞれ`HistoryDecay`%と`CaptureHistoryDecay`%へ減衰させる。
/// USI層は新規対局、受理した規則または`USI_Hash`の変更、ワーカー数の変更で初期化する。
/// 拒否した変更と同じワーカー数の再指定では保持する。
/// killer手とcorrection historyの補正表は持ち越さない。
/// 寿命の判断は`docs/adr/carry-history-tables-within-a-game.md`にある。
pub struct HistoryTables {
    pub(crate) workers: Vec<Box<HistoryTable>>,
    pub(crate) capture: Vec<CaptureHistory>,
}

impl HistoryTables {
    /// 指定したワーカー数の表を0で初期化する。
    pub fn new(threads: NonZeroUsize) -> Self {
        Self {
            workers: (0..threads.get())
                .map(|_| Box::new([[[0; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT]))
                .collect(),
            capture: (0..threads.get()).map(|_| CaptureHistory::new()).collect(),
        }
    }

    /// ワーカー数を保ったまま全ワーカーの表を0にする。
    pub fn clear(&mut self) {
        for history in &mut self.workers {
            for value in history.iter_mut().flatten().flatten() {
                *value = 0;
            }
        }
        for history in &mut self.capture {
            history.decay(0);
        }
    }
}

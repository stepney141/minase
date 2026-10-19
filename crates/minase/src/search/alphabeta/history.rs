//! 対局内で持ち越すワーカー別のbutterfly historyと捕獲履歴。

use core::num::NonZeroUsize;

use minase_core::board::{BOARD_SQUARE_COUNT, Square};
use minase_core::piece::{COLOR_COUNT, PieceCode, PieceKind};
use minase_core::position::{Position, Undo};

use super::capture_history::CaptureHistory;

/// 手番側・移動元・移動先で参照するhistory表。
pub(crate) type HistoryTable = [[[i32; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT];

/// 歩兵と仲人の絶対配置を表す鍵を局面から計算する。
/// 成った歩兵と仲人、手番、および成り権保留状態は含めない。
pub(super) fn pawn_key(position: &Position) -> u64 {
    Square::all()
        .filter_map(|square| {
            position
                .piece_at(square)
                .and_then(|piece| pawn_piece_key(piece, square))
        })
        .fold(0, |key, piece_key| key ^ piece_key)
}

/// 着手前の鍵へ`Undo`の移動と捕獲を反映する。
/// 居喰いとじっとでは移動元と到達升が同じでも、同じ排他的論理和で扱える。
pub(super) fn pawn_key_after_move(mut key: u64, undo: &Undo) -> u64 {
    if let Some(piece_key) = pawn_piece_key(undo.moved_piece_before, undo.mv.from) {
        key ^= piece_key;
    }
    let moved_piece_after = if undo.mv.promote {
        undo.moved_piece_before.promote().expect("合法な成り")
    } else {
        undo.moved_piece_before
    };
    if let Some(piece_key) = pawn_piece_key(moved_piece_after, undo.mv.to) {
        key ^= piece_key;
    }
    for captured in undo.captured.iter().flatten() {
        if let Some(piece_key) = pawn_piece_key(captured.piece, captured.square) {
            key ^= piece_key;
        }
    }
    key
}

fn pawn_piece_key(piece: PieceCode, square: Square) -> Option<u64> {
    let kind = match piece.kind()? {
        PieceKind::Pawn => 0,
        PieceKind::GoBetween => 1,
        _ => return None,
    };
    let color = piece.color()?.index();
    Some(PAWN_KEYS[(color * 2 + kind) * BOARD_SQUARE_COUNT + square.dense_index()])
}

/// 固定シードのsplitmix64で所有者、駒種、および升ごとの定数をコンパイル時に作る。
const PAWN_KEYS: [u64; COLOR_COUNT * 2 * BOARD_SQUARE_COUNT] = {
    let mut keys = [0; COLOR_COUNT * 2 * BOARD_SQUARE_COUNT];
    let mut state = 0x6d69_6e61_7365_0012_u64;
    let mut i = 0;
    while i < keys.len() {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        keys[i] = z ^ (z >> 31);
        i += 1;
    }
    keys
};

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

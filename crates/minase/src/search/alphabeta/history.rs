//! 対局内で持ち越すhistory表。
//!
//! Butterfly historyと捕獲履歴はワーカーごとに持ち、continuation historyとpawn historyは
//! 探索チームで共有する。各表は符号つきの更新量を上限へ切り詰めてから、共通の重力式で更新する。

use core::num::NonZeroUsize;
use std::sync::atomic::{AtomicI16, Ordering};

use minase_core::board::{BOARD_SQUARE_COUNT, Square};
use minase_core::piece::{COLOR_COUNT, PieceCode, PieceKind};
use minase_core::position::{Position, Undo};

use super::capture_history::CaptureHistory;

/// Butterfly historyの絶対値の上限。
/// Stockfishの`ButterflyHistory`と同じ7183を使う（src/history.h:128）。
pub(super) const HISTORY_LIMIT: i32 = 7183;

/// 新しい対局でのbutterfly historyの初期値。
/// Stockfishの初期化と同じ−5を使う（src/search.cpp:714）。
pub(super) const HISTORY_INITIAL: i32 = -5;

/// Continuation historyの絶対値の上限。
/// Stockfishの`PieceToHistory`と同じ30000を使う（src/history.h:138）。
pub(super) const CONTINUATION_HISTORY_LIMIT: i32 = 30_000;

/// 新しい対局でのcontinuation historyの初期値。
/// Stockfishの初期化と同じ−586を使う（src/search.cpp:722-727）。
pub(super) const CONTINUATION_HISTORY_INITIAL: i16 = -586;

/// Pawn historyの絶対値の上限。
/// Stockfishの`PawnHistory`と同じ8192を使う（src/history.h:146）。
pub(super) const PAWN_HISTORY_LIMIT: i32 = 8192;

/// 新しい対局でのpawn historyの初期値。
/// Stockfishの初期化と同じ−1338を使う（src/search.cpp:717-720）。
pub(super) const PAWN_HISTORY_INITIAL: i16 = -1338;

/// Low ply historyの絶対値の上限。
/// Stockfishの`LowPlyHistory`と同じ7183を使う（src/history.h:132）。
pub(super) const LOW_PLY_HISTORY_LIMIT: i32 = 7183;

/// Low ply historyを持つ根からのply数。
pub(super) const LOW_PLY_HISTORY_PLY_COUNT: usize = 5;

/// 探索開始時のlow ply historyの初期値。
/// Stockfishと同じ102を使う（src/search.cpp:339）。
pub(super) const LOW_PLY_HISTORY_INITIAL: i16 = 102;

/// Continuation historyの要素数。
pub(super) const CONTINUATION_HISTORY_LEN: usize = 2
    * 2
    * minase_core::piece::PIECE_KIND_COUNT
    * BOARD_SQUARE_COUNT
    * minase_core::piece::PIECE_KIND_COUNT
    * BOARD_SQUARE_COUNT;

/// Pawn historyのワーカー1つ分のエントリ数。
const PAWN_HISTORY_BASE_ENTRIES: usize = 1024;

/// Pawn historyの1エントリに含む要素数。
const PAWN_HISTORY_ENTRY_LEN: usize =
    COLOR_COUNT * minase_core::piece::PIECE_KIND_COUNT * BOARD_SQUARE_COUNT;

/// 手番側・移動元・移動先で参照するhistory表。
pub(crate) type HistoryTable = [[[i32; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT];

/// 根からのply・移動元・到達升で参照するワーカー専用の表。
pub(crate) type LowPlyHistory =
    [[[i16; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; LOW_PLY_HISTORY_PLY_COUNT];

/// 符号つきの更新量を表の上限で切り詰め、重力式で履歴値を更新する。
///
/// `v + b - trunc(v * |b| / D)`を計算し、積には`i64`を使う。
/// Stockfishの`StatsEntry::operator<<`と同じ式である（src/history.h:70-76）。
pub(super) fn history_gravity(value: i32, bonus: i32, limit: i32) -> i32 {
    let value = i64::from(value);
    let bonus = i64::from(bonus.clamp(-limit, limit));
    let updated = value + bonus - value * bonus.abs() / i64::from(limit);
    debug_assert!((i64::from(-limit)..=i64::from(limit)).contains(&updated));
    updated as i32
}

/// 文脈の捕獲種別・手番の一致・両手の着手前の駒種と到達升で参照する共有表。
///
/// 各要素は`Relaxed`順序で読み、新しい値を計算してから`Relaxed`順序で書く。
/// 読み書きの間に別ワーカーが更新した場合、その更新が失われることを許す。
pub(crate) struct ContinuationHistory {
    values: Box<[AtomicI16]>,
}

impl ContinuationHistory {
    fn new() -> Self {
        Self {
            values: atomic_history(CONTINUATION_HISTORY_LEN, CONTINUATION_HISTORY_INITIAL),
        }
    }

    fn clear(&self) {
        for value in &self.values {
            value.store(CONTINUATION_HISTORY_INITIAL, Ordering::Relaxed);
        }
    }

    /// 指定要素の値を返す。
    pub(super) fn read(&self, index: usize) -> i32 {
        i32::from(self.values[index].load(Ordering::Relaxed))
    }

    /// 指定要素へ上限30000の重力式で更新量を与える。
    pub(super) fn update(&self, index: usize, bonus: i32) {
        update_atomic_history(&self.values[index], bonus, CONTINUATION_HISTORY_LIMIT);
    }

    #[cfg(test)]
    pub(crate) fn store(&self, index: usize, value: i16) {
        self.values[index].store(value, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> &[AtomicI16] {
        &self.values
    }
}

/// 歩兵と仲人の配置の鍵、着手前の駒コード、および到達升で参照する共有表。
///
/// 各要素は`Relaxed`順序で読み、新しい値を計算してから`Relaxed`順序で書く。
/// 読み書きの間に別ワーカーが更新した場合、その更新が失われることを許す。
pub(crate) struct PawnHistory {
    values: Box<[AtomicI16]>,
    entry_mask: usize,
}

impl PawnHistory {
    fn new(threads: NonZeroUsize) -> Self {
        let worker_entries = threads
            .get()
            .checked_next_power_of_two()
            .expect("worker count must have a representable next power of two");
        let entries = PAWN_HISTORY_BASE_ENTRIES
            .checked_mul(worker_entries)
            .expect("pawn history entry count must fit usize");
        let len = entries
            .checked_mul(PAWN_HISTORY_ENTRY_LEN)
            .expect("pawn history length must fit usize");
        Self {
            values: atomic_history(len, PAWN_HISTORY_INITIAL),
            entry_mask: entries - 1,
        }
    }

    fn clear(&self) {
        for value in &self.values {
            value.store(PAWN_HISTORY_INITIAL, Ordering::Relaxed);
        }
    }

    /// 指定した鍵、駒、および到達升に対応する値を返す。
    #[cfg(test)]
    pub(super) fn read(&self, key: u64, piece: PieceCode, to: Square) -> i32 {
        i32::from(self.values[self.index(key, piece, to)].load(Ordering::Relaxed))
    }

    /// 指定要素へ上限8192の重力式で更新量を与える。
    pub(super) fn update(&self, key: u64, piece: PieceCode, to: Square, bonus: i32) {
        update_atomic_history(
            &self.values[self.index(key, piece, to)],
            bonus,
            PAWN_HISTORY_LIMIT,
        );
    }

    fn index(&self, key: u64, piece: PieceCode, to: Square) -> usize {
        let piece_index = piece.color().expect("pawn history mover color").index()
            * minase_core::piece::PIECE_KIND_COUNT
            + piece.kind().expect("pawn history mover kind").index();
        (((key as usize) & self.entry_mask) * COLOR_COUNT * minase_core::piece::PIECE_KIND_COUNT
            + piece_index)
            * BOARD_SQUARE_COUNT
            + to.dense_index()
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.entry_mask + 1
    }

    #[cfg(test)]
    pub(crate) fn store(&self, key: u64, piece: PieceCode, to: Square, value: i16) {
        self.values[self.index(key, piece, to)].store(value, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> &[AtomicI16] {
        &self.values
    }
}

fn atomic_history(len: usize, initial: i16) -> Box<[AtomicI16]> {
    std::iter::repeat_with(|| AtomicI16::new(initial))
        .take(len)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn update_atomic_history(value: &AtomicI16, bonus: i32, limit: i32) {
    let old = i32::from(value.load(Ordering::Relaxed));
    let updated = history_gravity(old, bonus, limit);
    value.store(updated as i16, Ordering::Relaxed);
}

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

/// 対局内で次の根の探索へ持ち越すhistory表。
///
/// 探索チームは根の探索ごとにワーカー別の2表だけを、それぞれ`HistoryDecay`%と
/// `CaptureHistoryDecay`%へ減衰させる。共有の2表は減衰させない。
/// USI層は新規対局、受理した規則または`USI_Hash`の変更、ワーカー数の変更で初期化する。
/// 拒否した変更と同じワーカー数の再指定では保持する。
/// killer手とcorrection historyの補正表は持ち越さない。
/// 寿命の判断は`docs/adr/carry-history-tables-within-a-game.md`にある。
pub struct HistoryTables {
    pub(crate) workers: Vec<Box<HistoryTable>>,
    pub(crate) capture: Vec<CaptureHistory>,
    pub(crate) continuation: ContinuationHistory,
    pub(crate) pawn: PawnHistory,
}

impl HistoryTables {
    /// 指定したワーカー数の表をStockfishと同じ初期値で初期化する。
    pub fn new(threads: NonZeroUsize) -> Self {
        Self {
            workers: (0..threads.get())
                .map(|_| {
                    Box::new(
                        [[[HISTORY_INITIAL; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; COLOR_COUNT],
                    )
                })
                .collect(),
            capture: (0..threads.get()).map(|_| CaptureHistory::new()).collect(),
            continuation: ContinuationHistory::new(),
            pawn: PawnHistory::new(threads),
        }
    }

    /// ワーカー数を保ったまま全ワーカーの表をStockfishと同じ初期値へ戻す。
    pub fn clear(&mut self) {
        for history in &mut self.workers {
            for value in history.iter_mut().flatten().flatten() {
                *value = HISTORY_INITIAL;
            }
        }
        for history in &mut self.capture {
            history.clear();
        }
        self.continuation.clear();
        self.pawn.clear();
    }
}

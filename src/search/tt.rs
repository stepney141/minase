//! 探索局面の置換表。

use core::mem::size_of;

use crate::Square;
use crate::core::mv::Move;

use super::{MATE_THRESHOLD, MAX_PLY};

/// 置換表の既定容量(MB)。
pub const DEFAULT_SIZE_MB: usize = 256;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Bound {
    Exact = 1,
    Lower = 2,
    Upper = 3,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    // 先頭8バイトと後半8バイトを分け、将来2個のu64へ移しやすくする。
    key: u32,
    best_move: u32,
    score: i16,
    depth: u8,
    bound: u8,
    generation: u8,
}

impl Entry {
    const EMPTY: Self = Self {
        key: 0,
        best_move: 0,
        score: 0,
        depth: 0,
        bound: 0,
        generation: 0,
    };

    fn bound(self) -> Option<Bound> {
        match self.bound {
            value if value == Bound::Exact as u8 => Some(Bound::Exact),
            value if value == Bound::Lower as u8 => Some(Bound::Lower),
            value if value == Bound::Upper as u8 => Some(Bound::Upper),
            _ => None,
        }
    }
}

const _: () = assert!(size_of::<Entry>() <= 16);

#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    pub(super) best_move: Move,
    pub(super) score: i32,
    pub(super) depth: u8,
    pub(super) bound: Bound,
}

/// 1スロット1エントリの直接マップ型置換表。
///
/// 容量はMB単位で受け取り、指定容量を超えない最大の2の冪個の
/// エントリを確保する。探索中は[`resize`](Self::resize)してはならない。
pub struct TranspositionTable {
    entries: Vec<Entry>,
    mask: usize,
    generation: u8,
}

impl TranspositionTable {
    /// 指定した容量(MB)で空の置換表を作る。
    ///
    /// # Panics
    ///
    /// `size_mb`が0、容量計算がオーバーフローする、または1エントリも
    /// 収容できない場合はpanicする。
    pub fn new(size_mb: usize) -> Self {
        let entry_count = entry_count(size_mb);
        Self {
            entries: vec![Entry::EMPTY; entry_count],
            mask: entry_count - 1,
            generation: 0,
        }
    }

    /// 全エントリを空にし、世代を初期化する。
    pub fn clear(&mut self) {
        self.entries.fill(Entry::EMPTY);
        self.generation = 0;
    }

    /// 探索中でない置換表を指定容量(MB)へ作り直す。
    ///
    /// # Panics
    ///
    /// `size_mb`の条件は[`new`](Self::new)と同じである。
    pub fn resize(&mut self, size_mb: usize) {
        *self = Self::new(size_mb);
    }

    pub(super) fn new_search(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn probe(&self, key: u64, ply: u32) -> Option<Hit> {
        let entry = self.entries[key as usize & self.mask];
        let bound = entry.bound()?;
        (entry.key == verification_key(key)).then(|| Hit {
            best_move: unpack_move(entry.best_move),
            score: score_from_tt(entry.score, ply),
            depth: entry.depth,
            bound,
        })
    }

    pub(super) fn store(
        &mut self,
        key: u64,
        depth: u32,
        score: i32,
        bound: Bound,
        best_move: Move,
        ply: u32,
    ) {
        let index = key as usize & self.mask;
        let existing = self.entries[index];
        let key = verification_key(key);
        let age = self.generation.wrapping_sub(existing.generation);
        // MAX_PLYは256だが、根以外の残り深さは最大255である。公開APIから
        // 256を渡されても比較を保守的にするため、格納幅の上限へ飽和させる。
        let depth = depth.min(u8::MAX as u32) as u8;
        let replace =
            existing.bound().is_none() || existing.key == key || age > 0 || existing.depth < depth;
        if !replace {
            return;
        }

        self.entries[index] = Entry {
            key,
            best_move: pack_move(best_move),
            score: score_to_tt(score, ply),
            depth,
            bound: bound as u8,
            generation: self.generation,
        };
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(DEFAULT_SIZE_MB)
    }
}

fn entry_count(size_mb: usize) -> usize {
    assert!(size_mb > 0, "transposition table size must be positive");
    let bytes = size_mb
        .checked_mul(1024 * 1024)
        .expect("transposition table size overflow");
    let capacity = bytes / size_of::<Entry>();
    assert!(capacity > 0, "transposition table is too small");
    1 << capacity.ilog2()
}

fn verification_key(key: u64) -> u32 {
    (key >> 32) as u32
}

pub(super) fn pack_move(mv: Move) -> u32 {
    let from = mv.from.dense_index() as u32;
    let mid = mv.mid.map_or(0xff, |square| square.dense_index() as u32);
    let to = mv.to.dense_index() as u32;
    from | (mid << 8) | (to << 16) | (u32::from(mv.promote) << 24)
}

pub(super) fn unpack_move(packed: u32) -> Move {
    let from = Square::from_dense((packed & 0xff) as usize)
        .expect("stored move must have a valid source square");
    let mid = ((packed >> 8) & 0xff) as usize;
    let mid = (mid != 0xff).then(|| {
        Square::from_dense(mid).expect("stored move must have a valid intermediate square")
    });
    let to = Square::from_dense(((packed >> 16) & 0xff) as usize)
        .expect("stored move must have a valid destination square");
    Move {
        from,
        mid,
        to,
        promote: packed & (1 << 24) != 0,
    }
}

fn score_to_tt(score: i32, ply: u32) -> i16 {
    let score = if score >= MATE_THRESHOLD {
        score + ply as i32
    } else if score <= -MATE_THRESHOLD {
        score - ply as i32
    } else {
        score
    };
    i16::try_from(score).expect("transposition table score must fit in i16")
}

fn score_from_tt(score: i16, ply: u32) -> i32 {
    let score = i32::from(score);
    if score >= MATE_THRESHOLD {
        score - ply as i32
    } else if score <= -MATE_THRESHOLD {
        score + ply as i32
    } else {
        score
    }
}

const _: () = assert!(MAX_PLY == 256);

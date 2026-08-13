//! 探索局面の置換表。

use core::mem::size_of;

use crate::Square;
use crate::core::mv::Move;

use super::{MATE_THRESHOLD, MAX_PLY};

/// 置換表の既定容量(MB)。
pub const DEFAULT_SIZE_MB: usize = 256;

/// 格納された評価値と探索窓の関係。0は空エントリの目印に予約する。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Bound {
    /// 窓内で確定した正確な値。
    Exact = 1,
    /// ベータカットによる下界。
    Lower = 2,
    /// 窓を下回った上界。
    Upper = 3,
}

/// 置換表の1エントリ。
#[repr(C)]
#[derive(Clone, Copy)]
struct Entry {
    // 先頭8バイトと後半8バイトを分け、将来2個のu64へ移しやすくする。
    /// 局面キーの上位32ビットによる照合キー。
    key: u32,
    /// 詰め込み表現の最善手。
    best_move: u32,
    /// 格納形式(根からの手数を除いた形)の評価値。
    score: i16,
    /// 探索した残り深さ。
    depth: u8,
    /// [`Bound`]の判別値。0は空エントリを表す。
    bound: u8,
    /// 書き込んだ探索の世代。
    generation: u8,
}

impl Entry {
    /// 空エントリ。bound=0が未使用の目印になる。
    const EMPTY: Self = Self {
        key: 0,
        best_move: 0,
        score: 0,
        depth: 0,
        bound: 0,
        generation: 0,
    };

    /// 判別値を[`Bound`]へ戻す。空エントリでは`None`を返す。
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

/// 置換表の照合に成功したエントリの内容。
#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    /// 格納されていた最善手。
    pub(super) best_move: Move,
    /// 現在の手数基準へ戻した評価値。
    pub(super) score: i32,
    /// 格納時の残り深さ。
    pub(super) depth: u8,
    /// 評価値と探索窓の関係。
    pub(super) bound: Bound,
}

/// 1スロット1エントリの直接マップ型置換表。
///
/// 容量はMB単位で受け取り、指定容量を超えない最大の2の冪個の
/// エントリを確保する。探索中は[`resize`](Self::resize)してはならない。
pub struct TranspositionTable {
    /// エントリの配列。長さは2の冪。
    entries: Vec<Entry>,
    /// 局面キーからスロット番号を取り出すビットマスク。
    mask: usize,
    /// 現在の探索の世代。
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

    /// 新しい探索の開始を記録し、既存エントリを置換候補として古びさせる。
    pub(super) fn new_search(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// 局面キーに対応するエントリを照合して返す。
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

    /// 探索結果をエントリへ書き込む。
    ///
    /// 同一局面、過去世代、またはより深い結果は既存エントリを置き換え、
    /// 同世代のより浅い結果は捨てる。
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

/// 指定容量(MB)に収まる最大の2の冪のエントリ数を返す。
fn entry_count(size_mb: usize) -> usize {
    assert!(size_mb > 0, "transposition table size must be positive");
    let bytes = size_mb
        .checked_mul(1024 * 1024)
        .expect("transposition table size overflow");
    let capacity = bytes / size_of::<Entry>();
    assert!(capacity > 0, "transposition table is too small");
    1 << capacity.ilog2()
}

/// 局面キーの上位32ビットを照合キーとして取り出す。
/// スロット番号は下位ビットから作るため、成分が重ならない。
fn verification_key(key: u64) -> u32 {
    (key >> 32) as u32
}

/// 指し手を32ビットへ詰め込む。中間升なしは0xffで表す。
pub(super) fn pack_move(mv: Move) -> u32 {
    let from = mv.from.dense_index() as u32;
    let mid = mv.mid.map_or(0xff, |square| square.dense_index() as u32);
    let to = mv.to.dense_index() as u32;
    from | (mid << 8) | (to << 16) | (u32::from(mv.promote) << 24)
}

/// 詰め込み表現から指し手を復元する。
///
/// # Panics
///
/// 升番号が盤上の範囲を外れている場合にパニックする。
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

/// 評価値を格納形式へ変換する。
///
/// 詰みの評価値は根からの手数を含むため、現在ノードからの手数基準へ
/// 直してから格納し、別の深さでの再利用時に正しく戻せるようにする。
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

/// 格納形式の評価値を、現在ノードの根からの手数基準へ戻す。
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

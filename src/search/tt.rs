//! 探索局面の置換表。

use core::mem::size_of;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::fmt;

#[cfg(not(target_has_atomic = "64"))]
compile_error!("the transposition table requires 64-bit atomic integers");

use crate::Square;
use crate::core::mv::Move;

use super::{MATE_THRESHOLD, MAX_PLY};

/// 置換表の既定容量(MB)。
pub const DEFAULT_SIZE_MB: usize = 256;

/// 置換表の構築またはサイズ変更に失敗した原因。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TranspositionTableError {
    /// 容量が0である。
    Empty,
    /// MiBからbyteへの容量計算が`usize`に収まらない。
    SizeOverflow,
    /// エントリ配列のメモリを確保できない。
    AllocationFailed,
}

impl fmt::Display for TranspositionTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "transposition table size must be positive",
            Self::SizeOverflow => "transposition table size overflow",
            Self::AllocationFailed => "transposition table allocation failed",
        })
    }
}

impl std::error::Error for TranspositionTableError {}

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

/// `critical`の検証キーはbit 0..=31に置く。
pub(super) const CRITICAL_KEY_SHIFT: u32 = 0;
/// `critical`の検証キーマスク。
pub(super) const CRITICAL_KEY_MASK: u64 = 0xffff_ffff << CRITICAL_KEY_SHIFT;
/// `critical`の評価値はbit 32..=47に置く。
pub(super) const CRITICAL_SCORE_SHIFT: u32 = 32;
/// `critical`の評価値マスク。
pub(super) const CRITICAL_SCORE_MASK: u64 = 0xffff << CRITICAL_SCORE_SHIFT;
/// `critical`の深さはbit 48..=55に置く。
pub(super) const CRITICAL_DEPTH_SHIFT: u32 = 48;
/// `critical`の深さマスク。
pub(super) const CRITICAL_DEPTH_MASK: u64 = 0xff << CRITICAL_DEPTH_SHIFT;
/// `critical`のバウンドはbit 56..=57に置く。
pub(super) const CRITICAL_BOUND_SHIFT: u32 = 56;
/// `critical`のバウンドマスク。
pub(super) const CRITICAL_BOUND_MASK: u64 = 0b11 << CRITICAL_BOUND_SHIFT;
/// `critical`の予約領域はbit 58..=63に置き、常に0にする。
pub(super) const CRITICAL_RESERVED_MASK: u64 = 0b11_1111 << 58;

/// `advisory`の指し手はbit 0..=24に置く。
pub(super) const ADVISORY_MOVE_SHIFT: u32 = 0;
/// `advisory`の指し手マスク。
pub(super) const ADVISORY_MOVE_MASK: u64 = 0x01ff_ffff << ADVISORY_MOVE_SHIFT;
/// `advisory`の世代はbit 25..=32に置く。
pub(super) const ADVISORY_GENERATION_SHIFT: u32 = 25;
/// `advisory`の世代マスク。
pub(super) const ADVISORY_GENERATION_MASK: u64 = 0xff << ADVISORY_GENERATION_SHIFT;
/// `advisory`の予約領域はbit 33..=63に置き、常に0にする。
pub(super) const ADVISORY_RESERVED_MASK: u64 = 0x7fff_ffff << 33;
/// 指し手25ビットがすべて1の「手なし」符号。
pub(super) const NO_MOVE: u32 = 0x01ff_ffff;

/// 置換表の1エントリ。2個の64ビット原子値で厳密に16バイトを占める。
#[repr(C)]
struct Entry {
    /// 検証キー、評価値、深さ、バウンド、および予約領域。
    critical: AtomicU64,
    /// 指し手、世代、および予約領域。
    advisory: AtomicU64,
}

impl Entry {
    /// 空エントリ。`critical`のバウンド0が未使用の目印になる。
    fn empty() -> Self {
        Self {
            critical: AtomicU64::new(0),
            advisory: AtomicU64::new(0),
        }
    }
}

const _: () = assert!(size_of::<Entry>() == 16);

/// テスト用にエントリ型のバイト数を返す。
#[cfg(test)]
pub(super) const fn entry_size() -> usize {
    size_of::<Entry>()
}

/// `critical`から取り出した探索値。
struct CriticalFields {
    /// 局面キーの上位32ビットによる照合キー。
    key: u32,
    /// 格納形式の評価値。
    score: i16,
    /// 探索した残り深さ。
    depth: u8,
    /// 評価値と探索窓の関係。
    bound: Bound,
}

/// 置換表の照合に成功したエントリの内容。
#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    /// 手順序付けにだけ使う助言手。
    ///
    /// 並行書込み時は検証キーおよび評価値と同じ格納操作に由来する保証がない。
    /// 生成済み合法手との一致を確認せず、着手や枝刈りに使ってはならない。
    pub(super) best_move: Option<Move>,
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
    generation: AtomicU8,
}

impl TranspositionTable {
    /// 指定した容量(MB)で空の置換表を作る。
    ///
    /// # Errors
    ///
    /// `size_mb`が0、容量計算がオーバーフローする、またはエントリ配列を
    /// 確保できない場合はエラーを返す。
    pub fn new(size_mb: usize) -> Result<Self, TranspositionTableError> {
        let entry_count = entry_count(size_mb)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(entry_count)
            .map_err(|_| TranspositionTableError::AllocationFailed)?;
        entries.resize_with(entry_count, Entry::empty);
        Ok(Self {
            entries,
            mask: entry_count - 1,
            generation: AtomicU8::new(0),
        })
    }

    /// 全エントリを空にし、世代を初期化する。
    pub fn clear(&mut self) {
        for entry in &mut self.entries {
            *entry = Entry::empty();
        }
        *self.generation.get_mut() = 0;
    }

    /// 探索中でない置換表を指定容量(MB)へ作り直す。
    ///
    /// # Errors
    ///
    /// `size_mb`の条件は[`new`](Self::new)と同じである。失敗した場合は
    /// 既存の置換表を変更しない。
    pub fn resize(&mut self, size_mb: usize) -> Result<(), TranspositionTableError> {
        let replacement = Self::new(size_mb)?;
        *self = replacement;
        Ok(())
    }

    /// 新しい探索の開始を記録し、既存エントリを置換候補として古びさせる。
    pub(super) fn new_search(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// テスト用に現在の世代カウンタを返す。
    #[cfg(test)]
    pub(super) fn generation(&self) -> u8 {
        self.generation.load(Ordering::Relaxed)
    }

    /// 局面キーに対応するエントリを照合して返す。
    ///
    /// 照合対象は評価値、深さ、バウンドであり、[`Hit::best_move`]は別の
    /// 原子語にある助言値である。呼出し側は合法手との一致を検証して使う。
    pub(super) fn probe(&self, key: u64, ply: u32) -> Option<Hit> {
        let entry = &self.entries[key as usize & self.mask];
        let critical = entry.critical.load(Ordering::Acquire);
        let fields = unpack_critical(critical)?;
        if fields.key != verification_key(key) {
            return None;
        }
        let advisory = entry.advisory.load(Ordering::Relaxed);
        let best_move = unpack_move(unpack_advisory_move(advisory))?;
        Some(Hit {
            best_move,
            score: score_from_tt(fields.score, ply),
            depth: fields.depth,
            bound: fields.bound,
        })
    }

    /// 探索結果をエントリへ書き込む。
    ///
    /// 同一局面は既存以上の深さ、異なる局面は過去世代または既存より深い
    /// 結果だけが既存エントリを置き換える。
    pub(super) fn store(
        &self,
        key: u64,
        depth: u32,
        score: i32,
        bound: Bound,
        best_move: Option<Move>,
        ply: u32,
    ) {
        let index = key as usize & self.mask;
        let entry = &self.entries[index];
        let existing_critical = entry.critical.load(Ordering::Acquire);
        let existing_advisory = entry.advisory.load(Ordering::Relaxed);
        let existing = unpack_critical(existing_critical);
        let key = verification_key(key);
        let generation = self.generation.load(Ordering::Relaxed);
        let age = generation.wrapping_sub(unpack_advisory_generation(existing_advisory));
        // MAX_PLYは256だが、根以外の残り深さは最大255である。公開APIから
        // 256を渡されても比較を保守的にするため、格納幅の上限へ飽和させる。
        let depth = depth.min(u8::MAX as u32) as u8;
        let replace = match existing {
            None => true,
            Some(existing) if existing.key == key => depth >= existing.depth,
            Some(existing) => age > 0 || depth > existing.depth,
        };
        if !replace {
            return;
        }

        let advisory = pack_advisory(best_move, generation);
        let critical = pack_critical(key, score_to_tt(score, ply), depth, bound);
        entry.advisory.store(advisory, Ordering::Relaxed);
        entry.critical.store(critical, Ordering::Release);
    }

    /// テスト用にキーが指すスロットの生の原子値を返す。
    #[cfg(test)]
    pub(super) fn raw_entry(&self, key: u64) -> (u64, u64) {
        let entry = &self.entries[key as usize & self.mask];
        (
            entry.critical.load(Ordering::Acquire),
            entry.advisory.load(Ordering::Relaxed),
        )
    }

    /// テスト用にキーが指すスロットへ生の原子値を書き込む。
    #[cfg(test)]
    pub(super) fn write_raw(&self, key: u64, critical: u64, advisory: u64) {
        let entry = &self.entries[key as usize & self.mask];
        entry.advisory.store(advisory, Ordering::Relaxed);
        entry.critical.store(critical, Ordering::Release);
    }
}

/// 指定容量(MB)に収まる最大の2の冪のエントリ数を返す。
fn entry_count(size_mb: usize) -> Result<usize, TranspositionTableError> {
    if size_mb == 0 {
        return Err(TranspositionTableError::Empty);
    }
    let bytes = size_mb
        .checked_mul(1024 * 1024)
        .ok_or(TranspositionTableError::SizeOverflow)?;
    let capacity = bytes / size_of::<Entry>();
    Ok(1 << capacity.ilog2())
}

/// 局面キーの上位32ビットを照合キーとして取り出す。
/// スロット番号は下位ビットから作るため、成分が重ならない。
fn verification_key(key: u64) -> u32 {
    (key >> 32) as u32
}

/// 探索値を`critical`のビット配置へ詰め込む。
fn pack_critical(key: u32, score: i16, depth: u8, bound: Bound) -> u64 {
    (u64::from(key) << CRITICAL_KEY_SHIFT)
        | (u64::from(score as u16) << CRITICAL_SCORE_SHIFT)
        | (u64::from(depth) << CRITICAL_DEPTH_SHIFT)
        | ((bound as u64) << CRITICAL_BOUND_SHIFT)
}

/// `critical`を探索値へ復号する。予約領域またはバウンドが不正なら失敗する。
fn unpack_critical(critical: u64) -> Option<CriticalFields> {
    if critical & CRITICAL_RESERVED_MASK != 0 {
        return None;
    }
    let bound = match ((critical & CRITICAL_BOUND_MASK) >> CRITICAL_BOUND_SHIFT) as u8 {
        value if value == Bound::Exact as u8 => Bound::Exact,
        value if value == Bound::Lower as u8 => Bound::Lower,
        value if value == Bound::Upper as u8 => Bound::Upper,
        _ => return None,
    };
    Some(CriticalFields {
        key: ((critical & CRITICAL_KEY_MASK) >> CRITICAL_KEY_SHIFT) as u32,
        score: ((critical & CRITICAL_SCORE_MASK) >> CRITICAL_SCORE_SHIFT) as u16 as i16,
        depth: ((critical & CRITICAL_DEPTH_MASK) >> CRITICAL_DEPTH_SHIFT) as u8,
        bound,
    })
}

/// 助言情報を`advisory`のビット配置へ詰め込む。
fn pack_advisory(best_move: Option<Move>, generation: u8) -> u64 {
    let packed_move = best_move.map_or(NO_MOVE, pack_move);
    let advisory = (u64::from(packed_move) << ADVISORY_MOVE_SHIFT)
        | (u64::from(generation) << ADVISORY_GENERATION_SHIFT);
    debug_assert_eq!(advisory & ADVISORY_RESERVED_MASK, 0);
    advisory
}

/// `advisory`から詰め込み表現の指し手を取り出す。
fn unpack_advisory_move(advisory: u64) -> u32 {
    ((advisory & ADVISORY_MOVE_MASK) >> ADVISORY_MOVE_SHIFT) as u32
}

/// `advisory`から世代を取り出す。
fn unpack_advisory_generation(advisory: u64) -> u8 {
    ((advisory & ADVISORY_GENERATION_MASK) >> ADVISORY_GENERATION_SHIFT) as u8
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
/// 「手なし」なら`Some(None)`、升番号が盤外なら`None`を返す。
pub(super) fn unpack_move(packed: u32) -> Option<Option<Move>> {
    if packed == NO_MOVE {
        return Some(None);
    }
    let from = Square::from_dense((packed & 0xff) as usize)?;
    let mid = ((packed >> 8) & 0xff) as usize;
    let mid = if mid == 0xff {
        None
    } else {
        Some(Square::from_dense(mid)?)
    };
    let to = Square::from_dense(((packed >> 16) & 0xff) as usize)?;
    Some(Some(Move {
        from,
        mid,
        to,
        promote: packed & (1 << 24) != 0,
    }))
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

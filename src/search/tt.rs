//! 探索局面の置換表。

use core::mem::{align_of, size_of};
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
    /// クラスタ配列のメモリを確保できない。
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

/// 「置換表のクラスタ化」（strength-stage6.md）の64バイト境界に揃えた4エントリ。
#[repr(align(64))]
struct Cluster {
    /// 添字順に照会するエントリ。
    entries: [Entry; 4],
}

impl Cluster {
    /// 全エントリが空のクラスタ。
    fn empty() -> Self {
        Self {
            entries: core::array::from_fn(|_| Entry::empty()),
        }
    }
}

const _: () = assert!(size_of::<Cluster>() == 64);
const _: () = assert!(align_of::<Cluster>() == 64);

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
    /// 手順序付けに使う助言手。
    ///
    /// 記録手の有無は減深判断にも使う（strength-stage6.md「internal iterative reduction」節）。
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

/// 1スロット4エントリのクラスタ型置換表。
///
/// 容量はMB単位で受け取り、指定容量を超えない最大の2の冪個の
/// クラスタを確保する（strength-stage6.md「置換表のクラスタ化」）。
/// 探索中は[`resize`](Self::resize)してはならない。
pub struct TranspositionTable {
    /// クラスタの配列。長さは2の冪。
    clusters: Vec<Cluster>,
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
    /// `size_mb`が0、容量計算がオーバーフローする、またはクラスタ配列を
    /// 確保できない場合はエラーを返す。
    pub fn new(size_mb: usize) -> Result<Self, TranspositionTableError> {
        let cluster_count = cluster_count(size_mb)?;
        let mut clusters = Vec::new();
        clusters
            .try_reserve_exact(cluster_count)
            .map_err(|_| TranspositionTableError::AllocationFailed)?;
        clusters.resize_with(cluster_count, Cluster::empty);
        Ok(Self {
            clusters,
            mask: cluster_count - 1,
            generation: AtomicU8::new(0),
        })
    }

    /// 全エントリを空にし、世代を初期化する。
    pub fn clear(&mut self) {
        for cluster in &mut self.clusters {
            *cluster = Cluster::empty();
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

    /// クラスタ内で局面キーに最初に一致するエントリを返す。
    ///
    /// 照合対象は評価値、深さ、バウンドであり、[`Hit::best_move`]は別の
    /// 原子語にある助言値である。呼出し側は合法手との一致を検証して使う。
    pub(super) fn probe(&self, key: u64, ply: u32) -> Option<Hit> {
        let cluster = &self.clusters[key as usize & self.mask];
        for entry in &cluster.entries {
            let critical = entry.critical.load(Ordering::Acquire);
            let Some(fields) = unpack_critical(critical) else {
                continue;
            };
            if fields.key != verification_key(key) {
                continue;
            }
            let advisory = entry.advisory.load(Ordering::Relaxed);
            let best_move = unpack_move(unpack_advisory_move(advisory))?;
            return Some(Hit {
                best_move,
                score: score_from_tt(fields.score, ply),
                depth: fields.depth,
                bound: fields.bound,
            });
        }
        None
    }

    /// 探索結果をエントリへ書き込む。
    ///
    /// strength-stage6.md「置換表のクラスタ化」に従い、同一局面は既存以上の
    /// 深さだけを保存する。異なる局面は空きを優先し、空きがなければ
    /// 世代の周回差が最大、深さが最小、添字が最小の順で置換先を選ぶ。
    pub(super) fn store(
        &self,
        key: u64,
        depth: u32,
        score: i32,
        bound: Bound,
        best_move: Option<Move>,
        ply: u32,
    ) {
        let cluster = &self.clusters[key as usize & self.mask];
        let key = verification_key(key);
        let generation = self.generation.load(Ordering::Relaxed);
        // MAX_PLYは256だが、根以外の残り深さは最大255である。公開APIから
        // 256を渡されても比較を保守的にするため、格納幅の上限へ飽和させる。
        let depth = depth.min(u8::MAX as u32) as u8;
        let mut replacement = &cluster.entries[0];
        let mut oldest_age = 0;
        let mut shallowest_depth = u8::MAX;
        let mut empty = None;
        for entry in &cluster.entries {
            let critical = entry.critical.load(Ordering::Acquire);
            let advisory = entry.advisory.load(Ordering::Relaxed);
            if let Some(existing) = unpack_critical(critical)
                && existing.key == key
            {
                if depth < existing.depth {
                    return;
                }
                replacement = entry;
                empty = None;
                break;
            }
            if critical == 0 {
                // 空きより後ろに同一キーがある可能性があるので照合を続ける。
                if empty.is_none() {
                    empty = Some(entry);
                }
                continue;
            }
            let age = generation.wrapping_sub(unpack_advisory_generation(advisory));
            let existing_depth = ((critical & CRITICAL_DEPTH_MASK) >> CRITICAL_DEPTH_SHIFT) as u8;
            if age > oldest_age || (age == oldest_age && existing_depth < shallowest_depth) {
                replacement = entry;
                oldest_age = age;
                shallowest_depth = existing_depth;
            }
        }
        let entry = empty.unwrap_or(replacement);

        let advisory = pack_advisory(best_move, generation);
        let critical = pack_critical(key, score_to_tt(score, ply), depth, bound);
        entry.advisory.store(advisory, Ordering::Relaxed);
        entry.critical.store(critical, Ordering::Release);
    }

    /// テスト用にキーが指すクラスタの先頭エントリの生の原子値を返す。
    #[cfg(test)]
    pub(super) fn raw_entry(&self, key: u64) -> (u64, u64) {
        let entry = &self.clusters[key as usize & self.mask].entries[0];
        (
            entry.critical.load(Ordering::Acquire),
            entry.advisory.load(Ordering::Relaxed),
        )
    }

    /// テスト用にキーが指すクラスタの先頭エントリへ生の原子値を書き込む。
    #[cfg(test)]
    pub(super) fn write_raw(&self, key: u64, critical: u64, advisory: u64) {
        let entry = &self.clusters[key as usize & self.mask].entries[0];
        entry.advisory.store(advisory, Ordering::Relaxed);
        entry.critical.store(critical, Ordering::Release);
    }
}

/// 指定容量(MB)に収まる最大の2の冪のクラスタ数を返す。
fn cluster_count(size_mb: usize) -> Result<usize, TranspositionTableError> {
    if size_mb == 0 {
        return Err(TranspositionTableError::Empty);
    }
    let bytes = size_mb
        .checked_mul(1024 * 1024)
        .ok_or(TranspositionTableError::SizeOverflow)?;
    let capacity = bytes / size_of::<Cluster>();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::sq;

    // strength-stage6.md「置換表のクラスタ化」「検証」。下位ビットは共通で、
    // 上位32ビットだけが異なる。検証キー0も空エントリと区別する。
    const KEYS: [u64; 5] = [
        0x0000_0000_0000_0042,
        0x1111_1111_0000_0042,
        0x2222_2222_0000_0042,
        0x3333_3333_0000_0042,
        0x4444_4444_0000_0042,
    ];

    #[test]
    fn cluster_layout_and_capacity_follow_the_specification() {
        assert_eq!(size_of::<Entry>(), 16);
        assert_eq!(size_of::<Cluster>(), 64);
        assert_eq!(align_of::<Cluster>(), 64);
        for (size_mb, expected_clusters) in [(1, 16_384), (2, 32_768), (3, 32_768), (4, 65_536)] {
            let table = TranspositionTable::new(size_mb).unwrap();
            assert_eq!(table.clusters.len(), expected_clusters);
            assert_eq!(table.mask, expected_clusters - 1);
            assert_eq!(table.clusters.as_ptr() as usize % 64, 0);
        }
    }

    #[test]
    fn cluster_resize_preserves_error_contract_and_resets_contents_on_success() {
        let mut table = TranspositionTable::new(1).unwrap();
        table.new_search();
        table.store(KEYS[0], 8, 100, Bound::Exact, None, 0);
        for (size_mb, error) in [
            (0, TranspositionTableError::Empty),
            (usize::MAX, TranspositionTableError::SizeOverflow),
        ] {
            assert_eq!(table.resize(size_mb), Err(error));
            assert_eq!(table.probe(KEYS[0], 0).unwrap().score, 100);
            assert_eq!(table.generation(), 1);
            assert_eq!(table.clusters.len(), 16_384);
        }
        table.resize(3).unwrap();
        assert_eq!(table.clusters.len(), 32_768);
        assert_eq!(table.mask, 32_767);
        assert_eq!(table.generation(), 0);
        assert!(table.probe(KEYS[0], 0).is_none());
    }

    #[test]
    fn cluster_keeps_four_colliding_positions_in_empty_entries() {
        let table = TranspositionTable::new(1).unwrap();
        let bounds = [Bound::Exact, Bound::Lower, Bound::Upper, Bound::Exact];
        let moves = core::array::from_fn::<_, 4, _>(|index| {
            Some(Move {
                from: sq(index as u8, 0),
                mid: None,
                to: sq(index as u8, 1),
                promote: false,
            })
        });
        for index in 0..4 {
            table.new_search();
            table.store(
                KEYS[index],
                8 - index as u32,
                100 + index as i32,
                bounds[index],
                moves[index],
                0,
            );
        }
        for index in 0..4 {
            let hit = table.probe(KEYS[index], 0).unwrap();
            assert_eq!(hit.depth, 8 - index as u8);
            assert_eq!(hit.score, 100 + index as i32);
            assert_eq!(hit.bound, bounds[index]);
            assert_eq!(hit.best_move, moves[index]);
        }
        assert!(table.probe(KEYS[4], 0).is_none());
    }

    #[test]
    fn cluster_replaces_oldest_generation_including_wraparound() {
        for initial_generation in [0, 252] {
            let table = TranspositionTable::new(1).unwrap();
            for _ in 0..initial_generation {
                table.new_search();
            }
            for (index, &key) in KEYS[..4].iter().enumerate() {
                // 最古になる添字1を最深にし、深さより古さが優先されることを検査する。
                let depth = if index == 1 { 20 } else { 1 };
                table.store(key, depth, 100, Bound::Exact, None, 0);
                table.new_search();
            }
            assert_eq!(
                table.generation(),
                if initial_generation == 0 { 4 } else { 0 }
            );
            // 添字0だけ現在世代へ更新し、古さを順に0、3、2、1にする。
            table.store(KEYS[0], 1, 200, Bound::Exact, None, 0);
            table.store(KEYS[4], 0, 500, Bound::Lower, None, 0);
            assert!(table.probe(KEYS[1], 0).is_none());
            for &key in &[KEYS[0], KEYS[2], KEYS[3], KEYS[4]] {
                assert!(table.probe(key, 0).is_some());
            }
            assert_eq!(table.probe(KEYS[4], 0).unwrap().score, 500);
        }
    }

    #[test]
    fn cluster_breaks_age_ties_by_depth_then_index() {
        for (depths, evicted) in [([8, 3, 6, 5], 1), ([8, 3, 3, 5], 1), ([3; 4], 0)] {
            let table = TranspositionTable::new(1).unwrap();
            for (&key, depth) in KEYS[..4].iter().zip(depths) {
                table.store(key, depth, 100, Bound::Exact, None, 0);
            }
            // 現在世代で、どの既存値より浅い結果でも空きがなければ置き換える。
            table.store(KEYS[4], 0, 500, Bound::Lower, None, 0);
            for (index, &key) in KEYS.iter().enumerate() {
                assert_eq!(table.probe(key, 0).is_none(), index == evicted);
            }
        }
    }

    #[test]
    fn cluster_matches_existing_key_before_an_empty_or_older_entry() {
        for leave_empty in [false, true] {
            let table = TranspositionTable::new(1).unwrap();
            for &key in &KEYS[..4] {
                table.store(key, 8, 100, Bound::Exact, None, 0);
            }
            if leave_empty {
                table.write_raw(KEYS[0], 0, 0);
            }
            table.new_search();
            // 添字2の既存値より浅い結果は、空きや別の置換候補があっても保存しない。
            table.store(KEYS[2], 0, 200, Bound::Lower, None, 0);
            let hit = table.probe(KEYS[2], 0).unwrap();
            assert_eq!((hit.score, hit.depth, hit.bound), (100, 8, Bound::Exact));

            table.store(KEYS[2], 8, 300, Bound::Upper, None, 0);
            let hit = table.probe(KEYS[2], 0).unwrap();
            assert_eq!((hit.score, hit.depth, hit.bound), (300, 8, Bound::Upper));
            assert_eq!(table.probe(KEYS[0], 0).is_none(), leave_empty);
            for &key in &[KEYS[1], KEYS[3]] {
                assert_eq!(table.probe(key, 0).unwrap().score, 100);
            }
            // 空きへ同一キーを重複保存していないことを、後続の保存で検査する。
            if leave_empty {
                table.store(KEYS[4], 0, 500, Bound::Exact, None, 0);
                for &key in &KEYS[1..] {
                    assert!(table.probe(key, 0).is_some());
                }
            }
        }
    }

    #[test]
    fn cluster_probe_returns_first_matching_entry() {
        let mut table = TranspositionTable::new(1).unwrap();
        for &key in &KEYS[..4] {
            table.store(key, 8, 100, Bound::Exact, None, 0);
        }
        // 並行保存で同一キーが複数エントリに残った状態を構成する。
        let cluster = &mut table.clusters[KEYS[0] as usize & table.mask];
        *cluster.entries[1].critical.get_mut() =
            pack_critical(verification_key(KEYS[3]), 200, 2, Bound::Lower);
        let hit = table.probe(KEYS[3], 0).unwrap();
        assert_eq!((hit.score, hit.depth, hit.bound), (200, 2, Bound::Lower));
    }
}

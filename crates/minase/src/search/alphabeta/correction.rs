//! 材料、駒配置、および探索経路の着手を鍵にした静的評価の補正（correction history）。
//!
//! 手番側ごとに材料、pawn、minor、先手と後手のnon-pawnの5表を引き、
//! 2手前、4手前、6手前の着手を文脈にした表の値とともに、重み付きの和から
//! 求めた補正値を通常探索の各ノードの静的評価と静止探索のstand-patに加える。
//! 補正後の静的評価は、静的評価を使う前向き枝刈りの判定のすべてに使う（Stockfish src/search.cpp:832, 846-870）。
//! 鍵を選んだ理由は`docs/adr/sum-material-placement-and-continuation-correction-histories-in-stockfish-form.md`にある。

use core::num::NonZeroUsize;
use std::sync::atomic::{AtomicI16, Ordering};

use super::history::history_gravity;
use super::ordering::oriented_square;
use super::params;
use super::royal::royal_under_attack;
use super::searcher::{MoveRecord, PathMove, Searcher};
use super::tt::Bound;
use crate::eval::{PIECE_STATE_COUNT, piece_state_of};
use crate::search::REPETITION_BAND_START;
use minase_core::board::BOARD_SQUARE_COUNT;
use minase_core::piece::{COLOR_COUNT, Color, PIECE_KIND_COUNT, PieceCode, PieceKind};
use minase_core::position::Undo;
use minase_core::{Position, Square};

impl Searcher<'_> {
    /// 現在のplyで引く全表の重み付きの和を返す。
    pub(super) fn correction_weighted_sum_at(&self, side: Color, ply: u32) -> i64 {
        let ply = ply as usize;
        let non_pawn_keys = self.non_pawn_keys[ply];
        let continuation = match continuation_correction_indices(&self.path_moves, side, ply) {
            Some([two, four, six]) => ContinuationCorrectionValues::Present {
                two: self.continuation_correction.read(two),
                four: self.continuation_correction.read(four),
                six: self.continuation_correction.read(six),
            },
            None => ContinuationCorrectionValues::NoPreviousMove,
        };
        correction_weighted_sum(
            CorrectionWeights::current(),
            self.correction
                .read(side, self.material_keys[ply], CorrectionTableKind::Material),
            self.correction
                .read(side, self.pawn_keys[ply], CorrectionTableKind::Pawn),
            self.correction
                .read(side, self.minor_keys[ply], CorrectionTableKind::Minor),
            self.correction.read(
                side,
                non_pawn_keys[Color::Black.index()],
                CorrectionTableKind::BlackNonPawn,
            ),
            self.correction.read(
                side,
                non_pawn_keys[Color::White.index()],
                CorrectionTableKind::WhiteNonPawn,
            ),
            continuation,
        )
    }

    /// 現在のplyで2手前、4手前、6手前の文脈から引く添字を返す。
    #[cfg(test)]
    pub(super) fn continuation_correction_indices(
        &self,
        side: Color,
        ply: u32,
    ) -> Option<[usize; 3]> {
        continuation_correction_indices(&self.path_moves, side, ply as usize)
    }

    /// 詰み帯と専用の帯の外で、境界と残差の条件を満たし、王駒に疑似利きが届かないノードだけを学習する。
    /// 反復による勝敗は経路に依存し、静的評価の誤差ではないため補正履歴に取り込まない。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn update_correction(
        &mut self,
        position: &Position,
        depth: u32,
        ply: u32,
        best_score: i32,
        bound: Bound,
        best_capture: bool,
        static_eval: i32,
        mut royal_attacked: Option<bool>,
    ) {
        if best_score.abs() >= REPETITION_BAND_START {
            return;
        }
        let Some(bonus) = correction_update_bonus(
            best_score - static_eval,
            depth,
            bound,
            best_capture,
            params::correction_bonus(),
            params::correction_fail_low_bonus(),
            self.nnue.pawn_value(),
        ) else {
            return;
        };
        if !*royal_attacked.get_or_insert_with(|| royal_under_attack(position)) {
            self.apply_correction_bonus(position.side_to_move(), ply, bonus);
        }
    }

    /// 求めた更新量を局面の5表と2手前、4手前、6手前の文脈へ順に適用する。
    pub(super) fn apply_correction_bonus(&mut self, side: Color, ply: u32, bonus: i32) {
        let ply = ply as usize;
        self.correction.update(
            side,
            self.material_keys[ply],
            CorrectionTableKind::Material,
            bonus,
            MATERIAL_UPDATE_MULTIPLIER,
        );
        self.correction.update(
            side,
            self.pawn_keys[ply],
            CorrectionTableKind::Pawn,
            bonus,
            PAWN_UPDATE_MULTIPLIER,
        );
        self.correction.update(
            side,
            self.minor_keys[ply],
            CorrectionTableKind::Minor,
            bonus,
            MINOR_UPDATE_MULTIPLIER,
        );
        self.correction.update(
            side,
            self.non_pawn_keys[ply][Color::Black.index()],
            CorrectionTableKind::BlackNonPawn,
            bonus,
            NON_PAWN_UPDATE_MULTIPLIER,
        );
        self.correction.update(
            side,
            self.non_pawn_keys[ply][Color::White.index()],
            CorrectionTableKind::WhiteNonPawn,
            bonus,
            NON_PAWN_UPDATE_MULTIPLIER,
        );
        if let Some(indices) = continuation_correction_indices(&self.path_moves, side, ply) {
            for (index, multiplier) in indices.into_iter().zip([
                CONTINUATION_2_UPDATE_MULTIPLIER,
                CONTINUATION_4_UPDATE_MULTIPLIER,
                CONTINUATION_6_UPDATE_MULTIPLIER,
            ]) {
                self.continuation_correction
                    .update(index, bonus, multiplier);
            }
        }
    }
}

/// 局面を鍵にした補正表のワーカー1つ分のエントリ数。
/// Stockfishの`CORRHIST_BASE_SIZE`と同じ65,536を使う（src/history.h:40）。
const CORRECTION_BASE_ENTRIES: usize = 65_536;
const TABLE_COUNT: usize = 5;
/// 補正表の要素と表ごとの更新量の絶対値の上限。
/// Stockfishと同じ1024を使う（src/history.h:41）。
pub(super) const CORRECTION_LIMIT: i32 = 1024;
/// 局面を鍵にした5表の初期値。Stockfishと同じ−5を使う（src/search.cpp:719）。
pub(super) const CORRECTION_INITIAL: i16 = -5;
/// 材料表の更新倍率。Stockfishに材料表はないため、基準のpawnと同じ128/128を使う
/// （src/search.cpp:130）。
const MATERIAL_UPDATE_MULTIPLIER: i32 = 128;
/// pawn表の更新倍率。Stockfishでは更新量をそのまま使う（src/search.cpp:130）。
const PAWN_UPDATE_MULTIPLIER: i32 = 128;
/// minor表の更新倍率。Stockfishと同じ150/128を使う（src/search.cpp:131）。
const MINOR_UPDATE_MULTIPLIER: i32 = 150;
/// non-pawn表の更新倍率。Stockfishと同じ186/128を使う（src/search.cpp:127, 132-133）。
const NON_PAWN_UPDATE_MULTIPLIER: i32 = 186;
/// 2手前の文脈の更新倍率。Stockfishと同じ130/128を使う（src/search.cpp:139）。
const CONTINUATION_2_UPDATE_MULTIPLIER: i32 = 130;
/// 4手前の文脈の更新倍率。Stockfishと同じ70/128を使う（src/search.cpp:140）。
const CONTINUATION_4_UPDATE_MULTIPLIER: i32 = 70;
/// 6手前の文脈の更新倍率。Stockfishと同じ35/128を使う（src/search.cpp:141）。
const CONTINUATION_6_UPDATE_MULTIPLIER: i32 = 35;
const UPDATE_MULTIPLIER_DENOMINATOR: i64 = 128;
const CORRECTION_DENOMINATOR: i64 = 102_400;
const UPDATE_BASE_LIMIT: i64 = 256;
const UPDATE_SCALE_NUMERATOR: i64 = 1061;
const UPDATE_SCALE_DENOMINATOR: i64 = 1024;
const CONTINUATION_CONTEXT_KIND_COUNT: usize = PIECE_KIND_COUNT + 1;
/// Continuation correction historyの要素数。
pub(super) const CONTINUATION_CORRECTION_LEN: usize =
    CONTINUATION_CONTEXT_KIND_COUNT * BOARD_SQUARE_COUNT * PIECE_KIND_COUNT * BOARD_SQUARE_COUNT;
/// Continuation correction historyの初期値。Stockfishと同じ5を使う（src/search.cpp:731-733）。
pub(super) const CONTINUATION_CORRECTION_INITIAL: i16 = 5;
/// 1手前の手がない場合に加える値。
/// Stockfishの80695を`80695 × 102400 / (131072 × 208)`で換算した303を使う
/// （src/search.cpp:104-109）。
const NO_PREVIOUS_MOVE_CORRECTION: i64 = 303;

/// 1つの配列に束ねる補正表の番号。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CorrectionTableKind {
    Material = 0,
    Pawn = 1,
    Minor = 2,
    BlackNonPawn = 3,
    WhiteNonPawn = 4,
}

/// 探索チームで共有する、材料、pawn、minor、先手と後手のnon-pawnの5表。
///
/// 各要素は`Relaxed`順序で読み、新しい値を計算してから`Relaxed`順序で書く。
/// 読み書きの間に別ワーカーが更新した場合、その更新が失われることを許す
/// （src/history.h:60-63）。
pub(crate) struct CorrectionValues {
    values: Box<[AtomicI16]>,
    entry_mask: usize,
}

impl CorrectionValues {
    /// ワーカー数以上の最小の2のべきに65,536を掛けたエントリ数で作る
    /// （src/history.h:40、src/thread.cpp:213）。
    pub(super) fn new(threads: NonZeroUsize) -> Self {
        let worker_entries = threads
            .get()
            .checked_next_power_of_two()
            .expect("worker count must have a representable next power of two");
        let entries = CORRECTION_BASE_ENTRIES
            .checked_mul(worker_entries)
            .expect("correction history entry count must fit usize");
        let len = entries
            .checked_mul(COLOR_COUNT)
            .and_then(|len| len.checked_mul(TABLE_COUNT))
            .expect("correction history length must fit usize");
        Self {
            values: std::iter::repeat_with(|| AtomicI16::new(CORRECTION_INITIAL))
                .take(len)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            entry_mask: entries - 1,
        }
    }

    pub(super) fn clear(&self) {
        for value in &self.values {
            value.store(CORRECTION_INITIAL, Ordering::Relaxed);
        }
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> &[AtomicI16] {
        &self.values
    }

    fn index(&self, side: Color, key: u64, table: CorrectionTableKind) -> usize {
        (((key as usize) & self.entry_mask) * COLOR_COUNT + side.index()) * TABLE_COUNT
            + table as usize
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.entry_mask + 1
    }
}

/// 単一ワーカー用の、全要素が−5の5表を作る。
#[cfg(test)]
pub(super) fn new_correction_values() -> CorrectionValues {
    CorrectionValues::new(NonZeroUsize::MIN)
}

/// 文脈の手と1手前の手で引く、ワーカー専用のcontinuation correction history。
pub(crate) struct ContinuationCorrectionHistory {
    values: Box<[i16]>,
}

impl ContinuationCorrectionHistory {
    /// 全要素が5の表を作る。
    pub(super) fn new() -> Self {
        Self {
            values: vec![CONTINUATION_CORRECTION_INITIAL; CONTINUATION_CORRECTION_LEN]
                .into_boxed_slice(),
        }
    }

    /// 全要素を初期値5へ戻す。
    pub(super) fn clear(&mut self) {
        self.values.fill(CONTINUATION_CORRECTION_INITIAL);
    }

    /// 指定要素の値を返す。
    pub(super) fn read(&self, index: usize) -> i32 {
        i32::from(self.values[index])
    }

    /// 文脈ごとの倍率を掛けた更新量を、指定要素へ重力式で取り込む。
    pub(super) fn update(&mut self, index: usize, bonus: i32, multiplier: i32) {
        self.values[index] = updated_correction(self.values[index], bonus, multiplier);
    }

    #[cfg(test)]
    pub(crate) fn store(&mut self, index: usize, value: i16) {
        self.values[index] = value;
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> &[i16] {
        &self.values
    }

    #[cfg(test)]
    pub(crate) fn values_mut(&mut self) -> &mut [i16] {
        &mut self.values
    }
}

/// 探索チームで共有する5表を共有参照で扱う。
pub(super) struct CorrectionTable<'a> {
    values: &'a CorrectionValues,
}

impl<'a> CorrectionTable<'a> {
    pub(super) fn new(values: &'a CorrectionValues) -> Self {
        Self { values }
    }

    /// 表の大きさに応じた鍵の下位ビット、手番側、および表の番号から要素を引く。
    pub(super) fn read(&self, side: Color, key: u64, table: CorrectionTableKind) -> i32 {
        i32::from(self.values.values[self.values.index(side, key, table)].load(Ordering::Relaxed))
    }

    /// 表ごとの倍率を掛けた更新量を、同じ添字の要素へ重力式で取り込む。
    pub(super) fn update(
        &self,
        side: Color,
        key: u64,
        table: CorrectionTableKind,
        bonus: i32,
        multiplier: i32,
    ) {
        let value = &self.values.values[self.values.index(side, key, table)];
        let old = value.load(Ordering::Relaxed);
        value.store(
            updated_correction(old, bonus, multiplier),
            Ordering::Relaxed,
        );
    }

    #[cfg(test)]
    pub(crate) fn store(&self, side: Color, key: u64, table: CorrectionTableKind, value: i16) {
        self.values.values[self.values.index(side, key, table)].store(value, Ordering::Relaxed);
    }
}

/// 文脈の手、1手前の手、および現在の手番側から平坦な表の添字を求める。
/// 文脈がなければ駒種29と回転しない升0の区画を使う。
pub(super) fn continuation_correction_index(
    context: Option<MoveRecord>,
    previous: MoveRecord,
    side: Color,
) -> usize {
    let (context_kind, context_to) = context.map_or((PIECE_KIND_COUNT, 0), |record| {
        (
            record
                .piece
                .kind()
                .expect("continuation correction context piece kind")
                .index(),
            oriented_square(record.mv.to.dense_index(), side),
        )
    });
    let previous_piece = if previous.mv.promote {
        previous
            .piece
            .promote()
            .expect("continuation correction previous move promotion")
    } else {
        previous.piece
    };
    let previous_kind = previous_piece
        .kind()
        .expect("continuation correction previous piece kind")
        .index();
    let previous_to = oriented_square(previous.mv.to.dense_index(), side);
    ((context_kind * BOARD_SQUARE_COUNT + context_to) * PIECE_KIND_COUNT + previous_kind)
        * BOARD_SQUARE_COUNT
        + previous_to
}

fn continuation_correction_indices(
    path_moves: &[PathMove],
    side: Color,
    ply: usize,
) -> Option<[usize; 3]> {
    let previous = path_record(path_moves, ply, 1)?;
    Some([2, 4, 6].map(|distance| {
        continuation_correction_index(path_record(path_moves, ply, distance), previous, side)
    }))
}

fn path_record(path_moves: &[PathMove], ply: usize, distance: usize) -> Option<MoveRecord> {
    ply.checked_sub(distance)
        .and_then(|index| path_moves.get(index))
        .and_then(|entry| entry.record)
}

#[derive(Clone, Copy)]
struct CorrectionWeights {
    material: i32,
    pawn: i32,
    minor: i32,
    non_pawn: i32,
    continuation: i32,
    continuation_six: i32,
}

impl CorrectionWeights {
    fn current() -> Self {
        Self {
            material: params::correction_material_weight(),
            pawn: params::correction_pawn_weight(),
            minor: params::correction_minor_weight(),
            non_pawn: params::correction_non_pawn_weight(),
            continuation: params::correction_continuation_weight(),
            continuation_six: params::correction_continuation6_weight(),
        }
    }
}

#[derive(Clone, Copy)]
enum ContinuationCorrectionValues {
    NoPreviousMove,
    Present { two: i32, four: i32, six: i32 },
}

fn correction_weighted_sum(
    weights: CorrectionWeights,
    material: i32,
    pawn: i32,
    minor: i32,
    black_non_pawn: i32,
    white_non_pawn: i32,
    continuation: ContinuationCorrectionValues,
) -> i64 {
    let continuation = match continuation {
        ContinuationCorrectionValues::NoPreviousMove => NO_PREVIOUS_MOVE_CORRECTION,
        ContinuationCorrectionValues::Present { two, four, six } => {
            i64::from(weights.continuation) * (i64::from(two) + i64::from(four))
                + i64::from(weights.continuation_six) * i64::from(six)
        }
    };
    i64::from(weights.material) * i64::from(material)
        + i64::from(weights.pawn) * i64::from(pawn)
        + i64::from(weights.minor) * i64::from(minor)
        + i64::from(weights.non_pawn) * (i64::from(black_non_pawn) + i64::from(white_non_pawn))
        + continuation
}

/// 表の重み付きの和から、静的評価と同じ単位の補正量を求める。
pub(super) fn correction_from_weighted_sum(weighted_sum: i64, pawn: i32) -> i32 {
    (i64::from(pawn) * weighted_sum / CORRECTION_DENOMINATOR) as i32
}

/// 残差、深さ、および境界から、全表へ渡す更新量を求める。
#[allow(clippy::too_many_arguments)]
pub(super) fn correction_update_bonus(
    difference: i32,
    depth: u32,
    bound: Bound,
    best_capture: bool,
    bonus: i32,
    fail_low_bonus: i32,
    pawn: i32,
) -> Option<i32> {
    let applies = match bound {
        Bound::Upper => difference <= 0,
        Bound::Lower | Bound::Exact => difference > 0 && !best_capture,
    };
    if !applies {
        return None;
    }
    let coefficient = if bound == Bound::Upper {
        fail_low_bonus
    } else {
        bonus
    };
    // Stockfishと同じく、残差と深さから作る更新量を±256へ切り詰める（src/search.cpp:1670-1671）。
    let base = (i64::from(difference) * i64::from(depth) * i64::from(coefficient)
        / i64::from(pawn))
    .clamp(-UPDATE_BASE_LIMIT, UPDATE_BASE_LIMIT);
    // Stockfishと同じく、切り詰めた後の更新量へ1061/1024を掛ける（src/search.cpp:1672）。
    Some((UPDATE_SCALE_NUMERATOR * base / UPDATE_SCALE_DENOMINATOR) as i32)
}

/// 表ごとの倍率を掛け、上限1024の重力式で要素を更新する。
fn updated_correction(value: i16, bonus: i32, multiplier: i32) -> i16 {
    let bonus = scaled_update_bonus(bonus, multiplier);
    history_gravity(i32::from(value), bonus, CORRECTION_LIMIT) as i16
}

fn scaled_update_bonus(bonus: i32, multiplier: i32) -> i32 {
    (i64::from(bonus) * i64::from(multiplier) / UPDATE_MULTIPLIER_DENOMINATOR)
        .clamp(i64::from(-CORRECTION_LIMIT), i64::from(CORRECTION_LIMIT)) as i32
}

/// 位置に依存しない駒種別枚数のハッシュ。加算なので同種の複数枚も区別する。
pub(super) fn material_key(position: &Position) -> u64 {
    Square::all()
        .filter_map(|square| position.piece_at(square))
        .fold(0, |key, piece| key.wrapping_add(piece_key(piece)))
}

/// 捕獲と成りだけを反映する。居喰いと2枚取りもUndoの捕獲記録に従う。
pub(super) fn key_after_move(mut key: u64, undo: &Undo) -> u64 {
    for captured in undo.captured.iter().flatten() {
        key = key.wrapping_sub(piece_key(captured.piece));
    }
    if undo.mv.promote {
        key = key.wrapping_sub(piece_key(undo.moved_piece_before));
        key = key.wrapping_add(piece_key(
            undo.moved_piece_before.promote().expect("合法な成り"),
        ));
    }
    key
}

fn piece_key(piece: PieceCode) -> u64 {
    PIECE_KEYS[piece.color().expect("盤上の駒").index() * PIECE_STATE_COUNT + piece_state_of(piece)]
}

/// 固定シードのsplitmix64で駒コードごとの定数をコンパイル時に作る。
const PIECE_KEYS: [u64; COLOR_COUNT * PIECE_STATE_COUNT] = {
    let mut keys = [0; COLOR_COUNT * PIECE_STATE_COUNT];
    let mut state = 0x6d69_6e61_7365_0008_u64;
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

/// minorと所有者別non-pawnの配置鍵。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CorrectionPieceKeys {
    pub(super) minor: u64,
    pub(super) non_pawn: [u64; COLOR_COUNT],
}

/// minorと所有者別non-pawnの配置鍵を局面から計算する。
pub(super) fn correction_piece_keys(position: &Position) -> CorrectionPieceKeys {
    let mut keys = CorrectionPieceKeys::default();
    for square in Square::all() {
        if let Some(piece) = position.piece_at(square) {
            toggle_piece_square_keys(&mut keys, piece, square);
        }
    }
    keys
}

/// 着手前の配置鍵へ`Undo`の移動、成り、およびすべての捕獲を反映する。
pub(super) fn correction_piece_keys_after_move(
    mut keys: CorrectionPieceKeys,
    undo: &Undo,
) -> CorrectionPieceKeys {
    toggle_piece_square_keys(&mut keys, undo.moved_piece_before, undo.mv.from);
    let moved_piece_after = if undo.mv.promote {
        undo.moved_piece_before.promote().expect("合法な成り")
    } else {
        undo.moved_piece_before
    };
    toggle_piece_square_keys(&mut keys, moved_piece_after, undo.mv.to);
    for captured in undo.captured.iter().flatten() {
        toggle_piece_square_keys(&mut keys, captured.piece, captured.square);
    }
    keys
}

fn toggle_piece_square_keys(keys: &mut CorrectionPieceKeys, piece: PieceCode, square: Square) {
    let kind = piece.kind().expect("盤上の駒");
    let key = piece_square_key(piece, square);
    if is_minor(kind) {
        keys.minor ^= key;
    }
    if !matches!(kind, PieceKind::Pawn | PieceKind::GoBetween) {
        let color = piece.color().expect("盤上の駒");
        keys.non_pawn[color.index()] ^= key;
    }
}

fn is_minor(kind: PieceKind) -> bool {
    matches!(
        kind,
        PieceKind::FerociousLeopard
            | PieceKind::CopperGeneral
            | PieceKind::SilverGeneral
            | PieceKind::GoldGeneral
            | PieceKind::BlindTiger
            | PieceKind::DrunkElephant
            | PieceKind::Kirin
    )
}

fn piece_square_key(piece: PieceCode, square: Square) -> u64 {
    let color = piece.color().expect("盤上の駒").index();
    let state = piece_state_of(piece);
    PIECE_SQUARE_KEYS
        [(color * PIECE_STATE_COUNT + state) * BOARD_SQUARE_COUNT + square.dense_index()]
}

/// 固定シードのsplitmix64で所有者、駒状態、および升ごとの定数をコンパイル時に作る。
static PIECE_SQUARE_KEYS: [u64; COLOR_COUNT * PIECE_STATE_COUNT * BOARD_SQUARE_COUNT] = {
    let mut keys = [0; COLOR_COUNT * PIECE_STATE_COUNT * BOARD_SQUARE_COUNT];
    let mut state = 0x6d69_6e61_7365_0013_u64;
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

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::mv::Move;
    use minase_core::rules::MoveRules;
    use minase_core::test_util::{position, position_from_codes, sq};
    use minase_core::{MoveGenerator, PieceKind};

    fn board_with_kings(mut pieces: Vec<(Square, Color, PieceKind)>) -> Position {
        pieces.extend([
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
        ]);
        position(Color::Black, &pieces)
    }

    fn assert_incremental_piece_keys(name: &str, board: &Position, mv: Move) {
        let rules = MoveRules::standard();
        let mut moves = Vec::new();
        MoveGenerator::new(rules).generate_moves(board, &mut moves);
        assert!(
            moves.contains(&mv),
            "{name}: test move must be legal: {mv:?}"
        );

        let before = correction_piece_keys(board);
        let mut after = board.clone();
        let undo = after.make_move_unchecked(mv, rules);
        assert_eq!(
            correction_piece_keys_after_move(before, &undo),
            correction_piece_keys(&after),
            "{name}: {mv:?}"
        );
        after.unmake_move(undo);
        assert_eq!(correction_piece_keys(&after), before, "{name}: undo");
    }

    #[test]
    fn correction_amount_rounds_toward_zero_and_static_eval_is_bounded() {
        assert_eq!(correction_from_weighted_sum(1023, 100), 0);
        assert_eq!(correction_from_weighted_sum(1024, 100), 1);
        assert_eq!(correction_from_weighted_sum(-1024, 100), -1);
        assert_eq!(correction_from_weighted_sum(277 * 1024, 100), 277);
        assert_eq!(correction_from_weighted_sum(500 * 1024, 100), 500);

        use crate::eval::nnue::EVALUATION_LIMIT;
        use crate::search::alphabeta::pruning::corrected_static_eval;
        assert_eq!(corrected_static_eval(EVALUATION_LIMIT, 1), EVALUATION_LIMIT);
        assert_eq!(
            corrected_static_eval(-EVALUATION_LIMIT, -1),
            -EVALUATION_LIMIT
        );
    }

    #[test]
    fn correction_update_bonus_matches_numeric_examples() {
        assert_eq!(
            correction_update_bonus(50, 3, Bound::Exact, false, 20, 29, 100),
            Some(31)
        );
        assert_eq!(
            correction_update_bonus(10_000, 10, Bound::Exact, false, 20, 29, 100),
            Some(265)
        );
        assert_eq!(
            correction_update_bonus(-50, 3, Bound::Upper, false, 20, 20, 100),
            Some(-31)
        );
        // 深さを8で頭打ちにせず、9手をそのまま掛ける。
        assert_eq!(
            correction_update_bonus(50, 9, Bound::Exact, false, 20, 29, 100),
            Some(93)
        );
    }

    #[test]
    fn correction_update_conditions_follow_bound_and_capture() {
        for (difference, bound, capture, expected) in [
            (-1, Bound::Upper, false, true),
            (0, Bound::Upper, true, true),
            (1, Bound::Upper, false, false),
            (1, Bound::Lower, false, true),
            (1, Bound::Lower, true, false),
            (0, Bound::Lower, false, false),
            (-1, Bound::Lower, false, false),
            (1, Bound::Exact, false, true),
            (1, Bound::Exact, true, false),
            (0, Bound::Exact, false, false),
            (-1, Bound::Exact, false, false),
        ] {
            assert_eq!(
                correction_update_bonus(difference, 100, bound, capture, 20, 29, 100).is_some(),
                expected,
                "difference={difference}, bound={bound:?}, capture={capture}"
            );
        }
    }

    #[test]
    fn correction_gravity_matches_numeric_examples_and_limits() {
        assert_eq!(updated_correction(-5, 31, 128), 26);
        assert_eq!(updated_correction(1000, 265, 128), 1007);
        assert_eq!(updated_correction(-5, 62, 64), 26);
        assert_eq!(updated_correction(1024, 10_000, 128), 1024);
        assert_eq!(updated_correction(-1024, -10_000, 128), -1024);

        let values = new_correction_values();
        let table = CorrectionTable::new(&values);
        assert_eq!(
            table.read(Color::Black, 7, CorrectionTableKind::Material),
            -5
        );
        assert_eq!(
            table.read(Color::White, 7, CorrectionTableKind::Material),
            -5
        );
        table.update(Color::Black, 7, CorrectionTableKind::Material, 31, 128);
        table.update(Color::White, 7, CorrectionTableKind::Material, -31, 128);
        assert_eq!(
            table.read(Color::Black, 7, CorrectionTableKind::Material),
            26
        );
        assert_eq!(
            table.read(Color::White, 7, CorrectionTableKind::Material),
            -36
        );
        assert_eq!(
            table.read(
                Color::Black,
                CORRECTION_BASE_ENTRIES as u64 + 7,
                CorrectionTableKind::Material,
            ),
            26
        );
    }

    #[test]
    fn correction_table_keeps_side_table_and_low_key_bits_distinct() {
        let values = new_correction_values();
        assert_eq!(
            values.values().len(),
            CORRECTION_BASE_ENTRIES * COLOR_COUNT * TABLE_COUNT
        );
        let table = CorrectionTable::new(&values);
        let key = 17;
        table.store(Color::Black, key, CorrectionTableKind::Material, 11);
        table.store(Color::White, key, CorrectionTableKind::Material, 12);
        table.store(Color::Black, key, CorrectionTableKind::Pawn, 13);
        table.store(Color::Black, key, CorrectionTableKind::BlackNonPawn, 14);
        table.store(Color::Black, key, CorrectionTableKind::WhiteNonPawn, 15);

        assert_eq!(
            table.read(Color::Black, key, CorrectionTableKind::Material),
            11
        );
        assert_eq!(
            table.read(Color::White, key, CorrectionTableKind::Material),
            12
        );
        assert_eq!(table.read(Color::Black, key, CorrectionTableKind::Pawn), 13);
        assert_eq!(
            table.read(Color::Black, key, CorrectionTableKind::BlackNonPawn),
            14
        );
        assert_eq!(
            table.read(Color::Black, key, CorrectionTableKind::WhiteNonPawn),
            15
        );
        assert_eq!(
            table.read(
                Color::Black,
                key + CORRECTION_BASE_ENTRIES as u64,
                CorrectionTableKind::Material,
            ),
            11
        );
    }

    #[test]
    fn continuation_correction_index_orients_squares_and_uses_pre_and_post_move_kinds() {
        let context = MoveRecord {
            mv: Move {
                from: sq(2, 2),
                mid: None,
                to: sq(2, 3),
                promote: true,
            },
            piece: PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
            captured: None,
        };
        let previous = MoveRecord {
            mv: Move {
                from: sq(4, 4),
                mid: None,
                to: sq(4, 5),
                promote: true,
            },
            piece: PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
            captured: None,
        };

        // 文脈は着手前の歩兵0、1手前は成った後の金将17を使う。
        // 到達升は38と64で、後手番ではそれぞれ105と79へ回転する。
        assert_eq!(
            continuation_correction_index(Some(context), previous, Color::Black),
            161_200
        );
        assert_eq!(
            continuation_correction_index(Some(context), previous, Color::White),
            441_007
        );
        // 文脈なしは駒種29、升0を使い、この升0は後手番でも回転しない。
        assert_eq!(
            continuation_correction_index(None, previous, Color::Black),
            17_441_488
        );
        assert_eq!(
            continuation_correction_index(None, previous, Color::White),
            17_441_503
        );
    }

    #[test]
    fn correction_weighted_sum_matches_numeric_example() {
        let weighted_sum = correction_weighted_sum(
            CorrectionWeights {
                material: 277,
                pawn: 52,
                minor: 36,
                non_pawn: 44,
                continuation: 30,
                continuation_six: 24,
            },
            1024,
            -512,
            100,
            10,
            -10,
            ContinuationCorrectionValues::Present {
                two: 0,
                four: 0,
                six: 0,
            },
        );
        assert_eq!(weighted_sum, 260_624);
        assert_eq!(correction_from_weighted_sum(weighted_sum, 100), 254);
    }

    #[test]
    fn continuation_correction_weighted_sum_matches_numeric_example() {
        let weighted_sum = correction_weighted_sum(
            CorrectionWeights {
                material: 0,
                pawn: 0,
                minor: 0,
                non_pawn: 0,
                continuation: 30,
                continuation_six: 24,
            },
            0,
            0,
            0,
            0,
            0,
            ContinuationCorrectionValues::Present {
                two: 100,
                four: -50,
                six: 1024,
            },
        );
        assert_eq!(weighted_sum, 26_076);
        assert_eq!(
            correction_weighted_sum(
                CorrectionWeights {
                    material: 0,
                    pawn: 0,
                    minor: 0,
                    non_pawn: 0,
                    continuation: 30,
                    continuation_six: 24,
                },
                0,
                0,
                0,
                0,
                0,
                ContinuationCorrectionValues::NoPreviousMove,
            ),
            303
        );
    }

    #[test]
    fn correction_table_update_multipliers_match_numeric_example() {
        assert_eq!(scaled_update_bonus(100, MATERIAL_UPDATE_MULTIPLIER), 100);
        assert_eq!(scaled_update_bonus(100, PAWN_UPDATE_MULTIPLIER), 100);
        assert_eq!(scaled_update_bonus(100, MINOR_UPDATE_MULTIPLIER), 117);
        assert_eq!(scaled_update_bonus(100, NON_PAWN_UPDATE_MULTIPLIER), 145);
    }

    #[test]
    fn correction_key_depends_on_counts_owner_and_promotion_not_squares() {
        let a = position(Color::Black, &[(sq(1, 1), Color::Black, PieceKind::Pawn)]);
        let b = position(Color::White, &[(sq(8, 8), Color::Black, PieceKind::Pawn)]);
        assert_eq!(material_key(&a), material_key(&b));
        let c = position(
            Color::Black,
            &[
                (sq(1, 1), Color::Black, PieceKind::Pawn),
                (sq(8, 8), Color::Black, PieceKind::Pawn),
            ],
        );
        assert_ne!(material_key(&a), material_key(&c));
        let white = position(Color::Black, &[(sq(1, 1), Color::White, PieceKind::Pawn)]);
        assert_ne!(material_key(&a), material_key(&white));
        // 同じ駒種の金将でも、生駒と歩兵の成駒は別の駒コードである。
        let gold = PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted = PieceCode::new(Color::Black, PieceKind::Pawn)
            .unwrap()
            .promote()
            .unwrap();
        assert_ne!(piece_key(gold), piece_key(promoted));
    }

    #[test]
    fn correction_piece_key_sets_follow_current_kind_owner_state_and_square() {
        let square = sq(5, 5);
        let promoted_pawn = PieceCode::new(Color::Black, PieceKind::Pawn)
            .unwrap()
            .promote()
            .unwrap();
        let gold = PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted_position = position_from_codes(Color::Black, &[(square, promoted_pawn)]);
        let gold_position = position_from_codes(Color::Black, &[(square, gold)]);
        let promoted_keys = correction_piece_keys(&promoted_position);
        let gold_keys = correction_piece_keys(&gold_position);
        assert_ne!(promoted_keys.minor, 0, "成った歩兵はminorに入る");
        assert_ne!(promoted_keys.minor, gold_keys.minor);

        for piece in [
            PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
            PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
        ] {
            let board = position_from_codes(Color::Black, &[(square, piece)]);
            assert_eq!(correction_piece_keys(&board).minor, 0);
        }

        let king = position(Color::Black, &[(square, Color::White, PieceKind::King)]);
        let king_keys = correction_piece_keys(&king);
        assert_ne!(king_keys.non_pawn[Color::White.index()], 0);
        assert_eq!(king_keys.non_pawn[Color::Black.index()], 0);

        let pieces = [
            (sq(4, 4), Color::Black, PieceKind::SilverGeneral),
            (sq(7, 7), Color::White, PieceKind::Rook),
        ];
        assert_eq!(
            correction_piece_keys(&position(Color::Black, &pieces)),
            correction_piece_keys(&position(Color::White, &pieces)),
            "手番は配置鍵に含めない"
        );

        for color in Color::ALL {
            for kind in PieceKind::ALL {
                for piece in [
                    PieceCode::new(color, kind),
                    PieceCode::new_promoted(color, kind),
                ]
                .into_iter()
                .flatten()
                {
                    let board = position_from_codes(Color::Black, &[(square, piece)]);
                    let pawn = super::super::history::pawn_key(&board) != 0;
                    let non_pawn = correction_piece_keys(&board).non_pawn[color.index()] != 0;
                    assert_ne!(pawn, non_pawn, "{piece:?}");
                    assert_eq!(
                        pawn,
                        matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween)),
                        "{piece:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn correction_piece_keys_match_explicit_promotions_captures_and_lion_moves() {
        let promotion = |from, to| Move {
            from,
            mid: None,
            to,
            promote: true,
        };
        for (name, kind, mv) in [
            ("歩兵の成り", PieceKind::Pawn, promotion(sq(4, 7), sq(4, 8))),
            (
                "仲人の成り",
                PieceKind::GoBetween,
                promotion(sq(4, 7), sq(4, 8)),
            ),
            (
                "醉象の成り",
                PieceKind::DrunkElephant,
                promotion(sq(4, 7), sq(4, 8)),
            ),
            (
                "麒麟の成り",
                PieceKind::Kirin,
                promotion(sq(4, 7), sq(5, 8)),
            ),
            (
                "銀将の成り",
                PieceKind::SilverGeneral,
                promotion(sq(4, 7), sq(4, 8)),
            ),
        ] {
            assert_incremental_piece_keys(
                name,
                &board_with_kings(vec![(mv.from, Color::Black, kind)]),
                mv,
            );
        }

        let capture = Move {
            from: sq(4, 4),
            mid: None,
            to: sq(4, 9),
            promote: false,
        };
        assert_incremental_piece_keys(
            "小駒の捕獲",
            &board_with_kings(vec![
                (capture.from, Color::Black, PieceKind::Rook),
                (capture.to, Color::White, PieceKind::GoldGeneral),
            ]),
            capture,
        );

        let igui = Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(5, 5),
            promote: false,
        };
        assert_incremental_piece_keys(
            "獅子の居喰い",
            &board_with_kings(vec![
                (igui.from, Color::Black, PieceKind::Lion),
                (igui.mid.unwrap(), Color::White, PieceKind::GoldGeneral),
            ]),
            igui,
        );

        let jitto = Move {
            from: sq(5, 5),
            mid: None,
            to: sq(5, 5),
            promote: false,
        };
        assert_incremental_piece_keys(
            "獅子のじっと",
            &board_with_kings(vec![(jitto.from, Color::Black, PieceKind::Lion)]),
            jitto,
        );

        let double_capture = Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(5, 7),
            promote: false,
        };
        assert_incremental_piece_keys(
            "獅子の2枚取り",
            &board_with_kings(vec![
                (double_capture.from, Color::Black, PieceKind::Lion),
                (
                    double_capture.mid.unwrap(),
                    Color::White,
                    PieceKind::GoldGeneral,
                ),
                (double_capture.to, Color::White, PieceKind::SilverGeneral),
            ]),
            double_capture,
        );
    }

    #[test]
    fn correction_incremental_keys_match_seeded_playouts_and_undo() {
        let rules = MoveRules::standard();
        let generator = MoveGenerator::new(rules);
        let mut rng =
            minase_core::rng::XorShift64::new(core::num::NonZeroU64::new(0x1234_5678).unwrap());
        let mut starts = minase_core::test_util::bench_positions();
        starts.push(position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::Pawn),
            ],
        ));
        let mut captures = 0;
        let mut promotions = 0;
        let mut doubles = 0;
        for mut board in starts {
            let mut material = material_key(&board);
            let mut placement = correction_piece_keys(&board);
            for _ in 0..64 {
                let null = board.make_null_move();
                assert_eq!(material, material_key(&board));
                assert_eq!(placement, correction_piece_keys(&board));
                board.unmake_null_move(null);
                let mut moves = Vec::new();
                generator.generate_moves(&board, &mut moves);
                if moves.is_empty() {
                    break;
                }
                // 各局面の全候補も照合し、稀な2枚取りと成りを確実に含める。
                for &mv in &moves {
                    let undo = board.make_move_unchecked(mv, rules);
                    let material_after = key_after_move(material, &undo);
                    let placement_after = correction_piece_keys_after_move(placement, &undo);
                    let taken = undo.captured.iter().flatten().count();
                    captures += usize::from(taken > 0);
                    doubles += usize::from(taken == 2);
                    promotions += usize::from(mv.promote);
                    assert_eq!(material_after, material_key(&board), "{mv:?}");
                    assert_eq!(placement_after, correction_piece_keys(&board), "{mv:?}");
                    board.unmake_move(undo);
                    assert_eq!(material, material_key(&board));
                    assert_eq!(placement, correction_piece_keys(&board));
                }
                let mv = moves[rng.next() as usize % moves.len()];
                let undo = board.make_move_unchecked(mv, rules);
                material = key_after_move(material, &undo);
                placement = correction_piece_keys_after_move(placement, &undo);
                assert_eq!(material, material_key(&board));
                assert_eq!(placement, correction_piece_keys(&board));
            }
        }
        assert!(captures > 0 && promotions > 0 && doubles > 0);
    }
}

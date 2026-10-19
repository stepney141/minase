//! 駒種別の枚数を鍵にした、探索ワーカー内の静的評価の補正（correction history）。
//!
//! 手番側ごとに駒種別枚数のハッシュで表を引き、補正値を通常探索の各ノードの静的評価に加える。
//! 補正後の静的評価は、静的評価を使う前向き枝刈りの判定のすべてに使う（Stockfish src/search.cpp:832, 846-870）。
//! 鍵を選んだ理由は`docs/adr/key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md`にある。

use super::history::history_gravity;
use super::params;
use super::royal::royal_under_attack;
use super::searcher::Searcher;
use super::tt::Bound;
use crate::eval::{PIECE_STATE_COUNT, piece_state_of};
use crate::search::REPETITION_BAND_START;
use minase_core::piece::{COLOR_COUNT, Color, PieceCode};
use minase_core::position::Undo;
use minase_core::{Position, Square};

impl Searcher<'_> {
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
            self.correction.update(
                position.side_to_move(),
                self.material_keys[ply as usize],
                bonus,
                MATERIAL_UPDATE_MULTIPLIER,
            );
        }
    }
}

const ENTRY_COUNT: usize = 4096;
/// 補正表の要素と表ごとの更新量の絶対値の上限。
/// Stockfishと同じ1024を使う（src/history.h:41）。
pub(super) const CORRECTION_LIMIT: i32 = 1024;
/// 材料表の初期値。Stockfishの局面を鍵にした補正表と同じ−5を使う（src/search.cpp:719）。
pub(super) const CORRECTION_INITIAL: i16 = -5;
/// 材料表の更新倍率。Stockfishで基準となるpawnの表と同じ128/128を使う。
const MATERIAL_UPDATE_MULTIPLIER: i32 = 128;
const UPDATE_MULTIPLIER_DENOMINATOR: i64 = 128;
const CORRECTION_DENOMINATOR: i64 = 102_400;
const UPDATE_BASE_LIMIT: i64 = 256;
const UPDATE_SCALE_NUMERATOR: i64 = 1061;
const UPDATE_SCALE_DENOMINATOR: i64 = 1024;

/// 手番側ごとの材料表の値。
pub(super) type CorrectionValues = [[i16; ENTRY_COUNT]; COLOR_COUNT];

/// 全要素が−5の材料表を作る。
pub(super) fn new_correction_values() -> Box<CorrectionValues> {
    Box::new([[CORRECTION_INITIAL; ENTRY_COUNT]; COLOR_COUNT])
}

/// ワーカー別の材料表を扱う。
pub(super) struct CorrectionTable<'a> {
    values: &'a mut CorrectionValues,
}

impl<'a> CorrectionTable<'a> {
    pub(super) fn new(values: &'a mut CorrectionValues) -> Self {
        Self { values }
    }

    /// 手番側と鍵の下位12ビットで材料表を引く。
    pub(super) fn read(&self, side: Color, key: u64) -> i32 {
        i32::from(self.values[side.index()][key as usize & (ENTRY_COUNT - 1)])
    }

    /// 表ごとの倍率を掛けた更新量を、同じ添字の要素へ重力式で取り込む。
    pub(super) fn update(&mut self, side: Color, key: u64, bonus: i32, multiplier: i32) {
        let value = &mut self.values[side.index()][key as usize & (ENTRY_COUNT - 1)];
        *value = updated_correction(*value, bonus, multiplier);
    }
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
    let bonus = (i64::from(bonus) * i64::from(multiplier) / UPDATE_MULTIPLIER_DENOMINATOR)
        .clamp(i64::from(-CORRECTION_LIMIT), i64::from(CORRECTION_LIMIT)) as i32;
    history_gravity(i32::from(value), bonus, CORRECTION_LIMIT) as i16
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

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::rules::MoveRules;
    use minase_core::test_util::{position, sq};
    use minase_core::{MoveGenerator, PieceKind};

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

        let mut values = [[CORRECTION_INITIAL; ENTRY_COUNT]; COLOR_COUNT];
        let mut table = CorrectionTable::new(&mut values);
        assert_eq!(table.read(Color::Black, 7), -5);
        assert_eq!(table.read(Color::White, 7), -5);
        table.update(Color::Black, 7, 31, 128);
        table.update(Color::White, 7, -31, 128);
        assert_eq!(table.read(Color::Black, 7), 26);
        assert_eq!(table.read(Color::White, 7), -36);
        assert_eq!(table.read(Color::Black, 4096 + 7), 26);
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
            let mut key = material_key(&board);
            for _ in 0..64 {
                let null = board.make_null_move();
                assert_eq!(key, material_key(&board));
                board.unmake_null_move(null);
                let mut moves = Vec::new();
                generator.generate_moves(&board, &mut moves);
                if moves.is_empty() {
                    break;
                }
                // 各局面の全候補も照合し、稀な2枚取りと成りを確実に含める。
                for &mv in &moves {
                    let undo = board.make_move_unchecked(mv, rules);
                    let after = key_after_move(key, &undo);
                    let taken = undo.captured.iter().flatten().count();
                    captures += usize::from(taken > 0);
                    doubles += usize::from(taken == 2);
                    promotions += usize::from(mv.promote);
                    assert_eq!(after, material_key(&board), "{mv:?}");
                    board.unmake_move(undo);
                    assert_eq!(key, material_key(&board));
                }
                let mv = moves[rng.next() as usize % moves.len()];
                let undo = board.make_move_unchecked(mv, rules);
                key = key_after_move(key, &undo);
                assert_eq!(key, material_key(&board));
            }
        }
        assert!(captures > 0 && promotions > 0 && doubles > 0);
    }
}

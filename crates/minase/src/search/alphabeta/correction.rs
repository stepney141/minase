//! 駒種別の枚数を鍵にした、探索ワーカー内の静的評価の補正（correction history）。
//!
//! 手番側ごとに駒種別枚数のハッシュで表を引き、補正値をfutility pruningの判定にだけ加える。
//! 鍵と適用範囲を選んだ理由は`docs/adr/key-correction-history-by-material.md`にある。

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
    /// 詰み帯と専用の帯の外で、最善手が非捕獲手、かつ王駒に疑似利きが届かないノードだけを学習する。
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
        mut static_eval: Option<i32>,
        mut royal_attacked: Option<bool>,
    ) {
        // 正確な値、静的評価より低い上界、静的評価より高い下界なら補正の向きが確定する。
        // 捕獲による駒得は静的評価の偏りではないため除外する。理由は
        // docs/adr/key-correction-history-by-material.md にある。
        if best_score.abs() < REPETITION_BAND_START && !best_capture {
            let static_eval = *static_eval.get_or_insert_with(|| {
                #[cfg(feature = "invariants")]
                self.nnue
                    .assert_accumulator(position, &self.accumulators[ply as usize], ply);
                self.nnue
                    .evaluate_accumulator(&self.accumulators[ply as usize], position.side_to_move())
            });
            if (bound == Bound::Exact
                || (bound == Bound::Upper && best_score < static_eval)
                || (bound == Bound::Lower && best_score > static_eval))
                && !*royal_attacked.get_or_insert_with(|| royal_under_attack(position))
            {
                self.correction.update(
                    position.side_to_move(),
                    self.material_keys[ply as usize],
                    best_score - static_eval,
                    depth,
                );
            }
        }
    }
}

const ENTRY_COUNT: usize = 4096;
const SCALE: i64 = 1024;

/// 手番側ごとの固定小数点の補正値。探索の生成時だけ初期化する。
pub(super) struct CorrectionTable {
    values: Box<[[i64; ENTRY_COUNT]; COLOR_COUNT]>,
    cap: i64,
}

impl CorrectionTable {
    /// 歩兵価値と`CorrectionCap`の百分率から補正上限を求め、全要素を0にする。
    /// 各ワーカーの探索開始時に作り、反復深化の間だけ保持する。
    pub(super) fn new(pawn: i32) -> Self {
        Self {
            values: Box::new([[0; ENTRY_COUNT]; COLOR_COUNT]),
            cap: i64::from(pawn) * i64::from(params::correction_cap()) / 100,
        }
    }

    /// 手番側と鍵の下位12ビットで表を引き、静的評価と同じ単位の補正値を返す。
    /// 1024倍の固定小数点値を0方向へ整数除算し、歩兵価値×`CorrectionCap`%の正負の範囲へ収める。
    pub(super) fn read(&self, side: Color, key: u64) -> i32 {
        (self.values[side.index()][key as usize & (ENTRY_COUNT - 1)] / SCALE)
            .clamp(-self.cap, self.cap) as i32
    }

    /// 探索値と補正前の静的評価の差を、同じ添字の指数移動平均へ取り込む。
    ///
    /// 固定小数点値をF、差をDとすると `F += (D × 1024 − F) × min(depth, 8) × CorrectionWeight / 1024` で更新する。
    /// 重みは `min(depth, 8) × CorrectionWeight / 1024` であり、積の後の整数除算は0方向へ丸める。
    /// 更新後のFは、歩兵価値×`CorrectionCap`%の1024倍を正負の上限として切り詰める。
    pub(super) fn update(&mut self, side: Color, key: u64, difference: i32, depth: u32) {
        let fixed = &mut self.values[side.index()][key as usize & (ENTRY_COUNT - 1)];
        *fixed += (i64::from(difference) * SCALE - *fixed)
            * i64::from(depth.min(8))
            * i64::from(params::correction_weight())
            / 1024;
        *fixed = (*fixed).clamp(-self.cap * SCALE, self.cap * SCALE);
    }
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
    fn correction_fixed_point_weights_rounding_caps_and_sides() {
        let mut table = CorrectionTable::new(100);
        let weight = i64::from(params::correction_weight());
        // 初回は1未満、2回目は1以上となる差を選び、正負とも端数を保持する。
        let difference = (1023 / weight) as i32;
        assert_eq!(table.read(Color::Black, 7), 0);
        table.update(Color::Black, 7, difference, 1);
        assert_eq!(table.read(Color::Black, 7), 0);
        table.update(Color::Black, 7, difference, 1);
        assert_eq!(table.read(Color::Black, 7), 1);
        table.update(Color::White, 7, -difference, 1);
        assert_eq!(table.read(Color::White, 7), 0);
        table.update(Color::White, 7, -difference, 1);
        assert_eq!(table.read(Color::White, 7), -1);
        for depth in [8, 9, 256] {
            table.update(Color::Black, depth as u64, 100, depth);
            assert_eq!(
                table.read(Color::Black, depth as u64),
                (100 * 8 * weight / 1024) as i32
            );
        }
        table.update(Color::Black, 0, 100_000, 1);
        table.update(Color::White, 0, -100_000, 1);
        let cap = params::correction_cap();
        assert_eq!(table.read(Color::Black, 0), cap);
        assert_eq!(table.read(Color::White, 0), -cap);
        // 下位12ビットだけで引く。
        assert_eq!(table.read(Color::Black, 4096), cap);
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

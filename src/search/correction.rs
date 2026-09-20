//! 駒種別の枚数を鍵にした、探索ワーカー内の静的評価の補正。

use crate::core::mv::Undo;
use crate::core::piece::{COLOR_COUNT, Color, PieceCode};
use crate::eval::pst::{PIECE_STATE_COUNT, piece_state_of};
use crate::{Position, Square};

const ENTRY_COUNT: usize = 4096;
const SCALE: i64 = 1024;

/// 手番側ごとの固定小数点の補正値。探索の生成時だけ初期化する。
pub(super) struct CorrectionTable {
    values: Box<[[i64; ENTRY_COUNT]; COLOR_COUNT]>,
    cap: i64,
}

impl CorrectionTable {
    pub(super) fn new(pawn: i32) -> Self {
        Self {
            values: Box::new([[0; ENTRY_COUNT]; COLOR_COUNT]),
            cap: 2 * i64::from(pawn),
        }
    }

    pub(super) fn read(&self, side: Color, key: u64) -> i32 {
        (self.values[side.index()][key as usize & (ENTRY_COUNT - 1)] / SCALE)
            .clamp(-self.cap, self.cap) as i32
    }

    pub(super) fn update(&mut self, side: Color, key: u64, difference: i32, depth: u32) {
        let fixed = &mut self.values[side.index()][key as usize & (ENTRY_COUNT - 1)];
        *fixed += (i64::from(difference) * SCALE - *fixed) * i64::from(depth.min(8)) / 32;
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
    use crate::core::rules::MoveRules;
    use crate::test_util::{position, sq};
    use crate::{MoveGenerator, PieceKind};

    #[test]
    fn correction_fixed_point_weights_rounding_caps_and_sides() {
        let mut table = CorrectionTable::new(100);
        assert_eq!(table.read(Color::Black, 7), 0);
        table.update(Color::Black, 7, 31, 1);
        assert_eq!(table.values[0][7], 992);
        assert_eq!(table.read(Color::Black, 7), 0);
        table.update(Color::Black, 7, 31, 1);
        assert_eq!(table.values[0][7], 1953);
        assert_eq!(table.read(Color::Black, 7), 1);
        table.update(Color::White, 7, -31, 1);
        assert_eq!(table.values[1][7], -992);
        assert_eq!(table.read(Color::White, 7), 0);
        table.update(Color::White, 7, -31, 1);
        assert_eq!(table.values[1][7], -1953);
        assert_eq!(table.read(Color::White, 7), -1);
        for depth in [8, 9, 256] {
            table.update(Color::Black, depth as u64, 100, depth);
            assert_eq!(table.read(Color::Black, depth as u64), 25);
        }
        table.update(Color::Black, 0, 100_000, 1);
        table.update(Color::White, 0, -100_000, 1);
        assert_eq!(table.read(Color::Black, 0), 200);
        assert_eq!(table.read(Color::White, 0), -200);
        assert_eq!(table.values[0][0], 204_800);
        assert_eq!(table.values[1][0], -204_800);
        // 下位12ビットだけで引く。
        assert_eq!(table.read(Color::Black, 4096), 200);
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
        let mut rng = crate::rng::XorShift64::new(core::num::NonZeroU64::new(0x1234_5678).unwrap());
        let mut starts = crate::test_util::bench_positions();
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

//! 利き前計算テーブルの構築と参照。

use std::sync::OnceLock;

use crate::core::bitboard::Bitboard;
use crate::core::direction::Direction;
use crate::core::piece::{COLOR_COUNT, Color};
use crate::core::square::{RAW_SQUARE_COUNT, Square};

use super::fixed::{
    MOVEMENT_PROFILE_COUNT, MovementProfileId, all_profiles, movement_profile_data,
};
use super::sliding::{RayTable, build_ray_table};

/// 色×プロファイル×升ごとの固定利きテーブル。
type FixedAttackTable = Box<[Bitboard]>;

/// 全駒種の利き計算に使う前計算テーブル一式。
pub(crate) struct AttackTables {
    /// 方向×升ごとの盤端までの利き線。
    rays: Box<RayTable>,
    /// 色×プロファイル×升ごとの固定利き。
    fixed: FixedAttackTable,
    /// 遮蔽のない盤面での固定利きと走りの和。
    reach: FixedAttackTable,
    /// 色×プロファイルごとの走りの絶対方向マスク。
    slide_directions: [[u8; MOVEMENT_PROFILE_COUNT]; COLOR_COUNT],
    /// 各升を中心とする5×5の近傍。固定利きの逆引きに使う。
    neighbourhoods: [Bitboard; RAW_SQUARE_COUNT],
    /// 各升から獅子が直接跳べる2升先の集合(第12条第5項・第6項)。
    lion_jumps: [Bitboard; RAW_SQUARE_COUNT],
}

impl AttackTables {
    /// 全テーブルを構築する。
    fn build() -> Self {
        let rays = Box::new(build_ray_table());
        let mut fixed =
            vec![Bitboard::EMPTY; COLOR_COUNT * MOVEMENT_PROFILE_COUNT * RAW_SQUARE_COUNT]
                .into_boxed_slice();

        let mut slide_directions = [[0; MOVEMENT_PROFILE_COUNT]; COLOR_COUNT];
        for color in Color::ALL {
            for profile_id in all_profiles() {
                let profile = movement_profile_data(profile_id);
                for slide in profile.slides {
                    slide_directions[color.index()][profile_id.index()] |=
                        1 << slide.direction.for_color(color).index();
                }
                // movegen-speedup-2.md「段階4」: 固定利きは5×5近傍に収まる。
                for delta in profile.fixed_deltas {
                    let (file, rank) = delta.for_color(color);
                    assert!(file.abs() <= 2 && rank.abs() <= 2);
                }
                for from in Square::all() {
                    let mut mask = Bitboard::EMPTY;
                    for delta in profile.fixed_deltas {
                        let (file, rank) = delta.for_color(color);
                        if let Some(to) = from.offset(file, rank) {
                            mask.set(to);
                        }
                    }
                    fixed[Self::fixed_index(color, profile_id, from)] = mask;
                }
            }
        }

        let mut neighbourhoods = [Bitboard::EMPTY; RAW_SQUARE_COUNT];
        let mut lion_jumps = [Bitboard::EMPTY; RAW_SQUARE_COUNT];
        for from in Square::all() {
            for file_delta in -2_i8..=2 {
                for rank_delta in -2_i8..=2 {
                    if let Some(to) = from.offset(file_delta, rank_delta) {
                        neighbourhoods[from.raw_index()].set(to);
                        if file_delta.abs().max(rank_delta.abs()) == 2 {
                            lion_jumps[from.raw_index()].set(to);
                        }
                    }
                }
            }
        }

        let reach = fixed.clone();
        let mut tables = Self {
            reach,
            rays,
            fixed,
            slide_directions,
            lion_jumps,
            neighbourhoods,
        };
        for color in Color::ALL {
            for profile_id in all_profiles() {
                for from in Square::all() {
                    let index = Self::fixed_index(color, profile_id, from);
                    for slide in movement_profile_data(profile_id).slides {
                        tables.reach[index] |= tables.sliding_control(
                            from,
                            slide.direction.for_color(color),
                            Bitboard::EMPTY,
                        );
                    }
                }
            }
        }
        tables
    }

    /// 固定利きテーブルの添字を計算して返す。
    #[inline]
    const fn fixed_index(color: Color, profile: MovementProfileId, from: Square) -> usize {
        (color.index() * MOVEMENT_PROFILE_COUNT + profile.index()) * RAW_SQUARE_COUNT
            + from.raw_index()
    }

    /// 走りの絶対方向を8ビットのマスクで返す。
    /// `movegen-speedup-2.md`「段階4」の方向マスクの表引きに使う。
    #[inline]
    pub(crate) fn slide_directions(&self, color: Color, profile: MovementProfileId) -> u8 {
        self.slide_directions[color.index()][profile.index()]
    }

    /// 指定した色・プロファイル・升の固定利きを返す。
    #[inline]
    pub(crate) fn fixed(&self, color: Color, profile: MovementProfileId, from: Square) -> Bitboard {
        self.fixed[Self::fixed_index(color, profile, from)]
    }

    /// 遮蔽を考慮しない固定利きと走りの到達範囲を返す。
    #[inline]
    pub(crate) fn reach(&self, color: Color, profile: MovementProfileId, from: Square) -> Bitboard {
        self.reach[Self::fixed_index(color, profile, from)]
    }

    /// 周囲8升(王将の1升移動の範囲)を返す。獅子の第1段階の判定にも使う。
    #[inline]
    pub(crate) fn king_steps(&self, from: Square) -> Bitboard {
        self.fixed(
            Color::Black,
            super::fixed::movement_profile(crate::core::piece::PieceKind::King),
            from,
        )
    }

    /// 対象升を中心とする盤内の5×5近傍を返す。
    /// `movegen-speedup-2.md`「段階4」の固定利きの逆引きに使う。
    #[inline]
    pub(crate) fn neighbourhood(&self, square: Square) -> Bitboard {
        self.neighbourhoods[square.raw_index()]
    }

    /// 獅子が直接跳べる2升先の集合を返す。
    #[inline]
    pub(crate) fn lion_jumps(&self, from: Square) -> Bitboard {
        self.lion_jumps[from.raw_index()]
    }

    /// 指定方向の盤端までの利き線を返す。
    #[inline]
    pub(crate) fn ray(&self, from: Square, direction: Direction) -> Bitboard {
        self.rays[direction.index()][from.raw_index()]
    }

    /// 遮る駒を考慮した走りの利きを返す。進行方向の最初の駒がある升までを含む
    /// (第7条第4項・第5項)。
    pub(crate) fn sliding_control(
        &self,
        from: Square,
        direction: Direction,
        occupied: Bitboard,
    ) -> Bitboard {
        let candidates = self.ray(from, direction);
        let blockers = candidates & occupied;
        if direction.increases_raw_index() {
            // movegen-speedup-2.md「段階4」: 3語を192ビット整数として1を引く。
            // 遮蔽がない場合は全ビットが立ち、利き線全体が残る。
            let b = blockers.words();
            let (low, borrow) = b[0].overflowing_sub(1);
            let (middle, borrow) = b[1].overflowing_sub(u64::from(borrow));
            let (high, _) = b[2].overflowing_sub(u64::from(borrow));
            candidates & Bitboard::from_words([b[0] ^ low, b[1] ^ middle, b[2] ^ high])
        } else {
            match blockers.msb() {
                None => candidates,
                Some(blocker) => candidates & !self.rays[direction.index()][blocker.raw_index()],
            }
        }
    }
}

static ATTACK_TABLES: OnceLock<AttackTables> = OnceLock::new();

/// プロセス全体で共有する利きテーブルを返す。初回呼び出し時に構築する。
pub(crate) fn attack_tables() -> &'static AttackTables {
    ATTACK_TABLES.get_or_init(AttackTables::build)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::direction::step_square;

    // movegen-speedup.md「捕獲対象を生成前に除外する」: 遮蔽なしの範囲は
    // 全色・全プロファイル・全升で固定利きと盤端までの走りの和になる。
    #[test]
    fn reach_matches_fixed_and_unblocked_slides() {
        let tables = attack_tables();
        for color in Color::ALL {
            for profile in all_profiles() {
                for from in Square::all() {
                    let mut expected = tables.fixed(color, profile, from);
                    for slide in movement_profile_data(profile).slides {
                        expected |= tables.sliding_control(
                            from,
                            slide.direction.for_color(color),
                            Bitboard::EMPTY,
                        );
                    }
                    assert_eq!(tables.reach(color, profile, from), expected);
                }
            }
        }
    }

    // 実装契約(第7条4項・5項の走りの定義に接地): 走りの利きは、方向へ1升ずつ進む
    // 逐次歩行と一致する。盤端までの升を進行順に含み、最初の駒がある升を含んだ
    // 直後に打ち切られ、その先の升を含まない。
    #[test]
    fn sliding_control_matches_a_stepwise_walk_with_blockers() {
        /// 第7条4項・5項の文言どおりに、1升ずつ進んで期待利きを構成する対照実装。
        fn walk(from: Square, direction: Direction, occupied: Bitboard) -> Bitboard {
            let mut expected = Bitboard::EMPTY;
            let mut current = from;
            while let Some(next) = step_square(current, direction) {
                expected.set(next);
                if occupied.contains(next) {
                    break; // 走り駒は進行方向の最初の駒を越えられない(第7条4項)。
                }
                current = next;
            }
            expected
        }

        // 段階4: 全始点・全方向と単升遮蔽の全組合せで語境界も覆う。
        // 空盤・全集合・対角線・擬似乱数集合で遮蔽なしと複数遮蔽を調べる。
        let diagonal =
            Bitboard::from_squares(Square::all().filter(|square| square.file() == square.rank()));
        let mut occupancies = vec![Bitboard::EMPTY, Bitboard::FULL, diagonal];
        occupancies.extend(Square::all().map(Bitboard::from_square));
        let mut state = 0x5a4f_4252_4953_5401_u64;
        for _ in 0..8 {
            occupancies.push(Bitboard::from_squares(Square::all().filter(|_| {
                state = state
                    .wrapping_mul(2_862_933_555_777_941_757)
                    .wrapping_add(3_037_000_493);
                state >> 62 == 0
            })));
        }

        let tables = attack_tables();
        for occupied in occupancies {
            for from in Square::all() {
                for direction in Direction::ALL {
                    assert_eq!(
                        tables.sliding_control(from, direction, occupied),
                        walk(from, direction, occupied),
                        "from {:?} {direction:?}",
                        (from.file(), from.rank())
                    );
                }
            }
        }
    }
}

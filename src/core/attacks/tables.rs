//! 利き前計算テーブルの構築と参照。

use std::sync::OnceLock;

use crate::core::bitboard::Bitboard;
use crate::core::direction::{DIRECTION_COUNT, Direction, step_square};
use crate::core::piece::{COLOR_COUNT, Color};
use crate::core::square::{BOARD_RANKS, RAW_SQUARE_COUNT, Square};

use super::fixed::{
    MOVEMENT_PROFILE_COUNT, MovementProfileId, all_profiles, movement_profile_data,
};
use super::sliding::{RayTable, build_ray_table};

/// 色×プロファイル×升ごとの固定利きテーブル。
type FixedAttackTable = Box<[Bitboard]>;

/// 走りの距離制限の段階数(盤の段数と同じ12)。
const RANGE_COUNT: usize = BOARD_RANKS as usize;
/// 方向×升×距離ごとの範囲マスクテーブル。
type RangeMaskTable = Box<[Bitboard]>;

/// 全駒種の利き計算に使う前計算テーブル一式。
pub(crate) struct AttackTables {
    /// 方向×升ごとの盤端までの利き線。
    rays: Box<RayTable>,
    /// 色×プロファイル×升ごとの固定利き。
    fixed: FixedAttackTable,
    /// 距離制限付き走りに使う範囲マスク。
    range_masks: RangeMaskTable,
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

        for color in Color::ALL {
            for profile_id in all_profiles() {
                let profile = movement_profile_data(profile_id);
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

        let mut range_masks =
            vec![Bitboard::EMPTY; DIRECTION_COUNT * RAW_SQUARE_COUNT * RANGE_COUNT]
                .into_boxed_slice();
        for direction in Direction::ALL {
            for from in Square::all() {
                let mut current = from;
                let mut mask = Bitboard::EMPTY;
                for distance in 0..RANGE_COUNT {
                    if let Some(next) = step_square(current, direction) {
                        mask.set(next);
                        current = next;
                    }
                    range_masks[Self::range_index(direction, from, distance)] = mask;
                }
            }
        }

        let mut lion_jumps = [Bitboard::EMPTY; RAW_SQUARE_COUNT];
        for from in Square::all() {
            for file_delta in -2_i8..=2 {
                for rank_delta in -2_i8..=2 {
                    if file_delta.abs().max(rank_delta.abs()) == 2
                        && let Some(to) = from.offset(file_delta, rank_delta)
                    {
                        lion_jumps[from.raw_index()].set(to);
                    }
                }
            }
        }

        Self {
            rays,
            fixed,
            range_masks,
            lion_jumps,
        }
    }

    /// 固定利きテーブルの添字を計算して返す。
    #[inline]
    const fn fixed_index(color: Color, profile: MovementProfileId, from: Square) -> usize {
        (color.index() * MOVEMENT_PROFILE_COUNT + profile.index()) * RAW_SQUARE_COUNT
            + from.raw_index()
    }

    /// 範囲マスクテーブルの添字を計算して返す。
    #[inline]
    const fn range_index(direction: Direction, from: Square, distance_index: usize) -> usize {
        (direction.index() * RAW_SQUARE_COUNT + from.raw_index()) * RANGE_COUNT + distance_index
    }

    /// 指定した色・プロファイル・升の固定利きを返す。
    #[inline]
    pub(crate) fn fixed(&self, color: Color, profile: MovementProfileId, from: Square) -> Bitboard {
        self.fixed[Self::fixed_index(color, profile, from)]
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

    /// 獅子が直接跳べる2升先の集合を返す。
    #[inline]
    pub(crate) fn lion_jumps(&self, from: Square) -> Bitboard {
        self.lion_jumps[from.raw_index()]
    }

    /// 指定方向の盤端までの利き線を返す。
    #[inline]
    fn ray(&self, from: Square, direction: Direction) -> Bitboard {
        self.rays[direction.index()][from.raw_index()]
    }

    /// 距離制限と遮る駒を考慮した走りの利きを返す。進行方向の最初の駒がある升までを含む
    /// (第7条第4項・第5項)。
    pub(crate) fn sliding_control(
        &self,
        from: Square,
        direction: Direction,
        max_steps: Option<u8>,
        occupied: Bitboard,
    ) -> Bitboard {
        let full_ray = self.ray(from, direction);
        let candidates = match max_steps {
            None => full_ray,
            Some(0) => Bitboard::EMPTY,
            Some(steps) => {
                let index = usize::from(steps.min(BOARD_RANKS) - 1);
                full_ray & self.range_masks[Self::range_index(direction, from, index)]
            }
        };
        let blockers = candidates & occupied;
        let first = if direction.increases_raw_index() {
            blockers.lsb()
        } else {
            blockers.msb()
        };
        match first {
            None => candidates,
            Some(blocker) => candidates & !self.rays[direction.index()][blocker.raw_index()],
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
    use crate::core::attacks::movement_profile;
    use crate::core::piece::PieceKind;

    #[test]
    fn fixed_masks_rotate_for_white() {
        let tables = attack_tables();
        for kind in PieceKind::ALL {
            let profile = movement_profile(kind);
            for from in Square::all() {
                let rotated_from = Square::new(11 - from.file(), 11 - from.rank()).unwrap();
                let rotated_black = Bitboard::from_squares(
                    tables
                        .fixed(Color::Black, profile, from)
                        .into_iter()
                        .map(|square| Square::new(11 - square.file(), 11 - square.rank()).unwrap()),
                );
                assert_eq!(
                    rotated_black,
                    tables.fixed(Color::White, profile, rotated_from)
                );
            }
        }
    }

    #[test]
    fn limited_slides_obey_range_and_blockers() {
        let tables = attack_tables();
        let from = Square::new(4, 4).unwrap();
        let blocker = Square::new(4, 6).unwrap();
        let occupied = Bitboard::from_square(blocker);
        let actual = tables.sliding_control(from, Direction::North, Some(3), occupied);
        assert_eq!(
            actual,
            Bitboard::from_squares([Square::new(4, 5).unwrap(), blocker,])
        );
    }
}

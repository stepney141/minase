//! 利き前計算テーブルの構築と参照。

use std::sync::OnceLock;

use crate::board::Bitboard;
use crate::board::Direction;
use crate::board::{RAW_SQUARE_COUNT, Square};
use crate::piece::{COLOR_COUNT, Color, PieceCode, PieceKind};

use super::fixed::{
    MOVEMENT_PROFILE_COUNT, MovementProfileId, all_profiles, movement_profile,
    movement_profile_data,
};
use super::sliding::{RayTable, build_ray_table};

/// 色×プロファイル×升ごとの固定利きテーブル。
type FixedAttackTable = Box<[Bitboard]>;

/// 全駒種の利き計算に使う前計算テーブル一式。
/// 駒コードで表を引く理由は`docs/adr/decode-piece-codes-through-256-entry-tables.md`にある。
pub(crate) struct AttackTables {
    /// 方向×升ごとの盤端までの利き線。
    rays: Box<RayTable>,
    /// 色×プロファイル×升ごとの固定利き。
    fixed: FixedAttackTable,
    /// 5×5近傍の相対升から固定利きが届く駒種の集合。添字は生値の差に34を足した値。
    fixed_attacker_kinds: [[u32; 69]; COLOR_COUNT],
    /// 遮蔽のない盤面での固定利きと走りの和。
    reach: FixedAttackTable,
    /// 照会した対局者ごとの走りの絶対方向マスクであり、各行は`PieceCode::table_index`で引く。
    /// 構築できない駒コードの欄は0であり、照会した対局者の駒でないコードにも0を返して所属の検査を兼ねる。
    slide_mask_by_code: [[u8; 256]; COLOR_COUNT],
    /// `PieceCode::table_index`で引く駒種ビットの表であり、駒を表すコードとして構築できない欄は0である。
    kind_bit_by_code: [u32; 256],
    /// 各升を中心とする5×5の近傍。固定利きの逆引きに使う。
    neighbourhoods: [Bitboard; RAW_SQUARE_COUNT],
    /// 対象升への利きが依存する8方向の利き線と5×5近傍の和集合。
    attack_dependencies: [Bitboard; RAW_SQUARE_COUNT],
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

        let mut fixed_attacker_kinds = [[0; 69]; COLOR_COUNT];
        let mut slide_mask_by_code = [[0; 256]; COLOR_COUNT];
        let mut kind_bit_by_code = [0; 256];
        for color in Color::ALL {
            let mut profile_slide_masks = [0; MOVEMENT_PROFILE_COUNT];
            for profile_id in all_profiles() {
                let profile = movement_profile_data(profile_id);
                for slide in profile.slides {
                    profile_slide_masks[profile_id.index()] |=
                        1 << slide.direction.for_color(color).index();
                }
                // neighbourhoodによる逆引きの前提として、固定利きが5×5近傍に収まることを検査する。
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
            for kind in PieceKind::ALL {
                let profile_id = movement_profile(kind);
                let kind_bit = 1 << kind.index();
                for delta in movement_profile_data(profile_id).fixed_deltas {
                    let (file, rank) = delta.for_color(color);
                    let index = (i16::from(rank) * 16 + i16::from(file) + 34) as usize;
                    fixed_attacker_kinds[color.index()][index] |= kind_bit;
                }
                for code in [
                    PieceCode::new(color, kind),
                    PieceCode::new_promoted(color, kind),
                ]
                .into_iter()
                .flatten()
                {
                    let index = code.table_index();
                    slide_mask_by_code[color.index()][index] =
                        profile_slide_masks[profile_id.index()];
                    kind_bit_by_code[index] = kind_bit;
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

        let mut attack_dependencies = neighbourhoods;
        for from in Square::all() {
            for direction in Direction::ALL {
                attack_dependencies[from.raw_index()] |= rays[direction.index()][from.raw_index()];
            }
        }
        let reach = fixed.clone();
        let mut tables = Self {
            reach,
            rays,
            fixed,
            fixed_attacker_kinds,
            slide_mask_by_code,
            kind_bit_by_code,
            lion_jumps,
            neighbourhoods,
            attack_dependencies,
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

    /// 照会した対局者の駒なら、駒コードに対応する走りの絶対方向マスクを返す。
    /// 表は`PieceCode::table_index`で引き、駒として構築できないコードや相手駒のコードでは0を返すことで、所属の検査を兼ねる。
    #[inline]
    pub(crate) fn slide_mask_by_code(&self, color: Color, code: PieceCode) -> u8 {
        self.slide_mask_by_code[color.index()][code.table_index()]
    }

    /// `PieceCode::table_index`で表を引いて駒種ビットを返し、駒として構築できないコードでは0を返す。
    #[inline]
    pub(crate) fn kind_bit_by_code(&self, code: PieceCode) -> u32 {
        self.kind_bit_by_code[code.table_index()]
    }

    /// 指定した色・プロファイル・升の固定利きを返す。
    #[inline]
    pub(crate) fn fixed(&self, color: Color, profile: MovementProfileId, from: Square) -> Bitboard {
        self.fixed[Self::fixed_index(color, profile, from)]
    }

    /// 5×5近傍内の`from`から`target`へ固定利きが届く駒種をビット集合で返す。
    /// 生値は段幅16なので、この範囲の相対升は−34から34の差で一意に表せる。
    #[inline]
    pub(crate) fn fixed_attacker_kinds(&self, color: Color, target: Square, from: Square) -> u32 {
        let index = (i16::from(target.raw()) - i16::from(from.raw()) + 34) as usize;
        self.fixed_attacker_kinds[color.index()][index]
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
            super::fixed::movement_profile(crate::piece::PieceKind::King),
            from,
        )
    }

    /// 対象升を中心とする盤内の5×5近傍を返す。
    /// 全固定移動の移動量がこの範囲に収まることを利用し、固定利きの逆引きの走査範囲とする。
    #[inline]
    pub(crate) fn neighbourhood(&self, square: Square) -> Bitboard {
        self.neighbourhoods[square.raw_index()]
    }

    /// 指定升への利きに影響する占有升の範囲を返す。
    /// 固定利きと特殊移動の到達範囲は5×5近傍に収まり、走りの逆引きは8方向の遮蔽だけを見る。
    /// したがって、この和集合の外で占有が変わっても`attackers_to_by`の結果は変わらない。
    #[inline]
    pub(crate) fn attack_dependencies(&self, square: Square) -> Bitboard {
        self.attack_dependencies[square.raw_index()]
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

    /// 指定した走り線上の最初の遮蔽升を返す。空の線なら`None`となる。
    /// 捕獲できる駒はこの1枚だけなので、捕獲生成では途中の空升を求める必要がない。
    #[inline]
    pub(crate) fn first_blocker(
        &self,
        from: Square,
        direction: Direction,
        occupied: Bitboard,
    ) -> Option<Square> {
        let blockers = self.ray(from, direction) & occupied;
        if direction.increases_raw_index() {
            blockers.lsb()
        } else {
            blockers.msb()
        }
    }

    /// 遮る駒を考慮した走りの利きを返す。進行方向の最初の駒がある升までを含む
    /// (第7条第4項・第5項)。
    ///
    /// 升の生値が増える4方向では、利き線`ray`と占有の積`b`の3語を192ビット整数とみなし、
    /// 借りを伝播させて1を引く。
    /// `ray & (b ^ (b - 1))`で最初の遮蔽駒を含む升までを得る。
    /// `b`が空なら差分の全ビットが立ち、利き線全体が残る。
    /// 生値が減る4方向では、最上位の遮蔽駒を求め、その先の利き線を除く。
    /// 減る方向を`leading_zeros`と番兵表で書き換えない理由は
    /// `docs/adr/shift-pawn-origins-and-skip-other-low-yield-movegen-and-see-speedups.md`にある。
    pub(crate) fn sliding_control(
        &self,
        from: Square,
        direction: Direction,
        occupied: Bitboard,
    ) -> Bitboard {
        let candidates = self.ray(from, direction);
        let blockers = candidates & occupied;
        if direction.increases_raw_index() {
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
    use crate::board::step_square;

    #[test]
    fn piece_code_tables_match_decoded_movements() {
        let tables = attack_tables();
        let pieces = Color::ALL.into_iter().flat_map(|color| {
            PieceKind::ALL.into_iter().flat_map(move |kind| {
                [
                    PieceCode::new(color, kind),
                    PieceCode::new_promoted(color, kind),
                ]
                .into_iter()
                .flatten()
            })
        });
        let mut seen = [false; 256];
        for code in [PieceCode::EMPTY, PieceCode::WALL]
            .into_iter()
            .chain(pieces)
        {
            let index = code.table_index();
            assert!(!seen[index], "duplicate table index {index} for {code:?}");
            seen[index] = true;

            let kind = code.kind();
            let owner = code.color();
            assert_eq!(
                tables.kind_bit_by_code(code),
                kind.map_or(0, |kind| 1 << kind.index()),
                "{code:?}"
            );
            for color in Color::ALL {
                let expected = match (kind, owner) {
                    (Some(kind), Some(owner)) if owner == color => {
                        let profile = movement_profile_data(movement_profile(kind));
                        profile.slides.iter().fold(0, |mask, slide| {
                            mask | (1 << slide.direction.for_color(color).index())
                        })
                    }
                    _ => 0,
                };
                assert_eq!(
                    tables.slide_mask_by_code(color, code),
                    expected,
                    "{color:?} {code:?}"
                );
            }
        }

        for (index, used) in seen.into_iter().enumerate() {
            if !used {
                assert_eq!(tables.kind_bit_by_code[index], 0, "unused index {index}");
                for color in Color::ALL {
                    assert_eq!(
                        tables.slide_mask_by_code[color.index()][index],
                        0,
                        "{color:?} unused index {index}"
                    );
                }
            }
        }
    }

    // 実装契約(第7条4項・5項の走りの定義に接地): 走りの利きは、方向へ1升ずつ進む
    // 逐次歩行と一致する。盤端までの升を進行順に含み、最初の駒がある升を含んだ
    // 直後に打ち切られ、その先の升を含まない。
    #[test]
    fn article_7_4_5_sliding_control_matches_a_stepwise_walk_with_blockers() {
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

        // 全始点、全方向、および単升遮蔽の全組合せで語境界も覆う。
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
                    for enemy in [occupied, occupied & diagonal, Bitboard::EMPTY] {
                        let capture = tables
                            .first_blocker(from, direction, occupied)
                            .filter(|&square| enemy.contains(square));
                        assert_eq!(
                            Bitboard::from_squares(capture),
                            tables.sliding_control(from, direction, occupied) & enemy
                        );
                    }
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

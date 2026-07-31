//! 移行期限定の旧生成器オラクル。正準化の照合完了後、フェーズCで削除する。
//! 列挙ロジックはコミット c316a44 の `movegen/lion.rs` から移植した。

use std::collections::HashSet;

use super::lion;
use crate::attacks::{
    AttackTables, LionLikeProfile, SpecialMovement, attack_tables, movement_profile,
    movement_profile_data,
};
use crate::bitboard::Bitboard;
use crate::direction::step_square;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::square::{BOARD_SQUARE_COUNT, Square};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct LegacyPath {
    from: Square,
    mid: Option<Square>,
    to: Square,
}

#[derive(Clone, Copy)]
struct LocalOccupancy {
    occupied: Bitboard,
    own: Bitboard,
    enemy: Bitboard,
    current: Square,
}

impl LocalOccupancy {
    fn new(position: &Position, color: Color, current: Square) -> Self {
        Self {
            occupied: position.occupied(),
            own: position.pieces_of(color),
            enemy: position.pieces_of(color.opposite()),
            current,
        }
    }

    fn move_to(mut self, to: Square) -> Self {
        self.occupied.clear(self.current);
        self.own.clear(self.current);
        if self.enemy.contains(to) {
            self.occupied.clear(to);
            self.enemy.clear(to);
        }
        self.occupied.set(to);
        self.own.set(to);
        self.current = to;
        self
    }
}

fn generate_legacy_lion_double_and_jumps(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
    output: &mut Vec<LegacyPath>,
) {
    let own = position.pieces_of(color);
    let adjacent = tables.king_steps(from);

    for mid in adjacent & !own {
        let local = LocalOccupancy::new(position, color, from).move_to(mid);
        let second = tables.king_steps(mid) & !local.own;
        for to in second {
            output.push(LegacyPath {
                from,
                mid: Some(mid),
                to,
            });
        }
    }

    for to in tables.lion_jumps(from) & !own {
        output.push(LegacyPath {
            from,
            mid: None,
            to,
        });
    }
}

fn generate_legacy_lion_like_double_and_jumps(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
    output: &mut Vec<LegacyPath>,
) {
    let own = position.pieces_of(color);

    for relative in profile.directions {
        let direction = relative.for_color(color);
        let Some(first) = step_square(from, direction) else {
            continue;
        };
        let first_is_own = own.contains(first);

        if !first_is_own {
            output.push(LegacyPath {
                from,
                mid: Some(first),
                to: from,
            });
        }

        let Some(second) = step_square(first, direction) else {
            continue;
        };
        if own.contains(second) {
            continue;
        }
        output.push(LegacyPath {
            from,
            mid: None,
            to: second,
        });
        if !first_is_own {
            output.push(LegacyPath {
                from,
                mid: Some(first),
                to: second,
            });
        }
    }
}

fn push_legacy_lion_steps(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
    output: &mut Vec<LegacyPath>,
) {
    let own = position.pieces_of(color);
    for to in tables.king_steps(from) & !own {
        output.push(LegacyPath {
            from,
            mid: None,
            to,
        });
    }
}

fn push_legacy_lion_like_steps(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
    output: &mut Vec<LegacyPath>,
) {
    let own = position.pieces_of(color);
    for relative in profile.directions {
        let direction = relative.for_color(color);
        if let Some(to) = step_square(from, direction)
            && !own.contains(to)
        {
            output.push(LegacyPath {
                from,
                mid: None,
                to,
            });
        }
    }
}

fn canonicalize(
    position: &Position,
    color: Color,
    legacy: impl IntoIterator<Item = LegacyPath>,
) -> HashSet<Move> {
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    legacy
        .into_iter()
        .map(|path| {
            let mid = path.mid.and_then(|mid| {
                assert!(
                    !own.contains(mid),
                    "legacy oracle generated an own-occupied mid: {path:?}"
                );
                enemy.contains(mid).then_some(mid)
            });
            Move {
                from: path.from,
                mid,
                to: path.to,
                promote: false,
            }
        })
        .collect()
}

fn legacy_lion_special(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
) -> HashSet<Move> {
    let mut paths = Vec::new();
    generate_legacy_lion_double_and_jumps(tables, position, color, from, &mut paths);
    canonicalize(position, color, paths)
}

fn current_lion_special(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
) -> HashSet<Move> {
    let mut moves = Vec::new();
    lion::generate_lion_double_and_jumps(tables, position, color, from, &mut moves);
    moves.into_iter().collect()
}

fn current_lion_steps(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
) -> HashSet<Move> {
    let own = position.pieces_of(color);
    (tables.king_steps(from) & !own)
        .into_iter()
        .map(|to| Move {
            from,
            mid: None,
            to,
            promote: false,
        })
        .collect()
}

fn legacy_lion_family(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
) -> HashSet<Move> {
    let mut paths = Vec::new();
    push_legacy_lion_steps(tables, position, color, from, &mut paths);
    generate_legacy_lion_double_and_jumps(tables, position, color, from, &mut paths);
    canonicalize(position, color, paths)
}

fn current_lion_family(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
) -> HashSet<Move> {
    let mut moves = current_lion_steps(tables, position, color, from);
    moves.extend(current_lion_special(tables, position, color, from));
    moves
}

fn legacy_lion_like_special(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
) -> HashSet<Move> {
    let mut paths = Vec::new();
    generate_legacy_lion_like_double_and_jumps(position, color, from, profile, &mut paths);
    canonicalize(position, color, paths)
}

fn current_lion_like_special(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
) -> HashSet<Move> {
    let mut moves = Vec::new();
    lion::generate_lion_like_double_and_jumps(position, color, from, profile, &mut moves);
    moves.into_iter().collect()
}

fn legacy_lion_like_family(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
) -> HashSet<Move> {
    let mut paths = Vec::new();
    push_legacy_lion_like_steps(position, color, from, profile, &mut paths);
    generate_legacy_lion_like_double_and_jumps(position, color, from, profile, &mut paths);
    canonicalize(position, color, paths)
}

fn current_lion_like_family(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
) -> HashSet<Move> {
    let own = position.pieces_of(color);
    let mut moves = Vec::new();
    for relative in profile.directions {
        let direction = relative.for_color(color);
        if let Some(to) = step_square(from, direction)
            && !own.contains(to)
        {
            moves.push(Move {
                from,
                mid: None,
                to,
                promote: false,
            });
        }
    }
    lion::generate_lion_like_double_and_jumps(position, color, from, profile, &mut moves);
    moves.into_iter().collect()
}

fn assert_special_piece_matches(
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
    context: &str,
) {
    let (legacy, current) = match movement_profile_data(movement_profile(kind)).special {
        SpecialMovement::Lion => {
            let tables = attack_tables();
            let legacy_special = legacy_lion_special(tables, position, color, from);
            let current_special = current_lion_special(tables, position, color, from);
            let current_steps = current_lion_steps(tables, position, color, from);
            // 旧特殊関数の空mid経路は正準化後に隣接1升手にもなるが、
            // 現行実装ではその8方向をnormal側が生成する。
            let legacy_special_without_steps = legacy_special
                .difference(&current_steps)
                .copied()
                .collect::<HashSet<_>>();
            assert_eq!(
                legacy_special_without_steps,
                current_special,
                "{context}: lion special partition differs for color={color:?}, \
                 from={from:?}, missing={:?}, extra={:?}",
                legacy_special_without_steps
                    .difference(&current_special)
                    .collect::<Vec<_>>(),
                current_special
                    .difference(&legacy_special_without_steps)
                    .collect::<Vec<_>>(),
            );
            (
                legacy_lion_family(tables, position, color, from),
                current_lion_family(tables, position, color, from),
            )
        }
        SpecialMovement::LionLike(profile) => {
            let legacy_special = legacy_lion_like_special(position, color, from, profile);
            let current_special = current_lion_like_special(position, color, from, profile);
            assert_eq!(
                legacy_special,
                current_special,
                "{context}: lion-like special differs for color={color:?}, \
                 kind={kind:?}, from={from:?}, missing={:?}, extra={:?}",
                legacy_special
                    .difference(&current_special)
                    .collect::<Vec<_>>(),
                current_special
                    .difference(&legacy_special)
                    .collect::<Vec<_>>(),
            );
            (
                legacy_lion_like_family(position, color, from, profile),
                current_lion_like_family(position, color, from, profile),
            )
        }
        SpecialMovement::None => panic!("oracle requires a lion-family piece"),
    };

    assert_eq!(
        legacy,
        current,
        "{context}: color={color:?}, kind={kind:?}, from={from:?}, missing={:?}, extra={:?}",
        legacy.difference(&current).collect::<Vec<_>>(),
        current.difference(&legacy).collect::<Vec<_>>(),
    );
}

fn assert_position_matches(position: &Position, context: &str) -> usize {
    let mut comparisons = 0;
    for color in Color::ALL {
        for kind in [
            PieceKind::Lion,
            PieceKind::HornedFalcon,
            PieceKind::SoaringEagle,
        ] {
            for from in position.pieces_of_kind(color, kind) {
                assert_special_piece_matches(position, color, kind, from, context);
                comparisons += 1;
            }
        }
    }
    comparisons
}

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn position(side_to_move: Color, pieces: &[(Square, Color, PieceKind)]) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &(square, color, kind) in pieces {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }
    builder.finish().unwrap()
}

fn article_positions() -> Vec<Position> {
    vec![
        position(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Lion)]),
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::Lion),
                (sq(0, 1), Color::Black, PieceKind::Pawn),
                (sq(1, 0), Color::Black, PieceKind::Pawn),
                (sq(1, 1), Color::Black, PieceKind::Pawn),
            ],
        ),
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::Lion),
                (sq(0, 1), Color::Black, PieceKind::Pawn),
                (sq(1, 0), Color::White, PieceKind::Pawn),
                (sq(1, 1), Color::White, PieceKind::SilverGeneral),
            ],
        ),
        position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::Lion),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::White, PieceKind::SilverGeneral),
                (sq(4, 5), Color::Black, PieceKind::GoldGeneral),
            ],
        ),
        position(
            Color::White,
            &[
                (sq(6, 6), Color::White, PieceKind::Lion),
                (sq(6, 5), Color::Black, PieceKind::Pawn),
                (sq(5, 5), Color::Black, PieceKind::SilverGeneral),
                (sq(7, 6), Color::White, PieceKind::GoldGeneral),
            ],
        ),
        position(
            Color::Black,
            &[(sq(5, 5), Color::Black, PieceKind::HornedFalcon)],
        ),
        position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::HornedFalcon),
                (sq(5, 6), Color::Black, PieceKind::Pawn),
            ],
        ),
        position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::HornedFalcon),
                (sq(5, 6), Color::White, PieceKind::Pawn),
                (sq(5, 7), Color::White, PieceKind::SilverGeneral),
            ],
        ),
        position(
            Color::White,
            &[
                (sq(5, 5), Color::White, PieceKind::HornedFalcon),
                (sq(5, 4), Color::Black, PieceKind::Pawn),
                (sq(5, 3), Color::Black, PieceKind::SilverGeneral),
            ],
        ),
        position(
            Color::Black,
            &[(sq(5, 5), Color::Black, PieceKind::SoaringEagle)],
        ),
        position(
            Color::Black,
            &[
                (sq(5, 5), Color::Black, PieceKind::SoaringEagle),
                (sq(4, 6), Color::White, PieceKind::Pawn),
                (sq(6, 6), Color::Black, PieceKind::Pawn),
            ],
        ),
        position(
            Color::White,
            &[
                (sq(5, 5), Color::White, PieceKind::SoaringEagle),
                (sq(4, 4), Color::Black, PieceKind::Pawn),
                (sq(6, 4), Color::White, PieceKind::Pawn),
            ],
        ),
    ]
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        assert_ne!(seed, 0);
        Self { state: seed }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    fn index(&mut self, length: usize) -> usize {
        self.next() as usize % length
    }
}

fn random_empty_square(rng: &mut XorShift64, occupied: &mut [bool]) -> Square {
    let mut index = rng.index(BOARD_SQUARE_COUNT);
    while occupied[index] {
        index = rng.index(BOARD_SQUARE_COUNT);
    }
    occupied[index] = true;
    Square::from_dense(index).unwrap()
}

fn biased_random_position(rng: &mut XorShift64, iteration: usize) -> Position {
    const SPECIAL_KINDS: [PieceKind; 3] = [
        PieceKind::Lion,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ];
    const FILLER_KINDS: [PieceKind; 9] = [
        PieceKind::Pawn,
        PieceKind::GoBetween,
        PieceKind::SilverGeneral,
        PieceKind::GoldGeneral,
        PieceKind::Kirin,
        PieceKind::Phoenix,
        PieceKind::Bishop,
        PieceKind::Rook,
        PieceKind::FreeKing,
    ];
    const EXTRA_PIECE_COUNTS: [usize; 4] = [0, 4, 16, 48];

    let side_to_move = if rng.next() & 1 == 0 {
        Color::Black
    } else {
        Color::White
    };
    let focus_kind = SPECIAL_KINDS[(iteration / 16) % SPECIAL_KINDS.len()];
    let focus_color = if (iteration / 48).is_multiple_of(2) {
        Color::Black
    } else {
        Color::White
    };
    let king_pattern = iteration % 4;
    let extra_piece_count = EXTRA_PIECE_COUNTS[(iteration / 4) % EXTRA_PIECE_COUNTS.len()];
    let mut builder = PositionBuilder::new(side_to_move);
    let mut occupied = [false; BOARD_SQUARE_COUNT];

    let focus_square = random_empty_square(rng, &mut occupied);
    builder
        .put(focus_square, PieceCode::new(focus_color, focus_kind))
        .unwrap();
    if king_pattern & 1 != 0 {
        let square = random_empty_square(rng, &mut occupied);
        builder
            .put(square, PieceCode::new(Color::Black, PieceKind::King))
            .unwrap();
    }
    if king_pattern & 2 != 0 {
        let square = random_empty_square(rng, &mut occupied);
        builder
            .put(square, PieceCode::new(Color::White, PieceKind::King))
            .unwrap();
    }

    for _ in 0..extra_piece_count {
        let square = random_empty_square(rng, &mut occupied);
        let color = if rng.next() & 1 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let kind = FILLER_KINDS[rng.index(FILLER_KINDS.len())];
        builder.put(square, PieceCode::new(color, kind)).unwrap();
    }

    builder.finish().unwrap()
}

#[test]
fn legacy_oracle_matches_article_positions_after_canonicalization() {
    let positions = article_positions();
    assert_eq!(positions.len(), 12);

    let comparisons = positions
        .iter()
        .enumerate()
        .map(|(index, position)| {
            let comparisons =
                assert_position_matches(position, &format!("article position {index}"));
            assert_eq!(comparisons, 1);
            comparisons
        })
        .sum::<usize>();
    assert_eq!(comparisons, 12);
}

#[test]
fn legacy_oracle_matches_biased_seeded_random_positions_after_canonicalization() {
    const POSITION_COUNT: usize = 512;

    let mut rng = XorShift64::new(0x4f52_4143_4c45_0001);
    let mut comparisons = 0;
    for iteration in 0..POSITION_COUNT {
        let position = biased_random_position(&mut rng, iteration);
        let position_comparisons =
            assert_position_matches(&position, &format!("random position {iteration}"));
        assert_eq!(position_comparisons, 1);
        comparisons += position_comparisons;
    }
    assert_eq!(comparisons, POSITION_COUNT);
}

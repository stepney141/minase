use std::collections::HashSet;

use super::MoveGenerator;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::sfen::parse_sfen;
use crate::square::{BOARD_RANKS, BOARD_SQUARE_COUNT, Square};

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

#[test]
fn sfen_conversion_uses_shogiops_coordinates_and_piece_codes() {
    let position = parse_sfen("12/12/12/12/4B2l4/4S7/5+O6/7n4/12/12/12/12 b").unwrap();

    assert_eq!(
        position.piece_at(sq(4, 7)),
        Some(PieceCode::new(Color::Black, PieceKind::Bishop))
    );
    assert_eq!(
        position.piece_at(sq(7, 7)),
        Some(PieceCode::new(Color::White, PieceKind::Lance))
    );
    assert_eq!(
        position.piece_at(sq(4, 6)),
        Some(PieceCode::new(Color::Black, PieceKind::SilverGeneral))
    );
    assert_eq!(
        position.piece_at(sq(5, 5)),
        PieceCode::new(Color::Black, PieceKind::Kirin).promote()
    );
    assert_eq!(
        position.piece_at(sq(7, 4)),
        Some(PieceCode::new(Color::White, PieceKind::Lion))
    );
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct ReferenceBoard {
    squares: [Option<PieceCode>; BOARD_SQUARE_COUNT],
}

impl ReferenceBoard {
    fn from_position(position: &Position) -> Self {
        let mut squares = [None; BOARD_SQUARE_COUNT];
        for square in Square::all() {
            squares[square.dense_index()] = position.piece_at(square);
        }
        Self { squares }
    }

    fn piece_at(&self, square: Square) -> Option<PieceCode> {
        self.squares[square.dense_index()]
    }

    fn set(&mut self, square: Square, piece: Option<PieceCode>) {
        self.squares[square.dense_index()] = piece;
    }

    fn after_move(position: &Position, mv: Move) -> Self {
        let mut board = Self::from_position(position);
        let moving_piece_before = board
            .piece_at(mv.origin())
            .expect("reference move origin must contain a piece");
        let moving_piece_after = if mv.is_promoting() {
            moving_piece_before
                .promote()
                .expect("reference promoting move must have a promotable piece")
        } else {
            moving_piece_before
        };
        board.set(mv.origin(), None);
        if let Some(mid) = mv.mid {
            board.set(mid, None);
        }
        board.set(mv.to, Some(moving_piece_after));
        board
    }
}

#[derive(PartialEq, Eq, Debug)]
struct ReferencePositionState {
    board: ReferenceBoard,
    side_to_move: Color,
    lion_taken_by_non_lion: Option<Square>,
}

fn reference_apply_move(position: &Position, mv: Move) -> ReferencePositionState {
    let moving_kind = position
        .piece_at(mv.origin())
        .and_then(PieceCode::kind)
        .expect("reference move origin must contain a piece");
    let captures = reference_capture_squares(position, mv);
    let lion_taken_by_non_lion = (moving_kind != PieceKind::Lion)
        .then(|| {
            captures.into_iter().flatten().rfind(|&square| {
                position
                    .piece_at(square)
                    .is_some_and(|piece| piece.kind() == Some(PieceKind::Lion))
            })
        })
        .flatten();

    ReferencePositionState {
        board: ReferenceBoard::after_move(position, mv),
        side_to_move: position.side_to_move().opposite(),
        lion_taken_by_non_lion,
    }
}

const ORTHOGONAL: &[(i8, i8)] = &[(0, 1), (1, 0), (-1, 0), (0, -1)];
const DIAGONAL: &[(i8, i8)] = &[(1, 1), (-1, 1), (1, -1), (-1, -1)];
const ALL_DIRECTIONS: &[(i8, i8)] = &[
    (0, 1),
    (1, 1),
    (-1, 1),
    (1, 0),
    (-1, 0),
    (0, -1),
    (1, -1),
    (-1, -1),
];

fn oriented(color: Color, delta: (i8, i8)) -> (i8, i8) {
    match color {
        Color::Black => delta,
        Color::White => (-delta.0, -delta.1),
    }
}

fn fixed_deltas(kind: PieceKind) -> &'static [(i8, i8)] {
    match kind {
        PieceKind::Pawn => &[(0, 1)],
        PieceKind::GoBetween => &[(0, 1), (0, -1)],
        PieceKind::SideMover => &[(0, 1), (0, -1)],
        PieceKind::VerticalMover => &[(1, 0), (-1, 0)],
        PieceKind::DragonHorse => ORTHOGONAL,
        PieceKind::DragonKing => DIAGONAL,
        PieceKind::King | PieceKind::CrownPrince => ALL_DIRECTIONS,
        PieceKind::DrunkElephant => &[(0, 1), (1, 1), (-1, 1), (1, 0), (-1, 0), (1, -1), (-1, -1)],
        PieceKind::FerociousLeopard => &[(0, 1), (1, 1), (-1, 1), (0, -1), (1, -1), (-1, -1)],
        PieceKind::BlindTiger => &[(1, 1), (-1, 1), (1, 0), (-1, 0), (0, -1), (1, -1), (-1, -1)],
        PieceKind::CopperGeneral => &[(0, 1), (1, 1), (-1, 1), (0, -1)],
        PieceKind::SilverGeneral => &[(0, 1), (1, 1), (-1, 1), (1, -1), (-1, -1)],
        PieceKind::GoldGeneral => &[(0, 1), (1, 1), (-1, 1), (1, 0), (-1, 0), (0, -1)],
        PieceKind::Kirin => &[
            (1, 1),
            (-1, 1),
            (1, -1),
            (-1, -1),
            (0, 2),
            (2, 0),
            (-2, 0),
            (0, -2),
        ],
        PieceKind::Phoenix => &[
            (0, 1),
            (1, 0),
            (-1, 0),
            (0, -1),
            (2, 2),
            (-2, 2),
            (2, -2),
            (-2, -2),
        ],
        PieceKind::FlyingStag => &[(1, 1), (-1, 1), (1, 0), (-1, 0), (1, -1), (-1, -1)],
        PieceKind::Lance
        | PieceKind::ReverseChariot
        | PieceKind::Bishop
        | PieceKind::Rook
        | PieceKind::FreeKing
        | PieceKind::Lion
        | PieceKind::WhiteHorse
        | PieceKind::Whale
        | PieceKind::FlyingOx
        | PieceKind::FreeBoar
        | PieceKind::HornedFalcon
        | PieceKind::SoaringEagle => &[],
    }
}

fn slide_directions(kind: PieceKind) -> &'static [(i8, i8)] {
    match kind {
        PieceKind::Lance => &[(0, 1)],
        PieceKind::ReverseChariot => &[(0, 1), (0, -1)],
        PieceKind::SideMover => &[(1, 0), (-1, 0)],
        PieceKind::VerticalMover | PieceKind::FlyingStag => &[(0, 1), (0, -1)],
        PieceKind::Bishop | PieceKind::DragonHorse => DIAGONAL,
        PieceKind::Rook | PieceKind::DragonKing => ORTHOGONAL,
        PieceKind::FreeKing => ALL_DIRECTIONS,
        PieceKind::WhiteHorse => &[(0, 1), (1, 1), (-1, 1), (0, -1)],
        PieceKind::Whale => &[(0, 1), (0, -1), (1, -1), (-1, -1)],
        PieceKind::FlyingOx => &[(0, 1), (0, -1), (1, 1), (-1, 1), (1, -1), (-1, -1)],
        PieceKind::FreeBoar => &[(1, 0), (-1, 0), (1, 1), (-1, 1), (1, -1), (-1, -1)],
        PieceKind::HornedFalcon => &[(0, -1), (1, 0), (-1, 0), (1, 1), (-1, 1), (1, -1), (-1, -1)],
        PieceKind::SoaringEagle => &[(0, 1), (0, -1), (1, 0), (-1, 0), (1, -1), (-1, -1)],
        PieceKind::Pawn
        | PieceKind::GoBetween
        | PieceKind::King
        | PieceKind::DrunkElephant
        | PieceKind::FerociousLeopard
        | PieceKind::BlindTiger
        | PieceKind::CopperGeneral
        | PieceKind::SilverGeneral
        | PieceKind::GoldGeneral
        | PieceKind::Kirin
        | PieceKind::Phoenix
        | PieceKind::Lion
        | PieceKind::CrownPrince => &[],
    }
}

fn lion_like_directions(kind: PieceKind) -> &'static [(i8, i8)] {
    match kind {
        PieceKind::HornedFalcon => &[(0, 1)],
        PieceKind::SoaringEagle => &[(1, 1), (-1, 1)],
        _ => &[],
    }
}

fn is_own(position: &Position, color: Color, square: Square) -> bool {
    position
        .piece_at(square)
        .is_some_and(|piece| piece.color() == Some(color))
}

fn is_enemy(position: &Position, color: Color, square: Square) -> bool {
    position
        .piece_at(square)
        .is_some_and(|piece| piece.color() == Some(color.opposite()))
}

fn push_steps_and_slides(
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
    output: &mut Vec<Move>,
) {
    for &delta in fixed_deltas(kind) {
        let delta = oriented(color, delta);
        if let Some(to) = from.offset(delta.0, delta.1)
            && !is_own(position, color, to)
        {
            output.push(Move {
                from,
                mid: None,
                to,
                promote: false,
            });
        }
    }

    for &direction in slide_directions(kind) {
        let direction = oriented(color, direction);
        let mut current = from;
        while let Some(to) = current.offset(direction.0, direction.1) {
            if is_own(position, color, to) {
                break;
            }
            output.push(Move {
                from,
                mid: None,
                to,
                promote: false,
            });
            if position.piece_at(to).is_some() {
                break;
            }
            current = to;
        }
    }
}

fn push_lion_moves(position: &Position, color: Color, from: Square, output: &mut Vec<Move>) {
    let mut can_jitto = false;
    for &first_delta in ALL_DIRECTIONS {
        let Some(mid) = from.offset(first_delta.0, first_delta.1) else {
            continue;
        };
        if is_own(position, color, mid) {
            continue;
        }
        output.push(Move {
            from,
            mid: None,
            to: mid,
            promote: false,
        });

        if is_enemy(position, color, mid) {
            for &second_delta in ALL_DIRECTIONS {
                let Some(to) = mid.offset(second_delta.0, second_delta.1) else {
                    continue;
                };
                if to != from && is_own(position, color, to) {
                    continue;
                }
                output.push(Move {
                    from,
                    mid: Some(mid),
                    to,
                    promote: false,
                });
            }
        } else {
            can_jitto = true;
        }
    }

    for file_delta in -2_i8..=2 {
        for rank_delta in -2_i8..=2 {
            if file_delta.abs().max(rank_delta.abs()) != 2 {
                continue;
            }
            if let Some(to) = from.offset(file_delta, rank_delta)
                && !is_own(position, color, to)
            {
                output.push(Move {
                    from,
                    mid: None,
                    to,
                    promote: false,
                });
            }
        }
    }

    if can_jitto {
        output.push(Move {
            from,
            mid: None,
            to: from,
            promote: false,
        });
    }
}

fn push_lion_like_moves(
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
    output: &mut Vec<Move>,
) {
    let mut can_jitto = false;
    for &relative in lion_like_directions(kind) {
        let direction = oriented(color, relative);
        let Some(first) = from.offset(direction.0, direction.1) else {
            continue;
        };
        if !is_own(position, color, first) {
            output.push(Move {
                from,
                mid: None,
                to: first,
                promote: false,
            });
        }
        if position.piece_at(first).is_none() {
            can_jitto = true;
        } else if is_enemy(position, color, first) {
            output.push(Move {
                from,
                mid: Some(first),
                to: from,
                promote: false,
            });
        }

        let Some(to) = first.offset(direction.0, direction.1) else {
            continue;
        };
        if is_own(position, color, to) {
            continue;
        }
        output.push(Move {
            from,
            mid: None,
            to,
            promote: false,
        });
        if is_enemy(position, color, first) {
            output.push(Move {
                from,
                mid: Some(first),
                to,
                promote: false,
            });
        }
    }

    if can_jitto {
        output.push(Move {
            from,
            mid: None,
            to: from,
            promote: false,
        });
    }
}

fn reference_capture_squares(position: &Position, mv: Move) -> [Option<Square>; 2] {
    let moving_color = position
        .piece_at(mv.origin())
        .and_then(PieceCode::color)
        .expect("reference move origin must contain a piece");
    let enemy_at = |square| {
        position
            .piece_at(square)
            .is_some_and(|piece| piece.color() == Some(moving_color.opposite()))
            .then_some(square)
    };
    [
        mv.mid.and_then(enemy_at),
        (mv.to != mv.from).then(|| enemy_at(mv.to)).flatten(),
    ]
}

fn reference_tsukegui(position: &Position, mv: Move, lion_square: Square) -> bool {
    if position.piece_at(mv.origin()).and_then(PieceCode::kind) != Some(PieceKind::Lion) {
        return false;
    }
    let Some(mid) = mv.mid else {
        return false;
    };
    let distance = mv
        .from
        .file()
        .abs_diff(lion_square.file())
        .max(mv.from.rank().abs_diff(lion_square.rank()));
    let captures = reference_capture_squares(position, mv);
    distance == 2
        && mv.to == lion_square
        && captures == [Some(mid), Some(lion_square)]
        && position.piece_at(mid).is_some_and(|piece| {
            !matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween))
        })
}

fn board_piece_controls(
    board: &ReferenceBoard,
    piece: PieceCode,
    from: Square,
    target: Square,
) -> bool {
    let color = piece.color().unwrap();
    let kind = piece.kind().unwrap();

    if fixed_deltas(kind)
        .iter()
        .map(|&delta| oriented(color, delta))
        .any(|delta| from.offset(delta.0, delta.1) == Some(target))
    {
        return true;
    }

    for &relative in slide_directions(kind) {
        let direction = oriented(color, relative);
        let mut current = from;
        while let Some(next) = current.offset(direction.0, direction.1) {
            if next == target {
                return true;
            }
            if board.piece_at(next).is_some() {
                break;
            }
            current = next;
        }
    }

    match kind {
        PieceKind::Lion => {
            let distance = from
                .file()
                .abs_diff(target.file())
                .max(from.rank().abs_diff(target.rank()));
            (1..=2).contains(&distance)
        }
        PieceKind::HornedFalcon | PieceKind::SoaringEagle => lion_like_directions(kind)
            .iter()
            .map(|&delta| oriented(color, delta))
            .any(|delta| {
                from.offset(delta.0, delta.1) == Some(target)
                    || from
                        .offset(delta.0, delta.1)
                        .and_then(|mid| mid.offset(delta.0, delta.1))
                        == Some(target)
            }),
        _ => false,
    }
}

fn reference_lion_has_foot(position: &Position, mv: Move, lion_square: Square) -> bool {
    let defending_color = position
        .piece_at(lion_square)
        .and_then(PieceCode::color)
        .expect("captured lion must have a color");
    let board = ReferenceBoard::after_move(position, mv);
    let [mid_capture, destination_capture] = reference_capture_squares(position, mv);
    let captured_pawn_or_go_between_had_foot = destination_capture == Some(lion_square)
        && mid_capture.is_some_and(|mid| {
            position.piece_at(mid).is_some_and(|piece| {
                matches!(piece.kind(), Some(PieceKind::Pawn | PieceKind::GoBetween))
                    && board_piece_controls(&board, piece, mid, mv.destination())
            })
        });

    captured_pawn_or_go_between_had_foot
        || Square::all().any(|from| {
            board.piece_at(from).is_some_and(|piece| {
                piece.color() == Some(defending_color)
                    && board_piece_controls(&board, piece, from, mv.destination())
            })
        })
}

fn reference_special_move_is_legal(position: &Position, mv: Move) -> bool {
    let captures = reference_capture_squares(position, mv);
    let captured_lions = captures.map(|capture| {
        capture.filter(|&square| {
            position
                .piece_at(square)
                .is_some_and(|piece| piece.kind() == Some(PieceKind::Lion))
        })
    });
    let moving_kind = position.piece_at(mv.origin()).and_then(PieceCode::kind);

    if moving_kind == Some(PieceKind::Lion) {
        for lion_square in captured_lions.into_iter().flatten() {
            let distance = mv
                .origin()
                .file()
                .abs_diff(lion_square.file())
                .max(mv.origin().rank().abs_diff(lion_square.rank()));
            let legal = distance == 1
                || (distance == 2
                    && (reference_tsukegui(position, mv, lion_square)
                        || !reference_lion_has_foot(position, mv, lion_square)));
            if !legal {
                return false;
            }
        }
    }

    let move_is_tsukegui = captured_lions
        .into_iter()
        .flatten()
        .any(|lion_square| reference_tsukegui(position, mv, lion_square));
    if position.lion_taken_by_non_lion().is_some()
        && !move_is_tsukegui
        && captured_lions
            .into_iter()
            .flatten()
            .any(|lion_square| reference_lion_has_foot(position, mv, lion_square))
    {
        return false;
    }
    true
}

fn reference_can_promote(kind: PieceKind) -> bool {
    matches!(
        kind,
        PieceKind::Pawn
            | PieceKind::GoBetween
            | PieceKind::Lance
            | PieceKind::ReverseChariot
            | PieceKind::SideMover
            | PieceKind::VerticalMover
            | PieceKind::Bishop
            | PieceKind::Rook
            | PieceKind::DragonHorse
            | PieceKind::DragonKing
            | PieceKind::DrunkElephant
            | PieceKind::FerociousLeopard
            | PieceKind::BlindTiger
            | PieceKind::CopperGeneral
            | PieceKind::SilverGeneral
            | PieceKind::GoldGeneral
            | PieceKind::Kirin
            | PieceKind::Phoenix
    )
}

fn reference_in_promotion_zone(color: Color, square: Square) -> bool {
    match color {
        Color::Black => square.rank() >= BOARD_RANKS - 4,
        Color::White => square.rank() < 4,
    }
}

fn promoting_variant(mv: Move) -> Move {
    Move {
        promote: true,
        ..mv
    }
}

fn reference_expand_move(position: &Position, base: Move, output: &mut Vec<Move>) {
    if !reference_special_move_is_legal(position, base) {
        return;
    }

    output.push(base);
    let piece = position.piece_at(base.origin()).unwrap();
    let color = piece.color().unwrap();
    let kind = piece.kind().unwrap();
    if piece.is_promoted() || !reference_can_promote(kind) {
        return;
    }
    let captures = reference_capture_squares(position, base);
    let has_capture = captures.into_iter().any(|capture| capture.is_some());
    let from_in_zone = reference_in_promotion_zone(color, base.origin());
    let to_in_zone = reference_in_promotion_zone(color, base.destination());
    let enters_zone = !from_in_zone && to_in_zone;
    let capture_in_or_from_zone = has_capture && (from_in_zone || to_in_zone);
    let pawn_reaches_last_rank_without_capture = kind == PieceKind::Pawn
        && !has_capture
        && match color {
            Color::Black => base.destination().rank() == BOARD_RANKS - 1,
            Color::White => base.destination().rank() == 0,
        };
    if enters_zone || capture_in_or_from_zone || pawn_reaches_last_rank_without_capture {
        output.push(promoting_variant(base));
    }
}

fn reference_generate_moves(position: &Position) -> Vec<Move> {
    let color = position.side_to_move();
    let mut base_moves = Vec::new();
    for from in Square::all() {
        let Some(piece) = position.piece_at(from) else {
            continue;
        };
        if piece.color() != Some(color) {
            continue;
        }
        let kind = piece.kind().unwrap();
        push_steps_and_slides(position, color, kind, from, &mut base_moves);
        match kind {
            PieceKind::Lion => push_lion_moves(position, color, from, &mut base_moves),
            PieceKind::HornedFalcon | PieceKind::SoaringEagle => {
                push_lion_like_moves(position, color, kind, from, &mut base_moves);
            }
            _ => {}
        }
    }

    let mut moves = Vec::new();
    for base in base_moves {
        reference_expand_move(position, base, &mut moves);
    }
    moves
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

fn random_piece(rng: &mut XorShift64, color: Color, kind: PieceKind) -> PieceCode {
    if rng.next() & 1 != 0
        && let Some(promoted) = PieceCode::new_promoted(color, kind)
    {
        promoted
    } else {
        PieceCode::new(color, kind)
    }
}

fn random_position(rng: &mut XorShift64, focus_kind: PieceKind) -> Position {
    let side_to_move = if rng.next() & 1 == 0 {
        Color::Black
    } else {
        Color::White
    };
    let mut builder = PositionBuilder::new(side_to_move);
    let mut occupied = [false; BOARD_SQUARE_COUNT];

    let focus_index = rng.index(BOARD_SQUARE_COUNT);
    let focus_square = Square::from_dense(focus_index).unwrap();
    builder
        .put(focus_square, random_piece(rng, side_to_move, focus_kind))
        .unwrap();
    occupied[focus_index] = true;

    for _ in 0..20 {
        let mut index = rng.index(BOARD_SQUARE_COUNT);
        while occupied[index] {
            index = rng.index(BOARD_SQUARE_COUNT);
        }
        occupied[index] = true;
        let square = Square::from_dense(index).unwrap();
        let color = if rng.next() & 1 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let kind = PieceKind::ALL[rng.index(PieceKind::ALL.len())];
        builder.put(square, random_piece(rng, color, kind)).unwrap();
    }
    builder.finish().unwrap()
}

fn random_senjishi_position(rng: &mut XorShift64) -> Position {
    let mut builder = PositionBuilder::new(Color::Black);
    let fixed = [
        (sq(0, 0), Color::Black, PieceKind::Bishop),
        (sq(1, 1), Color::White, PieceKind::Lion),
        (sq(4, 4), Color::Black, PieceKind::Lion),
        (sq(4, 3), Color::Black, PieceKind::GoldGeneral),
        (sq(4, 6), Color::White, PieceKind::Rook),
    ];
    let mut occupied = [false; BOARD_SQUARE_COUNT];
    for (square, color, kind) in fixed {
        builder.put(square, PieceCode::new(color, kind)).unwrap();
        occupied[square.dense_index()] = true;
    }
    occupied[sq(4, 5).dense_index()] = true;

    for _ in 0..14 {
        let mut index = rng.index(BOARD_SQUARE_COUNT);
        while occupied[index] {
            index = rng.index(BOARD_SQUARE_COUNT);
        }
        occupied[index] = true;
        let square = Square::from_dense(index).unwrap();
        let color = if rng.next() & 1 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let kind = PieceKind::ALL[rng.index(PieceKind::ALL.len())];
        builder.put(square, random_piece(rng, color, kind)).unwrap();
    }

    let mut position = builder.finish().unwrap();
    position.make_move_unchecked(Move {
        from: sq(0, 0),
        mid: None,
        to: sq(1, 1),
        promote: false,
    });
    assert_eq!(position.validate(), Ok(()));
    assert!(position.lion_taken_by_non_lion().is_some());
    position
}

#[derive(Clone, Copy, Debug)]
enum IntermediateContents {
    Empty,
    PawnOrGoBetween,
    ValuableLion,
}

#[derive(Clone, Copy, Debug)]
enum FootShape {
    Sliding,
    Adjacent,
    None,
}

fn lion_duel_position(
    direction_index: usize,
    distance: u8,
    intermediate: IntermediateContents,
    foot: FootShape,
    slider_behind_origin: bool,
    senjishi: bool,
) -> Position {
    const DIRECTIONS: [(i8, i8); 8] = [
        (0, 1),
        (1, 1),
        (1, 0),
        (1, -1),
        (0, -1),
        (-1, -1),
        (-1, 0),
        (-1, 1),
    ];

    let direction = DIRECTIONS[direction_index];
    let origin = sq(5, 5);
    let target = origin
        .offset(direction.0 * distance as i8, direction.1 * distance as i8)
        .unwrap();
    let intermediate_delta = if distance == 1 {
        if direction.0 != 0 && direction.1 != 0 {
            (direction.0, 0)
        } else {
            (-direction.1, direction.0)
        }
    } else {
        direction
    };
    let mid = origin
        .offset(intermediate_delta.0, intermediate_delta.1)
        .unwrap();
    let perpendicular = (-direction.1, direction.0);
    let mut builder = PositionBuilder::new(if senjishi { Color::Black } else { Color::White });
    builder
        .put(origin, PieceCode::new(Color::White, PieceKind::Lion))
        .unwrap();
    let target_lion = if (direction_index + distance as usize).is_multiple_of(2) {
        PieceCode::new(Color::Black, PieceKind::Lion)
    } else {
        PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap()
    };
    builder.put(target, target_lion).unwrap();

    match intermediate {
        IntermediateContents::Empty => {}
        IntermediateContents::PawnOrGoBetween => {
            let kind = if (direction_index + distance as usize).is_multiple_of(2) {
                PieceKind::Pawn
            } else {
                PieceKind::GoBetween
            };
            builder
                .put(mid, PieceCode::new(Color::Black, kind))
                .unwrap();
        }
        IntermediateContents::ValuableLion => {
            builder
                .put(mid, PieceCode::new(Color::Black, PieceKind::Lion))
                .unwrap();
        }
    }

    match foot {
        FootShape::Sliding => {
            let square = target.offset(direction.0, direction.1).unwrap();
            let kind = if direction.0 == 0 || direction.1 == 0 {
                PieceKind::Rook
            } else {
                PieceKind::Bishop
            };
            builder
                .put(square, PieceCode::new(Color::Black, kind))
                .unwrap();
        }
        FootShape::Adjacent => {
            let square = target.offset(perpendicular.0, perpendicular.1).unwrap();
            builder
                .put(square, PieceCode::new(Color::Black, PieceKind::King))
                .unwrap();
        }
        FootShape::None => {}
    }

    if slider_behind_origin {
        let square = origin.offset(-direction.0, -direction.1).unwrap();
        let kind = if direction.0 == 0 || direction.1 == 0 {
            PieceKind::Rook
        } else {
            PieceKind::Bishop
        };
        builder
            .put(square, PieceCode::new(Color::Black, kind))
            .unwrap();
    }

    if senjishi {
        builder
            .put(sq(11, 10), PieceCode::new(Color::Black, PieceKind::Pawn))
            .unwrap();
        builder
            .put(sq(11, 11), PieceCode::new(Color::White, PieceKind::Lion))
            .unwrap();
    }

    let mut position = builder.finish().unwrap();
    if senjishi {
        position.make_move_unchecked(Move {
            from: sq(11, 10),
            mid: None,
            to: sq(11, 11),
            promote: false,
        });
        assert_eq!(position.validate(), Ok(()));
        assert_eq!(position.side_to_move(), Color::White);
        assert!(position.lion_taken_by_non_lion().is_some());
    }
    position
}

fn assert_move_sets_match(position: &Position, context: &str) {
    let mut actual = Vec::new();
    MoveGenerator::standard().generate_moves(position, &mut actual);
    let reference = reference_generate_moves(position);
    let actual_set: HashSet<_> = actual.iter().copied().collect();
    let reference_set: HashSet<_> = reference.iter().copied().collect();

    assert_eq!(
        actual_set,
        reference_set,
        "{context}: missing={:?}, extra={:?}",
        reference_set.difference(&actual_set).collect::<Vec<_>>(),
        actual_set.difference(&reference_set).collect::<Vec<_>>(),
    );

    for &mv in &actual_set {
        assert_eq!(
            position.captured_squares(mv),
            reference_capture_squares(position, mv),
            "{context}: captured_squares differ for {mv:?}",
        );

        let expected = reference_apply_move(position, mv);
        let mut actual_after = position.clone();
        actual_after.make_move_unchecked(mv);
        assert_eq!(
            actual_after.side_to_move(),
            expected.side_to_move,
            "{context}: side to move differs after {mv:?}",
        );
        assert_eq!(
            actual_after.lion_taken_by_non_lion(),
            expected.lion_taken_by_non_lion,
            "{context}: senjishi state differs after {mv:?}",
        );
        for square in Square::all() {
            assert_eq!(
                actual_after.piece_at(square),
                expected.board.piece_at(square),
                "{context}: board differs at {square:?} after {mv:?}",
            );
        }
        assert_eq!(
            actual_after.validate(),
            Ok(()),
            "{context}: invalid resulting position after {mv:?}",
        );
    }
}

#[test]
fn naive_reference_matches_generated_move_sets_on_seeded_random_positions() {
    let mut rng = XorShift64::new(0x4d49_4e41_5345_0004);
    for iteration in 0..1_000 {
        let kind = PieceKind::ALL[iteration % PieceKind::ALL.len()];
        let position = random_position(&mut rng, kind);
        assert_move_sets_match(
            &position,
            &format!("random position {iteration}, kind={kind:?}"),
        );
    }
    for iteration in 0..6 {
        let position = random_senjishi_position(&mut rng);
        assert_move_sets_match(&position, &format!("senjishi position {iteration}"));
    }
}

#[test]
fn lion_duel_reference_matches_generated_moves_across_parameter_matrix() {
    let intermediates = [
        IntermediateContents::Empty,
        IntermediateContents::PawnOrGoBetween,
        IntermediateContents::ValuableLion,
    ];
    let feet = [FootShape::Sliding, FootShape::Adjacent, FootShape::None];
    let mut positions = 0;

    for direction_index in 0..8 {
        for distance in 1..=3 {
            for intermediate in intermediates {
                for foot in feet {
                    for slider_behind_origin in [false, true] {
                        for senjishi in [false, true] {
                            let position = lion_duel_position(
                                direction_index,
                                distance,
                                intermediate,
                                foot,
                                slider_behind_origin,
                                senjishi,
                            );
                            assert_move_sets_match(
                                &position,
                                &format!(
                                    "lion duel direction={direction_index}, distance={distance}, \
                                     intermediate={intermediate:?}, foot={foot:?}, \
                                     slider_behind_origin={slider_behind_origin}, \
                                     senjishi={senjishi}"
                                ),
                            );
                            positions += 1;
                        }
                    }
                }
            }
        }
    }

    assert_eq!(positions, 864);
}

#[test]
fn seeded_random_moves_round_trip_validate_and_are_unique() {
    let generator = MoveGenerator::standard();
    let mut rng = XorShift64::new(0x554e_4d41_4b45_0004);

    for iteration in 0..64 {
        let kind = PieceKind::ALL[iteration % PieceKind::ALL.len()];
        let mut position = random_position(&mut rng, kind);
        let before = position.clone();
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        assert_eq!(
            moves.iter().copied().collect::<HashSet<_>>().len(),
            moves.len(),
            "duplicate move in random position {iteration}",
        );
        let enemy = position.pieces_of(position.side_to_move().opposite());
        let own = position.pieces_of(position.side_to_move());
        let mut jitto_origins = HashSet::new();
        for &mv in &moves {
            if let Some(mid) = mv.mid {
                assert!(
                    enemy.contains(mid),
                    "non-enemy mid in random position {iteration}: move={mv:?}"
                );
            }
            if mv.mid.is_none() && mv.to == mv.from {
                assert!(
                    jitto_origins.insert(mv.from),
                    "multiple jitto moves from one origin in random position {iteration}: \
                     move={mv:?}"
                );
            }
            if mv.to != mv.from {
                assert!(
                    !own.contains(mv.to),
                    "own-occupied destination in random position {iteration}: move={mv:?}"
                );
            }
        }
        let before_zobrist = before.zobrist();
        for mv in moves {
            let undo = position.make_move_unchecked(mv);
            assert_eq!(position.validate(), Ok(()), "move={mv:?}");
            position.unmake_move(undo);
            assert_eq!(position.validate(), Ok(()), "unmade move={mv:?}");
            assert_eq!(position, before, "move={mv:?}");
            assert_eq!(position.zobrist(), before_zobrist, "move={mv:?}");
        }
    }
}

#[test]
fn seeded_random_playout_unmakes_to_initial_position() {
    let generator = MoveGenerator::standard();
    let mut rng = XorShift64::new(0x504c_4159_4f55_5404);
    let mut position = Position::initial();
    let initial = position.clone();
    let mut history = Vec::new();

    for ply in 0..64 {
        let mut moves = Vec::new();
        generator.generate_moves(&position, &mut moves);
        assert!(!moves.is_empty(), "no move at ply {ply}");
        let mv = moves[rng.index(moves.len())];
        history.push(position.make_move_unchecked(mv));
        assert_eq!(position.validate(), Ok(()), "ply={ply}, move={mv:?}");
    }
    while let Some(undo) = history.pop() {
        position.unmake_move(undo);
        assert_eq!(
            position.validate(),
            Ok(()),
            "invalid position while unmaking ply {}",
            history.len()
        );
    }
    assert_eq!(position, initial);
}

#[test]
fn articles_3_8_and_8_3_5_move_leaving_king_capturable_is_generated() {
    let position = position(
        Color::Black,
        &[
            (sq(4, 0), Color::Black, PieceKind::King),
            (sq(4, 2), Color::Black, PieceKind::SideMover),
            (sq(4, 5), Color::White, PieceKind::Rook),
        ],
    );
    let exposes_king = Move {
        from: sq(4, 2),
        mid: None,
        to: sq(5, 2),
        promote: false,
    };
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert!(moves.contains(&exposes_king));
}

#[test]
fn article_12_8_igui_has_one_move_encoding() {
    let from = sq(5, 5);
    let mid = sq(5, 6);
    let position = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Lion),
            (mid, Color::White, PieceKind::SilverGeneral),
        ],
    );
    let expected = Move {
        from,
        mid: Some(mid),
        to: from,
        promote: false,
    };
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert_eq!(moves.iter().filter(|&&mv| mv == expected).count(), 1);
    assert_eq!(
        moves
            .iter()
            .filter(|&&mv| {
                mv.origin() == from
                    && mv.destination() == from
                    && position.captured_squares(mv) == [Some(mid), None]
            })
            .count(),
        1
    );
}

#[test]
fn articles_16_1_and_16_11_tsukegui_requires_the_first_capture_on_mid() {
    let position = position(
        Color::Black,
        &[
            (sq(2, 2), Color::Black, PieceKind::Lion),
            (sq(3, 2), Color::White, PieceKind::SilverGeneral),
            (sq(4, 2), Color::White, PieceKind::Lion),
            (sq(4, 5), Color::White, PieceKind::Rook),
        ],
    );
    let capture_without_first_capture = Move {
        from: sq(2, 2),
        mid: None,
        to: sq(4, 2),
        promote: false,
    };
    let mut moves = Vec::new();
    MoveGenerator::standard().generate_moves(&position, &mut moves);

    assert!(!moves.contains(&capture_without_first_capture));
}

#[test]
fn article_24_equivalent_positions_ignore_quiet_move_history() {
    let initial = position(
        Color::Black,
        &[
            (sq(3, 3), Color::Black, PieceKind::Lion),
            (sq(8, 8), Color::White, PieceKind::Lion),
        ],
    );
    let mut first = initial.clone();
    first.make_move_unchecked(Move {
        from: sq(3, 3),
        mid: None,
        to: sq(3, 4),
        promote: false,
    });
    first.make_move_unchecked(Move {
        from: sq(8, 8),
        mid: None,
        to: sq(8, 7),
        promote: false,
    });
    first.make_move_unchecked(Move {
        from: sq(3, 4),
        mid: None,
        to: sq(3, 3),
        promote: false,
    });
    first.make_move_unchecked(Move {
        from: sq(8, 7),
        mid: None,
        to: sq(8, 8),
        promote: false,
    });

    let mut second = initial;
    second.make_move_unchecked(Move {
        from: sq(3, 3),
        mid: None,
        to: sq(4, 3),
        promote: false,
    });
    second.make_move_unchecked(Move {
        from: sq(8, 8),
        mid: None,
        to: sq(7, 8),
        promote: false,
    });
    second.make_move_unchecked(Move {
        from: sq(4, 3),
        mid: None,
        to: sq(3, 3),
        promote: false,
    });
    second.make_move_unchecked(Move {
        from: sq(7, 8),
        mid: None,
        to: sq(8, 8),
        promote: false,
    });

    assert_eq!(first, second);
}

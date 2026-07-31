use crate::attacks::{AttackTables, LionLikeProfile};
use crate::bitboard::Bitboard;
use crate::direction::step_square;
use crate::mv::Move;
use crate::piece::Color;
use crate::position::Position;
use crate::square::Square;

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

pub(super) fn generate_lion_double_and_jumps(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    from: Square,
    output: &mut Vec<Move>,
) {
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    let adjacent = tables.king_steps(from);

    for mid in adjacent & enemy {
        let local = LocalOccupancy::new(position, color, from).move_to(mid);
        let second = tables.king_steps(mid) & !local.own;
        for to in second {
            output.push(Move {
                from,
                mid: Some(mid),
                to,
                promote: false,
            });
        }
    }

    for to in tables.lion_jumps(from) & !own {
        output.push(Move {
            from,
            mid: None,
            to,
            promote: false,
        });
    }

    if !(adjacent & !position.occupied()).is_empty() {
        output.push(Move {
            from,
            mid: None,
            to: from,
            promote: false,
        });
    }
}

pub(super) fn generate_lion_like_double_and_jumps(
    position: &Position,
    color: Color,
    from: Square,
    profile: LionLikeProfile,
    output: &mut Vec<Move>,
) {
    let own = position.pieces_of(color);
    let enemy = position.pieces_of(color.opposite());
    let mut can_jitto = false;

    for relative in profile.directions {
        let direction = relative.for_color(color);
        let Some(first) = step_square(from, direction) else {
            continue;
        };
        if !position.occupied().contains(first) {
            can_jitto = true;
        } else if enemy.contains(first) {
            output.push(Move {
                from,
                mid: Some(first),
                to: from,
                promote: false,
            });
        }

        let Some(second) = step_square(first, direction) else {
            continue;
        };
        if own.contains(second) {
            continue;
        }
        output.push(Move {
            from,
            mid: None,
            to: second,
            promote: false,
        });
        if enemy.contains(first) {
            output.push(Move {
                from,
                mid: Some(first),
                to: second,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attacks::attack_tables;
    use crate::piece::{PieceCode, PieceKind};
    use crate::position::PositionBuilder;

    fn sq(file: u8, rank: u8) -> Square {
        Square::new(file, rank).unwrap()
    }

    #[test]
    fn second_lion_stage_uses_updated_occupancy() {
        let mut builder = PositionBuilder::new(Color::Black);
        builder
            .put(sq(5, 5), PieceCode::new(Color::Black, PieceKind::Lion))
            .unwrap();
        builder
            .put(sq(5, 6), PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        builder
            .put(sq(6, 6), PieceCode::new(Color::White, PieceKind::Pawn))
            .unwrap();
        let position = builder.finish().unwrap();
        let mut moves = Vec::new();
        generate_lion_double_and_jumps(
            attack_tables(),
            &position,
            Color::Black,
            sq(5, 5),
            &mut moves,
        );

        assert!(moves.contains(&Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(6, 6),
            promote: false,
        }));
        assert!(moves.contains(&Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(5, 5),
            promote: false,
        }));
    }
}

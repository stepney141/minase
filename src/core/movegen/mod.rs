#[cfg(test)]
mod tests;

use core::fmt;

use crate::core::attacks::{
    AttackTables, LionLikeProfile, SpecialMovement, attack_tables, movement_profile,
    movement_profile_data,
};
use crate::core::bitboard::Bitboard;
use crate::core::direction::step_square;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::position::Position;
use crate::core::rules::{PromotionChoice, Rules};
use crate::core::square::Square;

pub struct MoveGenerator {
    tables: &'static AttackTables,
    rules: Rules,
}

impl MoveGenerator {
    pub fn standard() -> Self {
        Self::new(Rules::standard())
    }

    pub fn new(rules: Rules) -> Self {
        Self {
            tables: attack_tables(),
            rules,
        }
    }

    pub fn generate_moves(&self, position: &Position, output: &mut Vec<Move>) {
        generate_moves(self, position, output);
    }

    pub(crate) const fn tables(&self) -> &AttackTables {
        self.tables
    }

    pub(crate) const fn rules(&self) -> Rules {
        self.rules
    }
}

pub(crate) fn piece_control_with_occupancy(
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> crate::Bitboard {
    piece_control_with_tables(attack_tables(), occupied, color, kind, from)
}

fn promoting_variant(mv: Move) -> Move {
    Move {
        promote: true,
        ..mv
    }
}

fn piece_control_with_tables(
    tables: &AttackTables,
    occupied: Bitboard,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> Bitboard {
    let profile_id = movement_profile(kind);
    let profile = movement_profile_data(profile_id);
    let mut result = tables.fixed(color, profile_id, from);
    for slide in profile.slides {
        result |= tables.sliding_control(
            from,
            slide.direction.for_color(color),
            slide.max_steps,
            occupied,
        );
    }

    match profile.special {
        SpecialMovement::None => {}
        SpecialMovement::Lion => {
            result |= tables.king_steps(from) | tables.lion_jumps(from);
        }
        SpecialMovement::LionLike(lion_like) => {
            for direction in lion_like.directions {
                let direction = direction.for_color(color);
                if let Some(first) = crate::core::direction::step_square(from, direction) {
                    result.set(first);
                    if let Some(second) = crate::core::direction::step_square(first, direction) {
                        result.set(second);
                    }
                }
            }
        }
    }
    result
}

fn push_with_promotion(
    generator: &MoveGenerator,
    position: &Position,
    moving_kind: PieceKind,
    base: Move,
    output: &mut Vec<Move>,
) {
    if !generator.rules().special_move_is_legal(position, base) {
        return;
    }

    match generator
        .rules()
        .promotion_choice(position, &base, moving_kind)
    {
        PromotionChoice::NoPromotion => output.push(base),
        PromotionChoice::PromotionOptional => {
            output.push(base);
            output.push(promoting_variant(base));
        }
    }
}

fn generate_moves(generator: &MoveGenerator, position: &Position, output: &mut Vec<Move>) {
    let color = position.side_to_move();
    let own = position.pieces_of(color);

    for kind in PieceKind::ALL {
        let profile = movement_profile_data(movement_profile(kind));
        for from in position.pieces_of_kind(color, kind) {
            let mut base_moves = Vec::new();
            let step_destinations =
                (piece_control_without_special(generator.tables(), position, color, kind, from)
                    | special_step_destinations(generator.tables(), color, from, profile.special))
                    & !own;
            for to in step_destinations {
                base_moves.push(Move {
                    from,
                    mid: None,
                    to,
                    promote: false,
                });
            }

            match profile.special {
                SpecialMovement::None => {}
                SpecialMovement::Lion => {
                    generate_lion_double_and_jumps(
                        generator.tables(),
                        position,
                        color,
                        from,
                        &mut base_moves,
                    );
                }
                SpecialMovement::LionLike(profile) => {
                    generate_lion_like_double_and_jumps(
                        position,
                        color,
                        from,
                        profile,
                        &mut base_moves,
                    );
                }
            }
            for base in base_moves {
                push_with_promotion(generator, position, kind, base, output);
            }
        }
    }
}

fn special_step_destinations(
    tables: &AttackTables,
    color: Color,
    from: Square,
    special: SpecialMovement,
) -> Bitboard {
    match special {
        SpecialMovement::None => Bitboard::EMPTY,
        SpecialMovement::Lion => tables.king_steps(from),
        SpecialMovement::LionLike(profile) => {
            let mut destinations = Bitboard::EMPTY;
            for relative in profile.directions {
                if let Some(first) =
                    crate::core::direction::step_square(from, relative.for_color(color))
                {
                    destinations.set(first);
                }
            }
            destinations
        }
    }
}

fn piece_control_without_special(
    tables: &AttackTables,
    position: &Position,
    color: Color,
    kind: PieceKind,
    from: Square,
) -> Bitboard {
    let profile_id = movement_profile(kind);
    let profile = movement_profile_data(profile_id);
    let mut result = tables.fixed(color, profile_id, from);
    for slide in profile.slides {
        result |= tables.sliding_control(
            from,
            slide.direction.for_color(color),
            slide.max_steps,
            position.occupied(),
        );
    }
    result
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

fn generate_lion_double_and_jumps(
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

fn generate_lion_like_double_and_jumps(
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct IllegalMove(pub Move);

impl fmt::Display for IllegalMove {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "the move is not legal: {:?}", self.0)
    }
}

impl std::error::Error for IllegalMove {}

impl Position {
    pub fn try_make_move(
        &mut self,
        mv: Move,
        generator: &MoveGenerator,
    ) -> Result<crate::Undo, IllegalMove> {
        let mut moves = Vec::new();
        generator.generate_moves(self, &mut moves);
        if moves.contains(&mv) {
            Ok(self.make_move_unchecked(mv))
        } else {
            Err(IllegalMove(mv))
        }
    }
}

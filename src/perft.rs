use crate::core::movegen::MoveGenerator;
use crate::core::position::Position;

/// Counts legal-move paths to `depth` and restores `position` before returning.
pub fn perft(generator: &MoveGenerator, position: &mut Position, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }

    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
        .into_iter()
        .map(|mv| {
            let undo = position.make_move_unchecked(mv);
            let nodes = perft(generator, position, depth - 1);
            position.unmake_move(undo);
            nodes
        })
        .sum()
}

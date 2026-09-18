//! 設計書predecessor-generator.md「検証」の固定テストと通常移動の逆包含。

mod input;
mod material;
mod movement;
mod promotion;
mod properties;

use std::collections::HashSet;

use crate::test_util::{position, position_from_codes, sq};
use crate::{
    Color, Move, MoveGenerator, MoveRules, PieceCode, PieceKind, Position, PositionBuilder,
    PredecessorError, PredecessorGenerator, PromotionRule, Square,
};

/// 通常着手を組み立てる。
fn mv(from: Square, to: Square, promote: bool) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote,
    }
}

/// 設計書「検証」の健全性・一意性・入力不変性を公開APIだけで検査する。
fn checked(rules: MoveRules, target: &Position) -> Vec<Position> {
    let saved = target.clone();
    let result = PredecessorGenerator::new(rules)
        .generate_predecessors(target)
        .unwrap();
    assert_eq!(*target, saved);
    let forward = MoveGenerator::new(rules);
    let mut seen = HashSet::new();
    for predecessor in &result {
        assert_eq!(predecessor.validate(), Ok(()));
        assert_eq!(predecessor.side_to_move(), target.side_to_move().opposite());
        assert!(seen.insert(predecessor));
        let mut moves = Vec::new();
        forward.generate_moves(predecessor, &mut moves);
        assert!(
            moves.into_iter().any(|mv| {
                let mut replayed = predecessor.clone();
                replayed.try_make_move(mv, &forward).unwrap();
                replayed == *target
            }),
            "returned predecessor has no legal edge to target"
        );
    }
    result
}

/// 順方向に作った辺の逆包含と、返却集合全体の健全性を確認する。
fn round_trip(rules: MoveRules, predecessor: &Position, mv: Move) -> Vec<Position> {
    let mut target = predecessor.clone();
    target
        .try_make_move(mv, &MoveGenerator::new(rules))
        .unwrap();
    let result = checked(rules, &target);
    assert!(
        result.contains(predecessor),
        "missing predecessor for {mv:?}"
    );
    result
}

/// 保留状態を指定した局面を公開の構築経路で作る。
fn deferred_position(side: Color, pieces: &[(Square, PieceCode)], deferred: &[Square]) -> Position {
    let mut builder = PositionBuilder::new(side);
    for &(square, piece) in pieces {
        builder.put(square, piece).unwrap();
    }
    for &square in deferred {
        builder.mark_promotion_deferred(square).unwrap();
    }
    builder.finish().unwrap()
}

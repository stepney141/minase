//! 設計書predecessor-generator.md「検証」の固定テストと順方向辺の逆包含。

mod completeness;
mod deferred;
mod input;
mod lion_movement;
mod lion_state;
mod material;
mod movement;
mod oracle;
mod profile;
mod promotion;

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

/// 設計書「検証」の健全性・一意性・入力不変性を順方向生成器で検査する。
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
        assert_admissible(rules, predecessor);
        assert_eq!(predecessor.side_to_move(), target.side_to_move().opposite());
        assert!(seen.insert(predecessor));
        let mut moves = Vec::new();
        forward.generate_moves(predecessor, &mut moves);
        assert!(
            moves.into_iter().any(|mv| {
                let mut replayed = predecessor.clone();
                // この局面・規則の全合法手を上で生成済みなので、そのまま適用できる。
                // 全辺の検査でも、着手ごとに同じ合法手集合を再生成しない。
                replayed.make_move_unchecked(mv, rules);
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

/// 獅子力の固定テストで捕獲候補を限定するため、相手側の残り在庫を盤端へ置く。
///
/// 指定した駒の由来を数え、初期在庫との差を補う。指定した中央の移動経路は空ける。
fn stocked_position(side: Color, pieces: &[(Square, PieceCode)]) -> Position {
    let mut all = pieces.to_vec();
    let opponent = side.opposite();
    let initial = Position::initial();
    let mut stock = Vec::new();
    for square in initial.pieces_of(opponent).iter() {
        stock.push(initial.piece_at(square).unwrap());
    }
    for &(_, piece) in pieces {
        if piece.color() == Some(opponent) {
            let kind = piece.kind().unwrap();
            let origin = if piece.is_promoted() {
                kind.unpromoted().unwrap()
            } else {
                kind
            };
            let index = stock.iter().position(|p| p.kind() == Some(origin)).unwrap();
            stock.remove(index);
        }
    }
    // 先手の試験では後手陣の最奥側4段、後手では先手陣の最奥側4段を使う。
    let mut squares = Square::all().filter(|square| {
        let rank = square.rank();
        (if side == Color::Black {
            rank < 4
        } else {
            rank >= 8
        }) && !pieces.iter().any(|(occupied, _)| occupied == square)
    });
    for piece in stock {
        all.push((squares.next().expect("stock fits in four ranks"), piece));
    }
    position_from_codes(side, &all)
}

/// 初期配置から指定升の駒を除き、指定した駒を置いた局面を作る。
///
/// 両対局者の在庫をほぼ満たしたまま試験駒だけを動かすことで、復元する捕獲駒と
/// 記録升の変種を少数に保つ。初期配置の走り駒は歩兵の列に遮られるため、除いた
/// 歩兵が盤端の筋である限り、中央の試験駒へ利きが届かない。
fn initial_with(side: Color, removed: &[Square], placed: &[(Square, PieceCode)]) -> Position {
    let initial = Position::initial();
    let mut pieces: Vec<_> = initial
        .occupied()
        .iter()
        .filter(|square| !removed.contains(square))
        .map(|square| (square, initial.piece_at(square).unwrap()))
        .collect();
    pieces.extend_from_slice(placed);
    position_from_codes(side, &pieces)
}

/// 試験局面の盤面を保ち、指定した保留集合と先獅子記録で再構築する。
fn with_state(base: &Position, deferred: &[Square], record: Option<Square>) -> Position {
    let pieces: Vec<_> = base
        .occupied()
        .iter()
        .map(|s| (s, base.piece_at(s).unwrap()))
        .collect();
    let mut result = deferred_position(base.side_to_move(), &pieces, deferred);
    result.set_lion_capture(record).unwrap();
    result
}

/// 駒種の定義に従って、不成駒または成り専用の駒を作る。
fn piece(color: Color, kind: PieceKind) -> PieceCode {
    if matches!(kind, PieceKind::HornedFalcon | PieceKind::SoaringEagle) {
        PieceCode::new_promoted(color, kind).unwrap()
    } else {
        PieceCode::new(color, kind).unwrap()
    }
}

/// 設計書「直前局面の定義」の集合Aを製品の所属検査から独立に確認する。
fn assert_admissible(rules: MoveRules, p: &Position) {
    let mut origins = std::collections::HashMap::new();
    for square in p.occupied().iter() {
        let pc = p.piece_at(square).unwrap();
        let kind = pc.kind().unwrap();
        let origin = if pc.is_promoted() {
            kind.unpromoted().unwrap()
        } else {
            kind
        };
        *origins.entry((pc.color().unwrap(), origin)).or_insert(0) += 1;
    }
    // RULES.md第5条の配置から独立に記した由来別枚数。
    for (&(_, kind), &count) in &origins {
        let maximum = match kind {
            PieceKind::Pawn => 12,
            PieceKind::King
            | PieceKind::DrunkElephant
            | PieceKind::FreeKing
            | PieceKind::Kirin
            | PieceKind::Phoenix
            | PieceKind::Lion => 1,
            PieceKind::GoBetween
            | PieceKind::Lance
            | PieceKind::ReverseChariot
            | PieceKind::SideMover
            | PieceKind::VerticalMover
            | PieceKind::Bishop
            | PieceKind::Rook
            | PieceKind::DragonHorse
            | PieceKind::DragonKing
            | PieceKind::FerociousLeopard
            | PieceKind::BlindTiger
            | PieceKind::CopperGeneral
            | PieceKind::SilverGeneral
            | PieceKind::GoldGeneral => 2,
            _ => panic!("promoted-only kind cannot be an origin"),
        };
        assert!(count <= maximum);
    }
    let mut waiting = [0; 2];
    for square in p.promotion_deferred().iter() {
        let pc = p.piece_at(square).unwrap();
        let color = pc.color().unwrap();
        assert!(!pc.is_promoted() && pc.kind().unwrap().can_promote());
        assert!(if color == Color::Black {
            square.rank() >= 8
        } else {
            square.rank() <= 3
        });
        if rules.promotion == PromotionRule::P0 {
            assert!(rules.p5 && pc.kind() == Some(PieceKind::Pawn));
        }
        if !(rules.p5 && pc.kind() == Some(PieceKind::Pawn)) {
            waiting[color.index()] += 1;
        }
    }
    if rules.promotion == PromotionRule::P2 {
        assert!(waiting.into_iter().all(|n| n <= 1));
    }
    if let Some(square) = p.lion_capture_square() {
        let mover = p.side_to_move();
        let lion_count = origins.get(&(mover, PieceKind::Lion)).copied().unwrap_or(0)
            + origins
                .get(&(mover, PieceKind::Kirin))
                .copied()
                .unwrap_or(0);
        assert!(lion_count < 2);
        if let Some(pc) = p.piece_at(square) {
            assert_eq!(pc.color(), Some(mover.opposite()));
            assert!(pc.kind() != Some(PieceKind::Lion) || pc.is_promoted());
        } else {
            assert!(
                !p.pieces_of_kind(mover.opposite(), PieceKind::HornedFalcon)
                    .is_empty()
                    || !p
                        .pieces_of_kind(mover.opposite(), PieceKind::SoaringEagle)
                        .is_empty()
            );
        }
    }
}

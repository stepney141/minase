//! 設計書predecessor-generator.md「検証」の公開APIによる逆方向探索。

use std::collections::HashSet;

use minase::notation::sfen::{SetupPosition, parse_extended_sfen, to_extended_sfen};
use minase::{
    Color, Move, MoveGenerator, MoveRules, PieceCode, PieceKind, Position, PositionBuilder,
    PredecessorGenerator, RuleCode::*, Rules, Square,
};

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn mv(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn round_trip(position: &Position, rules: MoveRules, next: u32) -> Position {
    let setup = SetupPosition::new(position.clone(), position.lion_capture_square(), next).unwrap();
    let text = to_extended_sfen(&setup);
    let (mut parsed, record, number) = parse_extended_sfen(&text, rules).unwrap().into_parts();
    assert_eq!(number, next);
    parsed.set_lion_capture(record).unwrap();
    assert_eq!(&parsed, position, "{text}");
    parsed
}

#[test]
fn predecessor_search_depth_two_from_extended_sfen() {
    // 設計書「検証」: 外部利用者と同じ入出力経路から初期局面まで2手逆行する。
    let rules = MoveRules::standard();
    let forward = MoveGenerator::standard();
    let reverse = PredecessorGenerator::standard();
    let initial = Position::initial();
    let mut target = initial.clone();
    target
        .try_make_move(mv(sq(0, 3), sq(0, 4)), &forward)
        .unwrap();
    target
        .try_make_move(mv(sq(0, 8), sq(0, 7)), &forward)
        .unwrap();
    let mut frontier = HashSet::from([round_trip(&target, rules, 3)]);
    let mut seen = frontier.clone();
    for next in [2, 1] {
        let mut previous = HashSet::new();
        for q in frontier {
            for p in reverse.generate_predecessors(&q).unwrap() {
                let parsed = round_trip(&p, rules, next);
                if seen.insert(parsed.clone()) {
                    previous.insert(parsed);
                }
            }
        }
        frontier = previous;
    }
    assert!(frontier.contains(&initial));
}

#[test]
fn predecessor_search_extended_sfen_preserves_transient_states() {
    // 設計書「検証」「Positionの公開面」: 実際の着手で作った先獅子とP1保留を保存する。
    let rules = Rules::from_codes(&[L0, P1, R1, E0]).unwrap().moves;
    let forward = MoveGenerator::new(rules);
    for (kind, from, to, captured) in [
        (PieceKind::Rook, sq(5, 5), sq(5, 6), Some(PieceKind::Lion)),
        (PieceKind::Pawn, sq(5, 7), sq(5, 8), None),
    ] {
        let mut builder = PositionBuilder::new(Color::Black);
        for (s, color, kind) in [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (from, Color::Black, kind),
        ] {
            builder
                .put(s, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        if let Some(kind) = captured {
            builder
                .put(to, PieceCode::new(Color::White, kind).unwrap())
                .unwrap();
        }
        let mut target = builder.finish().unwrap();
        target.try_make_move(mv(from, to), &forward).unwrap();
        if captured.is_some() {
            assert_eq!(target.lion_capture_square(), Some(to));
        } else {
            assert!(target.promotion_deferred().contains(to));
        }
        round_trip(&target, rules, 2);
    }
}

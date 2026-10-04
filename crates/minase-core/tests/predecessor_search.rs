//! 設計書predecessor-generator.md「検証」の公開APIによる逆方向探索。

use std::collections::HashSet;

use minase_core::notation::sfen::{SetupPosition, parse_extended_sfen, to_extended_sfen};
use minase_core::{Move, MoveGenerator, MoveRules, Position, PredecessorGenerator, Square};

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

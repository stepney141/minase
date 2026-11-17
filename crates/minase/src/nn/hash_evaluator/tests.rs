use super::*;
use crate::diagnostics::bench::BENCH_POSITIONS;
use minase_core::{Game, Rules, parse_sfen};

const INITIAL_KEY: &str = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - -";

#[test]
fn fnv1a_matches_published_vectors() {
    for (text, expected) in [
        ("", 0xcbf29ce484222325),
        ("a", 0xaf63dc4c8601ec8c),
        ("foobar", 0x85944171f73967e8),
    ] {
        assert_eq!(fnv1a(text), expected);
    }
}

#[test]
fn initial_value_and_logits_follow_independent_fnv_calculation() {
    let position = Position::initial();
    assert_eq!(position_key(&position), INITIAL_KEY);
    let game = Game::from_position(Rules::JCSA, position);
    let moves: Vec<_> = ["6i6h", "7i7h"]
        .iter()
        .map(|text| {
            game.legal_moves()
                .into_iter()
                .find(|&mv| usi::text_generated(game.position(), mv) == *text)
                .unwrap()
        })
        .collect();
    let mut logits = vec![99.0];
    let mut values = vec![99.0];
    HashEvaluator
        .evaluate(
            &[Query {
                position: game.position(),
                moves: &moves,
            }],
            &mut logits,
            &mut values,
        )
        .unwrap();
    // FNV-1aの64ビットをUTF-8のバイト列に適用し、上位24ビットを2²⁴で割る。
    // 設計書の外でPythonの数行で計算した整数。3uの丸めはf32の乗算1回だけ。
    assert_eq!(values, [12_690_386.0 / 16_777_216.0]);
    assert_eq!(
        logits,
        [
            3.0_f32 * (2_296_928.0 / 16_777_216.0),
            3.0_f32 * (8_863_004.0 / 16_777_216.0)
        ]
    );
}

#[test]
fn batches_and_move_permutations_preserve_each_evaluation() {
    let first = Game::from_position(Rules::JCSA, Position::initial());
    let second = Game::from_position(Rules::JCSA, parse_sfen(BENCH_POSITIONS[1].sfen).unwrap());
    let moves = first.legal_moves();
    let others = second.legal_moves();
    let mut evaluator = HashEvaluator;
    let mut logits = Vec::new();
    let mut values = Vec::new();
    evaluator
        .evaluate(
            &[Query {
                position: first.position(),
                moves: &moves,
            }],
            &mut logits,
            &mut values,
        )
        .unwrap();
    let expected = logits.clone();
    let value = values[0];
    for order in [[0, 1, 0], [1, 0, 0], [0, 0, 1]] {
        let queries: Vec<_> = order
            .iter()
            .map(|index| {
                if *index == 0 {
                    Query {
                        position: first.position(),
                        moves: &moves,
                    }
                } else {
                    Query {
                        position: second.position(),
                        moves: &others,
                    }
                }
            })
            .collect();
        evaluator
            .evaluate(&queries, &mut logits, &mut values)
            .unwrap();
        let mut offset = 0;
        for (index, query) in queries.iter().enumerate() {
            if order[index] == 0 {
                assert_eq!(values[index], value);
                assert_eq!(&logits[offset..offset + moves.len()], &expected);
            }
            offset += query.moves.len();
        }
    }
    let reversed: Vec<_> = moves.iter().rev().copied().collect();
    evaluator
        .evaluate(
            &[Query {
                position: first.position(),
                moves: &reversed,
            }],
            &mut logits,
            &mut values,
        )
        .unwrap();
    assert_eq!(logits, expected.into_iter().rev().collect::<Vec<_>>());
    assert_eq!(values, [value]);
}

#[test]
fn all_bench_moves_have_bounded_logits_and_values() {
    for entry in BENCH_POSITIONS {
        let game = Game::from_position(Rules::JCSA, parse_sfen(entry.sfen).unwrap());
        let moves = game.legal_moves();
        let mut logits = Vec::new();
        let mut values = Vec::new();
        HashEvaluator
            .evaluate(
                &[Query {
                    position: game.position(),
                    moves: &moves,
                }],
                &mut logits,
                &mut values,
            )
            .unwrap();
        assert_eq!(logits.len(), moves.len());
        assert_eq!(values.len(), 1);
        assert!(logits.iter().all(|value| (0.0..3.0).contains(value)));
        assert!((0.0..1.0).contains(&values[0]));
    }
}

#[test]
fn empty_batches_clear_outputs_and_empty_moves_keep_value() {
    let mut logits = vec![1.0];
    let mut values = vec![1.0];
    HashEvaluator
        .evaluate(&[], &mut logits, &mut values)
        .unwrap();
    assert!(logits.is_empty());
    assert!(values.is_empty());
    logits.push(1.0);
    values.push(1.0);
    HashEvaluator
        .evaluate(
            &[Query {
                position: &Position::initial(),
                moves: &[],
            }],
            &mut logits,
            &mut values,
        )
        .unwrap();
    assert!(logits.is_empty());
    assert_eq!(values, [12_690_386.0 / 16_777_216.0]);
}

#[test]
fn key_includes_lion_capture_and_omits_move_number() {
    let board = INITIAL_KEY.split_whitespace().next().unwrap();
    let restore = |capture, number| {
        sfen::parse_extended_sfen(
            &format!("{board} b {capture} {number} -"),
            Rules::JCSA.moves,
        )
        .unwrap()
        .restore_position()
        .unwrap()
    };
    let plain = restore("-", 1);
    let captured = restore("7f", 1);
    assert_eq!(position_key(&captured), format!("{board} b 7f -"));
    assert_ne!(
        unit_hash(&position_key(&plain)),
        unit_hash(&position_key(&captured))
    );
    assert_eq!(position_key(&plain), position_key(&restore("-", 99)));
}

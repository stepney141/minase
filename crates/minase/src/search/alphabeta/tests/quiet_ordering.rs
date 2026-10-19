//! M3bで加える静かな手の並べ替え値と、価値の低い駒による脅威の数値例。

use super::*;
use crate::search::alphabeta::history::LOW_PLY_HISTORY_PLY_COUNT;
use crate::search::alphabeta::ordering::{
    LesserPieceThreats, QuietOrderingCoefficients, QuietOrderingInput, continuation_index,
    lesser_threat_delta, qualifies_for_check_bonus, quiet_ordering_value,
};
use minase_core::Bitboard;

#[test]
fn lesser_piece_threats_group_equal_values_and_use_strictly_lower_groups() {
    let pawn_a = sq(3, 3);
    let pawn_b = sq(8, 8);
    let rook = sq(5, 5);
    let king = sq(11, 11);
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (pawn_a, Color::White, PieceKind::Pawn),
            (pawn_b, Color::White, PieceKind::Pawn),
            (rook, Color::White, PieceKind::Rook),
            (king, Color::White, PieceKind::King),
        ],
    );
    let threats = LesserPieceThreats::with_piece_values(&board, |piece| {
        match piece.kind().expect("board piece has a kind") {
            PieceKind::Pawn => 10,
            PieceKind::Rook => 20,
            PieceKind::King => 40,
            kind => panic!("unexpected piece kind: {kind:?}"),
        }
    });
    let pawns = board.piece_control(pawn_a) | board.piece_control(pawn_b);
    let through_rook = pawns | board.piece_control(rook);
    let all = through_rook | board.piece_control(king);

    assert_eq!(
        threats.groups,
        vec![(10, pawns), (20, through_rook), (40, all)]
    );
    assert_eq!(threats.below(10), Bitboard::EMPTY);
    assert_eq!(threats.below(20), pawns);
    assert_eq!(threats.below(40), through_rook);
    assert_eq!(threats.below(41), all);
}

#[test]
fn royal_move_has_no_lesser_piece_threat_term() {
    let mv = step(sq(5, 5), sq(5, 6));
    let both = Bitboard::from_squares([mv.from, mv.to]);
    let from_only = Bitboard::from_square(mv.from);

    assert_eq!(lesser_threat_delta(PieceKind::King, both, mv), 0);
    assert_eq!(lesser_threat_delta(PieceKind::CrownPrince, both, mv), 0);
    assert_eq!(lesser_threat_delta(PieceKind::Rook, from_only, mv), 1);
}

#[test]
fn check_bonus_requires_a_check_whose_see_is_not_below_the_margin() {
    let nnue = test_nnue();
    let generator = MoveGenerator::standard();
    let losing = position(
        Color::Black,
        &[
            (sq(11, 11), Color::Black, PieceKind::King),
            (sq(4, 4), Color::Black, PieceKind::Rook),
            (sq(5, 0), Color::White, PieceKind::King),
            (sq(5, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let losing_check = step(sq(4, 4), sq(5, 4));
    assert!(legal_moves(&losing).contains(&losing_check));
    assert!(losing.gives_check(&generator, losing_check));
    assert!(see_prunes(
        &losing,
        engine_rules(),
        nnue,
        losing_check,
        nnue.pawn_value() * 75 / 208,
    ));
    assert!(!qualifies_for_check_bonus(
        &losing,
        &generator,
        nnue,
        losing_check,
    ));

    let safe = position(
        Color::Black,
        &[
            (sq(11, 11), Color::Black, PieceKind::King),
            (sq(4, 4), Color::Black, PieceKind::Rook),
            (sq(5, 0), Color::White, PieceKind::King),
        ],
    );
    let safe_check = step(sq(4, 4), sq(5, 4));
    assert!(legal_moves(&safe).contains(&safe_check));
    assert!(!see_prunes(
        &safe,
        engine_rules(),
        nnue,
        safe_check,
        nnue.pawn_value() * 75 / 208,
    ));
    assert!(qualifies_for_check_bonus(
        &safe, &generator, nnue, safe_check,
    ));

    let unknown = position(
        Color::Black,
        &[
            (sq(11, 11), Color::Black, PieceKind::King),
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(5, 1), Color::White, PieceKind::King),
            (sq(5, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let unknown_check = Move {
        from: sq(5, 5),
        mid: Some(sq(5, 4)),
        to: sq(5, 3),
        promote: false,
    };
    assert!(legal_moves(&unknown).contains(&unknown_check));
    assert!(unknown.gives_check(&generator, unknown_check));
    assert!(!see_prunes(
        &unknown,
        engine_rules(),
        nnue,
        unknown_check,
        nnue.pawn_value() * 75 / 208,
    ));
    assert!(qualifies_for_check_bonus(
        &unknown,
        &generator,
        nnue,
        unknown_check,
    ));
}

#[test]
fn quiet_ordering_value_matches_fixed_history_and_coefficient_examples() {
    let from = sq(4, 4);
    let to = sq(4, 5);
    let mut low_ply =
        Box::new([[[0_i16; BOARD_SQUARE_COUNT]; BOARD_SQUARE_COUNT]; LOW_PLY_HISTORY_PLY_COUNT]);
    low_ply[2][from.dense_index()][to.dense_index()] = -7;
    low_ply[4][from.dense_index()][to.dense_index()] = 7;
    let coefficients = QuietOrderingCoefficients {
        check_bonus: 103,
        threat_weight: 37,
    };
    let input = QuietOrderingInput {
        butterfly: 11,
        pawn: -7,
        continuation: [3, -4, 5, 0, 7].into_iter().sum(),
        ply: 2,
        from,
        to,
        check: false,
        moving_piece_value: 275,
        pawn_value: 100,
        threat_delta: 0,
    };

    // 2×11 + 2×(−7) + 11 + trunc(8×(−7)/3) = 1。
    assert_eq!(quiet_ordering_value(input, &low_ply, coefficients), 1);
    // 王手の103を加える。
    let checking = QuietOrderingInput {
        check: true,
        ..input
    };
    assert_eq!(quiet_ordering_value(checking, &low_ply, coefficients), 104);
    // trunc(275×37/100) = 101を、脅威へ入る手として引く。
    assert_eq!(
        quiet_ordering_value(
            QuietOrderingInput {
                threat_delta: -1,
                ..checking
            },
            &low_ply,
            coefficients,
        ),
        3
    );
    // ply 4ではtrunc(8×7/5) = 11を加え、ply 5ではlow ply historyを加えない。
    assert_eq!(
        quiet_ordering_value(
            QuietOrderingInput { ply: 4, ..input },
            &low_ply,
            coefficients,
        ),
        30
    );
    assert_eq!(
        quiet_ordering_value(
            QuietOrderingInput { ply: 5, ..input },
            &low_ply,
            coefficients,
        ),
        19
    );
}

#[test]
fn move_picker_reads_butterfly_pawn_and_all_available_continuations() {
    let first = step(sq(4, 4), sq(4, 5));
    let second = step(sq(6, 4), sq(6, 5));
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (first.from, Color::Black, PieceKind::Pawn),
            (second.from, Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    assert!(legal_moves(&board).contains(&first));
    assert!(legal_moves(&board).contains(&second));

    with_root_searcher(&board, &[], |searcher| {
        let contexts = [
            PieceKind::Pawn,
            PieceKind::Lance,
            PieceKind::ReverseChariot,
            PieceKind::Bishop,
            PieceKind::Rook,
            PieceKind::FreeKing,
        ];
        searcher.path_moves = contexts
            .into_iter()
            .enumerate()
            .map(|(index, kind)| PathMove {
                record: Some(MoveRecord {
                    mv: step(sq(index as u8, 1), sq(index as u8, 2)),
                    piece: PieceCode::new(Color::White, kind).unwrap(),
                    captured: None,
                }),
                attacking: None,
            })
            .collect();
        searcher.path_keys = vec![0; searcher.path_moves.len() + 1];
        let ply = 6;
        let first_piece = board.piece_at(first.from).unwrap();
        let second_piece = board.piece_at(second.from).unwrap();
        searcher.history[Color::Black.index()][first.from.dense_index()][first.to.dense_index()] =
            11;
        searcher.history[Color::Black.index()][second.from.dense_index()]
            [second.to.dense_index()] = 0;
        searcher.pawn_keys[ply as usize] = 0x1234;
        searcher
            .pawn_history
            .store(0x1234, first_piece, first.to, -7);
        searcher
            .pawn_history
            .store(0x1234, second_piece, second.to, 0);
        for (distance, value) in [(1, 3), (2, -4), (3, 5), (4, 0), (6, 7)] {
            let context = searcher.path_moves[ply as usize - distance].record.unwrap();
            searcher.continuation_history.store(
                continuation_index(context, first_piece, first, distance),
                value,
            );
            searcher.continuation_history.store(
                continuation_index(context, second_piece, second, distance),
                0,
            );
        }

        let mut context = searcher_move_picker_context(searcher, ply);
        context.coefficients = QuietOrderingCoefficients {
            check_bonus: 0,
            threat_weight: 0,
        };
        let mut picker = MovePicker::new(None, [None; KILLER_COUNT]);
        let ordered: Vec<_> =
            std::iter::from_fn(|| picker.next(&board, searcher.nnue, &searcher.generator, context))
                .map(|(mv, _)| mv)
                .collect();
        let first_index = ordered.iter().position(|&mv| mv == first).unwrap();
        let second_index = ordered.iter().position(|&mv| mv == second).unwrap();
        assert!(first_index < second_index);
    });
}

//! 駒の生成順によらない裁定と、各群の駒による詰み回避を確かめる。

use super::oracle::{
    oracle_can_capture_last_royal, oracle_candidate_immediate_result, oracle_has_no_legal_move,
    oracle_is_mate,
};
use super::*;
use crate::board::Bitboard;
use crate::game::repetition::repetition_is_forbidden;

/// 群を分けずに駒種順で生成し、候補の裁定には全件生成の参照実装を使う。
fn in_piece_kind_order(
    position: &mut Position,
    context: &AdjudicationContext<'_>,
) -> Option<WinReason> {
    let mover = position.side_to_move();
    let mut base_moves = Vec::new();
    let mut moves = Vec::new();
    let mut has_legal_move = false;
    for kind in PieceKind::ALL {
        for from in position.pieces_of_kind(mover, kind) {
            moves.clear();
            context
                .generator
                .generate_moves_from(position, from, &mut base_moves, &mut moves);
            for &mv in &moves {
                let captures_royals = mate::captures_last_royal(position, mv);
                let undo = position.make_move_unchecked(mv, context.rules.moves);
                let allowed = !repetition_is_forbidden(context.state.repetition(), position);
                has_legal_move |= allowed;
                let escapes = allowed
                    && (context.rules.e1
                        || captures_royals
                        || match oracle_candidate_immediate_result(context, position, mv, &undo) {
                            Some(GameResult::Win { winner, .. }) => winner == mover,
                            Some(GameResult::Draw { .. }) => true,
                            None => !oracle_can_capture_last_royal(position, context.generator),
                        });
                position.unmake_move(undo);
                if escapes {
                    return None;
                }
            }
        }
    }
    Some(if has_legal_move {
        WinReason::Mate
    } else {
        WinReason::Stalemate
    })
}

fn assert_orders(
    position: &mut Position,
    context: &AdjudicationContext<'_>,
    expected: Option<WinReason>,
) {
    let before = position.clone();
    let full = if oracle_has_no_legal_move(position, context.generator, context.state.repetition())
    {
        Some(WinReason::Stalemate)
    } else if !context.rules.e1 && oracle_is_mate(position, context) {
        Some(WinReason::Mate)
    } else {
        None
    };
    assert_eq!(full, expected, "full generation");
    assert_eq!(*position, before);
    assert_eq!(
        in_piece_kind_order(position, context),
        expected,
        "piece kind order"
    );
    assert_eq!(*position, before);
    assert_eq!(
        stalemate_or_mate(position, context),
        expected,
        "piece group order"
    );
    assert_eq!(*position, before);
}

/// 逃れる手が指定した駒に限られることを参照版で確かめ、両方の順序で裁定する。
fn assert_only_escaping_piece(
    position: &mut Position,
    context: &AdjudicationContext<'_>,
    witness: Move,
) {
    let before = position.clone();
    let mover = position.side_to_move();
    let mut moves = Vec::new();
    context.generator.generate_moves(position, &mut moves);
    let mut escapes = Vec::new();
    for mv in moves {
        let captures_royals = mate::captures_last_royal(position, mv);
        let undo = position.make_move_unchecked(mv, context.rules.moves);
        let escapes_mate = !repetition_is_forbidden(context.state.repetition(), position)
            && (captures_royals
                || match oracle_candidate_immediate_result(context, position, mv, &undo) {
                    Some(GameResult::Win { winner, .. }) => winner == mover,
                    Some(GameResult::Draw { .. }) => true,
                    None => !oracle_can_capture_last_royal(position, context.generator),
                });
        position.unmake_move(undo);
        if escapes_mate {
            escapes.push(mv);
        }
    }
    assert!(escapes.contains(&witness));
    assert!(escapes.iter().all(|mv| mv.from == witness.from));
    assert_eq!(*position, before);
    assert_orders(position, context, None);
}

/// 飛車が王手し、金将が王将の横と斜めの逃走先を塞ぐ。
fn checked_corner(extra: &[(Square, PieceCode)]) -> Position {
    let mut pieces = vec![
        (sq(0, 0), piece(Color::Black, PieceKind::King)),
        (sq(0, 5), piece(Color::White, PieceKind::Rook)),
        (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
        (sq(11, 11), piece(Color::White, PieceKind::King)),
    ];
    pieces.extend_from_slice(extra);
    position(Color::Black, &pieces)
}

/// 第21条・第23条・第32条E1。継続、詰み、合法手なしの結果は生成順で変わらず、
/// E1を有効にすると詰みだけが継続になる。
#[test]
fn articles_21_23_32_piece_group_order_matches_piece_kind_order() {
    let immobile = position(
        Color::Black,
        &[
            (sq(0, 11), piece(Color::Black, PieceKind::King)),
            (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(10, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    for (board, expected) in [
        (Position::initial(), None),
        (checked_corner(&[]), Some(WinReason::Mate)),
        (immobile, Some(WinReason::Stalemate)),
    ] {
        for e1 in [false, true] {
            let rules = Rules {
                exhaustion: ExhaustionRule::E2,
                e1,
                ..Rules::ENGINE_DEFAULT
            };
            let generator = MoveGenerator::new(rules.moves);
            let state = r1_state(&board);
            let context = AdjudicationContext::new(rules, &state, &generator);
            let expected = if e1 && expected == Some(WinReason::Mate) {
                None
            } else {
                expected
            };
            assert_orders(&mut board.clone(), &context, expected);
        }
    }
}

/// 第21条第3項a。第1群の王将を王手の筋から移すと詰みを回避できる。
#[test]
fn article_21_3_first_group_escapes_by_moving_a_royal() {
    let mut board = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules = Rules {
        exhaustion: ExhaustionRule::E2,
        ..Rules::ENGINE_DEFAULT
    };
    let generator = MoveGenerator::new(rules.moves);
    let state = r1_state(&board);
    assert_only_escaping_piece(
        &mut board,
        &AdjudicationContext::new(rules, &state, &generator),
        step(sq(0, 0), sq(1, 0)),
    );
}

/// 第21条第3項a。王将には逃走先がなく、第2群の飛車が王手した飛車を取って逃れる。
#[test]
fn article_21_3_second_group_escapes_by_capturing_the_attacker() {
    let mut board = checked_corner(&[(sq(1, 5), piece(Color::Black, PieceKind::Rook))]);
    assert_eq!(
        board.attackers_to_by(Color::White, sq(0, 0), board.occupied()),
        Bitboard::from_square(sq(0, 5))
    );
    assert_eq!(
        board.attackers_to_by(Color::Black, sq(0, 5), board.occupied()),
        Bitboard::from_square(sq(1, 5))
    );
    let rules = Rules {
        exhaustion: ExhaustionRule::E2,
        ..Rules::ENGINE_DEFAULT
    };
    let generator = MoveGenerator::new(rules.moves);
    let state = r1_state(&board);
    assert_only_escaping_piece(
        &mut board,
        &AdjudicationContext::new(rules, &state, &generator),
        step(sq(1, 5), sq(0, 5)),
    );
}

/// 第21条第3項a。王将の移動と攻撃駒の捕獲では逃れられず、第3群の飛車が合駒する。
#[test]
fn article_21_3_third_group_escapes_by_interposing() {
    let mut board = checked_corner(&[(sq(1, 3), piece(Color::Black, PieceKind::Rook))]);
    assert_eq!(
        board.attackers_to_by(Color::White, sq(0, 0), board.occupied()),
        Bitboard::from_square(sq(0, 5))
    );
    assert!(
        board
            .attackers_to_by(Color::Black, sq(0, 5), board.occupied())
            .is_empty()
    );
    let rules = Rules {
        exhaustion: ExhaustionRule::E2,
        ..Rules::ENGINE_DEFAULT
    };
    let generator = MoveGenerator::new(rules.moves);
    let state = r1_state(&board);
    assert_only_escaping_piece(
        &mut board,
        &AdjudicationContext::new(rules, &state, &generator),
        step(sq(1, 3), sq(0, 3)),
    );
}

/// 第23条・第31条R2・R3・第32条E1。唯一の手が反復で禁止されると、
/// E1と生成順にかかわらず合法手なしとなる。R3では同一局面の3回目の出現までは継続する。
#[test]
fn articles_23_31_32_repetition_prohibition_preserves_results_in_both_orders() {
    let mut pieces = vec![
        (sq(0, 0), piece(Color::Black, PieceKind::GoBetween)),
        (sq(11, 11), piece(Color::Black, PieceKind::King)),
        (sq(10, 10), piece(Color::Black, PieceKind::Pawn)),
        (sq(10, 11), piece(Color::Black, PieceKind::Pawn)),
        (sq(11, 10), piece(Color::Black, PieceKind::Pawn)),
        (sq(5, 5), piece(Color::White, PieceKind::King)),
    ];
    pieces.extend((2..=11).map(|rank| (sq(0, rank), piece(Color::Black, PieceKind::Pawn))));
    let cycle = [
        step(sq(5, 5), sq(5, 4)),
        step(sq(0, 0), sq(0, 1)),
        step(sq(5, 4), sq(5, 5)),
        step(sq(0, 1), sq(0, 0)),
    ];
    for (repetition, plies, expected) in [
        (RepetitionRule::R2, 3, Some(WinReason::Stalemate)),
        (RepetitionRule::R3, 3, None),
        (RepetitionRule::R3, 7, None),
        (RepetitionRule::R3, 11, Some(WinReason::Stalemate)),
    ] {
        for e1 in [false, true] {
            let rules = Rules {
                repetition,
                exhaustion: ExhaustionRule::E2,
                e1,
                ..Rules::ENGINE_DEFAULT
            };
            let generator = MoveGenerator::new(rules.moves);
            let mut board = position(Color::White, &pieces);
            let mut state = AdjudicationState::new(repetition, &board);
            for mv in cycle.into_iter().cycle().take(plies) {
                let mover = board.side_to_move();
                let undo = board.try_make_move_with_undo(mv, &generator).unwrap();
                assert!(!repetition_is_forbidden(state.repetition(), &board));
                assert_eq!(
                    state.record_move(&board, &generator, mover, mv, &undo),
                    None
                );
            }
            assert_orders(
                &mut board,
                &AdjudicationContext::new(rules, &state, &generator),
                expected,
            );
        }
    }
}

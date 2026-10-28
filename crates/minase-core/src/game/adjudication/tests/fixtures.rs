//! 規則の条文から定めた期待値を、本番と全件生成の参照版の双方に課す固定例。

use super::super::bare_king::{bare_king_result, can_capture_any_royal};
use super::super::exhaustion::{PieceExhaustionOutcome, piece_exhaustion_outcome};
use super::super::mate::can_capture_last_royal;
use super::oracle::*;
use super::reference::{production_has_no_legal_move, production_is_mate};
use super::*;
use crate::game::result::DrawReason;
use crate::position::{PositionBuilder, Undo};
use crate::rules::RuleCode;
use std::cell::Cell;

fn promoted(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new_promoted(color, kind).unwrap()
}

fn generated(position: &Position, generator: &MoveGenerator) -> Vec<Move> {
    let mut moves = Vec::new();
    generator.generate_moves(position, &mut moves);
    moves
}

fn assert_last_royal(position: &Position, generator: &MoveGenerator, expected: bool) {
    assert_eq!(oracle_can_capture_last_royal(position, generator), expected);
    assert_eq!(can_capture_last_royal(position, generator), expected);
}

fn assert_any_royal(
    position: &Position,
    generator: &MoveGenerator,
    attacker: Color,
    expected: bool,
) {
    assert_eq!(
        oracle_can_capture_any_royal(position, generator, attacker),
        expected
    );
    assert_eq!(can_capture_any_royal(position, attacker), expected);
}

fn assert_exhaustion(
    position: &Position,
    generator: &MoveGenerator,
    grace: bool,
    expected: PieceExhaustionOutcome,
) {
    assert_eq!(
        oracle_piece_exhaustion_outcome(position, generator, grace),
        expected
    );
    assert_eq!(
        piece_exhaustion_outcome(position, generator, grace),
        expected
    );
}

/// RULES.md第8条第1項・第7項、第12条第4項、第20条第4項。
/// 太子だけでも最後の王駒であり、王駒2枚は1手で両方取る必要がある。
/// 3枚は最大捕獲数2枚を超え、0枚なら取るべき王駒がない。
#[test]
fn articles_8_12_20_last_royal_capture_counts_all_remaining_royals() {
    let generator = MoveGenerator::standard();
    let prince = promoted(Color::White, PieceKind::CrownPrince);
    let king = piece(Color::White, PieceKind::King);
    let cases = [
        (vec![(sq(5, 6), prince)], true),
        (vec![(sq(5, 6), king), (sq(6, 6), prince)], true),
        (vec![(sq(4, 7), king), (sq(6, 7), prince)], false),
        (
            vec![(sq(4, 6), king), (sq(5, 6), king), (sq(6, 6), prince)],
            false,
        ),
        (vec![], false),
    ];
    for (royals, expected) in cases {
        let mut pieces = vec![
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
        ];
        pieces.extend(royals);
        assert_last_royal(&position(Color::Black, &pieces), &generator, expected);
    }
}

/// RULES.md第11条第1項d・第2項・第4項、第12条第5項・第7項。
/// 自駒または第15条の先獅子で取れない獅子を跳び越して王駒を取れる。
#[test]
fn articles_11_12_15_royal_capture_jumps_over_uncapturable_blockers() {
    let generator = MoveGenerator::standard();
    for (attacker, middle, target) in [
        (piece(Color::Black, PieceKind::Lion), sq(5, 5), sq(5, 6)),
        (
            promoted(Color::Black, PieceKind::HornedFalcon),
            sq(5, 5),
            sq(5, 6),
        ),
        (
            promoted(Color::Black, PieceKind::SoaringEagle),
            sq(6, 5),
            sq(7, 6),
        ),
    ] {
        for blocker in [
            piece(Color::Black, PieceKind::Pawn),
            piece(Color::White, PieceKind::Lion),
        ] {
            let mut board = position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(5, 4), attacker),
                    (middle, blocker),
                    (target, piece(Color::White, PieceKind::King)),
                    (sq(11, 9), piece(Color::White, PieceKind::Rook)),
                ],
            );
            // 直前に非獅子が別の獅子を取った状態。中間の獅子には後ろの王将の足がある。
            board.set_lion_capture(Some(sq(11, 9))).unwrap();
            let moves = generated(&board, &generator);
            assert!(moves.contains(&step(sq(5, 4), target)));
            assert!(
                !moves
                    .iter()
                    .any(|&mv| board.captured_squares(mv).contains(&Some(middle)))
            );
            assert_last_royal(&board, &generator, true);
        }
    }
}

/// RULES.md第30条P5・P6は最奥段への成りを強制しても、王駒の捕獲手を消さない。
#[test]
fn article_30_p5_p6_forced_promotion_preserves_royal_capture() {
    for (kind, code, deferred) in [
        (PieceKind::Pawn, RuleCode::P5, true),
        (PieceKind::Lance, RuleCode::P6, false),
    ] {
        let rules =
            Rules::from_codes(&[RuleCode::L0, RuleCode::P0, code, RuleCode::R1, RuleCode::E0])
                .unwrap();
        let generator = MoveGenerator::new(rules.moves);
        let from = sq(5, 10);
        let to = sq(5, 11);
        let mut builder = PositionBuilder::new(Color::Black);
        for (square, code) in [
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (from, piece(Color::Black, kind)),
            (to, piece(Color::White, PieceKind::King)),
        ] {
            builder.put(square, code).unwrap();
        }
        if deferred {
            builder.mark_promotion_deferred(from).unwrap();
        }
        let board = builder.finish().unwrap();
        let moves = generated(&board, &generator);
        assert!(moves.contains(&Move {
            promote: true,
            ..step(from, to)
        }));
        assert!(!moves.contains(&step(from, to)));
        assert_last_royal(&board, &generator, true);
    }
}

/// RULES.md第19条第2項・第3項。最奥段の不成の歩兵・香車は近くの王駒を取れない。
#[test]
fn article_19_dead_pawns_and_lances_cannot_capture_a_royal() {
    let generator = MoveGenerator::standard();
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(6, 11), piece(Color::Black, PieceKind::Lance)),
            (sq(5, 10), piece(Color::White, PieceKind::King)),
        ],
    );
    assert_last_royal(&board, &generator, false);
    assert_any_royal(&board, &generator, Color::Black, false);
}

fn assert_attacking(mut board: Position, mv: Move, expected: bool) {
    let generator = MoveGenerator::standard();
    let mover = board.side_to_move();
    assert!(generated(&board, &generator).contains(&mv));
    board.make_move_unchecked(mv, generator.rules());
    assert_eq!(
        oracle_move_was_attacking(&board, &generator, mover, mv),
        expected
    );
    assert_eq!(move_was_attacking(&board, &generator, mover, mv), expected);
}

/// RULES.md第14条第2項・第3項、第31条R1。
/// 動かした獅子の唯一の捕獲対象が、隣接しない足のある獅子なら攻撃的着手ではない。
#[test]
fn articles_14_31_attacking_move_respects_lion_capture_restrictions() {
    for foot in [false, true] {
        let mut pieces = vec![
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 4), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 7), piece(Color::White, PieceKind::Lion)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ];
        if foot {
            pieces.push((sq(11, 7), piece(Color::White, PieceKind::Rook)));
        }
        assert_attacking(
            position(Color::Black, &pieces),
            step(sq(5, 4), sq(5, 5)),
            !foot,
        );
    }
}

/// RULES.md第12条第9項、第31条R1。じっとの後も元の升から捕獲できれば攻撃的である。
/// Moveの正準形では、空升を経由するじっとはmid: Noneで表す。
#[test]
fn articles_12_31_jitto_keeps_the_moved_pieces_capture_threat() {
    let board = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 7), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    assert_attacking(board, step(sq(5, 5), sq(5, 5)), true);
}

/// RULES.md第31条R1は動かさなかった駒による王駒への利きも数え、
/// 王将と太子の片方への利きだけでも攻撃的とする。他の駒への既存の利きは数えない。
#[test]
fn article_31_attacking_move_counts_any_royal_but_not_unmoved_nonroyal_threats() {
    for (check, second_royal, expected) in [
        (true, false, true),
        (true, true, true),
        (false, false, false),
    ] {
        let mut pieces = vec![
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(6, 3), piece(Color::Black, PieceKind::GoldGeneral)),
            (sq(2, 10), piece(Color::White, PieceKind::King)),
            (sq(0, 5), piece(Color::Black, PieceKind::Rook)),
            (sq(0, 8), piece(Color::White, PieceKind::Pawn)),
        ];
        if check {
            pieces.push((sq(2, 1), piece(Color::Black, PieceKind::Rook)));
        }
        if second_royal {
            pieces.push((sq(9, 10), promoted(Color::White, PieceKind::CrownPrince)));
        }
        assert_attacking(
            position(Color::Black, &pieces),
            step(sq(6, 3), sq(6, 4)),
            expected,
        );
    }
}

/// RULES.md第22条第1項・第5項・第7項。余分な獅子を王将で取れれば1手猶予、
/// 取れない場合または猶予消化後は余分な駒を持つ側が勝つ。
#[test]
fn article_22_lion_extra_piece_can_start_or_exhaust_the_grace() {
    let generator = MoveGenerator::standard();
    for (lion, expected) in [
        (sq(5, 6), PieceExhaustionOutcome::GraceStart),
        (sq(5, 8), PieceExhaustionOutcome::Win(Color::White)),
    ] {
        let board = position(
            Color::Black,
            &[
                (sq(5, 5), piece(Color::Black, PieceKind::King)),
                (lion, piece(Color::White, PieceKind::Lion)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        );
        assert_any_royal(&board, &generator, Color::Black, false);
        assert_any_royal(&board, &generator, Color::White, lion == sq(5, 6));
        assert_exhaustion(&board, &generator, false, expected);
        assert_exhaustion(
            &board,
            &generator,
            true,
            PieceExhaustionOutcome::Win(Color::White),
        );
    }
}

/// RULES.md第22条第4項・第5項・第9項。死に駒を除き、王将と太子のどちらを取る手も
/// 猶予に数える。双方が王駒1枚なら同条第8項の引き分けとなる。
#[test]
fn article_22_grace_counts_either_royal_and_ignores_dead_pieces() {
    let generator = MoveGenerator::standard();
    for adjacent_kind in [PieceKind::King, PieceKind::CrownPrince] {
        let other_kind = if adjacent_kind == PieceKind::King {
            PieceKind::CrownPrince
        } else {
            PieceKind::King
        };
        let royal = |kind| {
            if kind == PieceKind::King {
                piece(Color::White, kind)
            } else {
                promoted(Color::White, kind)
            }
        };
        let board = position(
            Color::Black,
            &[
                (sq(5, 5), piece(Color::Black, PieceKind::King)),
                (sq(5, 6), royal(adjacent_kind)),
                (sq(6, 7), royal(other_kind)),
                (sq(0, 11), piece(Color::Black, PieceKind::Pawn)),
                (sq(1, 11), piece(Color::Black, PieceKind::Lance)),
                (sq(0, 0), piece(Color::White, PieceKind::Pawn)),
                (sq(1, 0), piece(Color::White, PieceKind::Lance)),
            ],
        );
        assert_any_royal(&board, &generator, Color::Black, true);
        assert_last_royal(&board, &generator, false);
        assert_exhaustion(
            &board,
            &generator,
            false,
            PieceExhaustionOutcome::GraceStart,
        );
    }
    let board = position(
        Color::Black,
        &[
            (sq(5, 5), piece(Color::Black, PieceKind::King)),
            (sq(9, 9), piece(Color::White, PieceKind::King)),
            (sq(0, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(1, 0), piece(Color::White, PieceKind::Lance)),
        ],
    );
    for color in Color::ALL {
        assert_any_royal(&board, &generator, color, false);
    }
    for grace in [false, true] {
        assert_exhaustion(&board, &generator, grace, PieceExhaustionOutcome::Draw);
    }
}

/// RULES.md第32条E3。価値ある駒2枚の側から離れた裸玉は負けとなり、
/// 双方の裸玉が互いを取れなければ引き分ける。隣接する余分な駒は取り返せる。
#[test]
fn article_32_bare_king_outcomes_match_the_reference() {
    let generator = MoveGenerator::standard();
    for (extra, expected) in [
        (
            Some(sq(9, 8)),
            Some(GameResult::Win {
                winner: Color::White,
                reason: WinReason::BareKing,
            }),
        ),
        (Some(sq(5, 6)), None),
        (
            None,
            Some(GameResult::Draw {
                reason: DrawReason::BareKing,
            }),
        ),
    ] {
        let mut pieces = vec![
            (sq(5, 5), piece(Color::Black, PieceKind::King)),
            (sq(9, 9), piece(Color::White, PieceKind::King)),
            (sq(1, 11), piece(Color::Black, PieceKind::Lance)),
        ];
        if let Some(square) = extra {
            pieces.push((square, piece(Color::White, PieceKind::Rook)));
        }
        let board = position(Color::Black, &pieces);
        assert_eq!(oracle_bare_king_result(&board, &generator), expected);
        assert_eq!(bare_king_result(&board), expected);
    }
}

fn production_r1_candidate(
    state: &AdjudicationState,
    position: &Position,
    generator: &MoveGenerator,
    candidate: Move,
    undo: &Undo,
) -> Option<GameResult> {
    let RepetitionHistory::R1(history) = state.repetition() else {
        panic!("R1 fixture requires R1 history");
    };
    let mover = position.side_to_move().opposite();
    history.candidate_result(
        position,
        state.ply(),
        mover,
        || move_was_attacking(position, generator, mover, candidate),
        move_is_irreversible(candidate, undo),
    )
}

fn record_sequence(
    position: &mut Position,
    state: &mut AdjudicationState,
    generator: &MoveGenerator,
    moves: &[Move],
) {
    let mut reference_state = state.clone();
    for &mv in moves {
        let mover = position.side_to_move();
        assert!(generated(position, generator).contains(&mv));
        let undo = position.make_move_unchecked(mv, generator.rules());
        assert_eq!(
            state.record_move(position, generator, mover, mv, &undo),
            None
        );
        assert_eq!(
            oracle_record_move(&mut reference_state, position, generator, mover, mv, &undo),
            None
        );
        assert_eq!(*state, reference_state);
    }
}

fn assert_r1_candidate(
    mut position: Position,
    state: &AdjudicationState,
    generator: &MoveGenerator,
    candidate: Move,
    expected: Option<GameResult>,
) {
    assert!(generated(&position, generator).contains(&candidate));
    let mover = position.side_to_move();
    let undo = position.make_move_unchecked(candidate, generator.rules());
    assert_eq!(
        production_r1_candidate(state, &position, generator, candidate, &undo),
        expected
    );
    assert_eq!(
        oracle_record_move(
            &mut state.clone(),
            &position,
            generator,
            mover,
            candidate,
            &undo
        ),
        expected
    );
}

/// RULES.md第31条R1。可逆手12手かつ4回目が裁定の境界であり、
/// 不可逆手、候補込み11手、未出現、候補で3回目のそれぞれでは裁定しない。
#[test]
fn article_31_r1_candidate_gates_match_recording_a_cloned_history() {
    let generator = MoveGenerator::standard();
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(10, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let cycle = [
        step(sq(0, 0), sq(0, 1)),
        step(sq(11, 11), sq(11, 10)),
        step(sq(0, 1), sq(0, 0)),
        step(sq(11, 10), sq(11, 11)),
    ];
    let mut board = start.clone();
    let mut state = r1_state(&board);
    let moves: Vec<_> = cycle.into_iter().cycle().take(11).collect();
    record_sequence(&mut board, &mut state, &generator, &moves);
    assert_r1_candidate(
        board.clone(),
        &state,
        &generator,
        cycle[3],
        Some(GameResult::Draw {
            reason: DrawReason::Repetition,
        }),
    );
    // 同じ11手の履歴から、歩兵を取る不可逆手と、未出現の升へ動く候補を試す。
    assert_r1_candidate(
        board.clone(),
        &state,
        &generator,
        step(sq(11, 10), sq(10, 9)),
        None,
    );
    assert_r1_candidate(
        board,
        &state,
        &generator,
        step(sq(11, 10), sq(10, 10)),
        None,
    );

    // 6手周期なら候補込み12手でも初期局面は3回目にとどまる。
    let cycle = [
        step(sq(0, 0), sq(0, 1)),
        step(sq(11, 11), sq(11, 10)),
        step(sq(0, 1), sq(1, 1)),
        step(sq(11, 10), sq(10, 10)),
        step(sq(1, 1), sq(0, 0)),
        step(sq(10, 10), sq(11, 11)),
    ];
    let mut board = start;
    let mut state = r1_state(&board);
    let moves: Vec<_> = cycle.into_iter().cycle().take(11).collect();
    record_sequence(&mut board, &mut state, &generator, &moves);
    assert_r1_candidate(board, &state, &generator, cycle[5], None);

    // じっとの2手周期では4回以上現れていても、候補込み11手なら裁定しない。
    let mut board = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(2, 2), piece(Color::Black, PieceKind::Lion)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
            (sq(9, 9), piece(Color::White, PieceKind::Lion)),
        ],
    );
    let mut state = r1_state(&board);
    let cycle = [step(sq(2, 2), sq(2, 2)), step(sq(9, 9), sq(9, 9))];
    let moves: Vec<_> = cycle.into_iter().cycle().take(10).collect();
    record_sequence(&mut board, &mut state, &generator, &moves);
    assert_r1_candidate(board, &state, &generator, cycle[0], None);
}

fn assert_r1_candidate_attack_calls(
    mut position: Position,
    state: &AdjudicationState,
    generator: &MoveGenerator,
    candidate: Move,
    expected_calls: u32,
    expected_result: Option<GameResult>,
) {
    let RepetitionHistory::R1(history) = state.repetition() else {
        panic!("R1 fixture requires R1 history");
    };
    let before = history.clone();
    let mover = position.side_to_move();
    let undo = position
        .try_make_move_with_undo(candidate, generator)
        .unwrap();
    let calls = Cell::new(0);
    let result = history.candidate_result(
        &position,
        state.ply(),
        mover,
        || {
            calls.set(calls.get() + 1);
            move_was_attacking(&position, generator, mover, candidate)
        },
        move_is_irreversible(candidate, &undo),
    );
    assert_eq!(calls.get(), expected_calls, "candidate={candidate:?}");
    assert_eq!(result, expected_result);
    assert_eq!(*history, before);
}

/// RULES.md第31条R1の前提を満たさない候補は攻撃判定を呼ばず、裁定する候補だけが
/// 1回呼ぶ。局面と着手列は候補の裁定結果を照合する固定例と同じものを用いる。
#[test]
fn article_31_r1_candidate_evaluates_attack_only_when_adjudicating() {
    let generator = MoveGenerator::standard();
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(10, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let cycle = [
        step(sq(0, 0), sq(0, 1)),
        step(sq(11, 11), sq(11, 10)),
        step(sq(0, 1), sq(0, 0)),
        step(sq(11, 10), sq(11, 11)),
    ];
    let mut board = start.clone();
    let mut state = r1_state(&board);
    let moves: Vec<_> = cycle.into_iter().cycle().take(11).collect();
    record_sequence(&mut board, &mut state, &generator, &moves);
    assert_r1_candidate_attack_calls(
        board.clone(),
        &state,
        &generator,
        cycle[3],
        1,
        Some(GameResult::Draw {
            reason: DrawReason::Repetition,
        }),
    );
    // 不可逆な捕獲と未出現の局面への移動は、攻撃判定を必要としない。
    for candidate in [step(sq(11, 10), sq(10, 9)), step(sq(11, 10), sq(10, 10))] {
        assert_r1_candidate_attack_calls(board.clone(), &state, &generator, candidate, 0, None);
    }

    // 候補込み12手でも、6手周期の初期局面は3回目にとどまる。
    let cycle = [
        step(sq(0, 0), sq(0, 1)),
        step(sq(11, 11), sq(11, 10)),
        step(sq(0, 1), sq(1, 1)),
        step(sq(11, 10), sq(10, 10)),
        step(sq(1, 1), sq(0, 0)),
        step(sq(10, 10), sq(11, 11)),
    ];
    let mut board = start;
    let mut state = r1_state(&board);
    let moves: Vec<_> = cycle.into_iter().cycle().take(11).collect();
    record_sequence(&mut board, &mut state, &generator, &moves);
    assert_r1_candidate_attack_calls(board, &state, &generator, cycle[5], 0, None);

    // 4回以上出現していても、候補込み11手では可逆手が不足する。
    let mut board = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(2, 2), piece(Color::Black, PieceKind::Lion)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
            (sq(9, 9), piece(Color::White, PieceKind::Lion)),
        ],
    );
    let mut state = r1_state(&board);
    let cycle = [step(sq(2, 2), sq(2, 2)), step(sq(9, 9), sq(9, 9))];
    let moves: Vec<_> = cycle.into_iter().cycle().take(10).collect();
    record_sequence(&mut board, &mut state, &generator, &moves);
    assert_r1_candidate_attack_calls(board, &state, &generator, cycle[0], 0, None);
}

fn assert_mate(position: &mut Position, context: &AdjudicationContext<'_>, expected: bool) {
    let before = position.clone();
    assert_eq!(oracle_is_mate(position, context), expected);
    assert_eq!(*position, before);
    assert_eq!(production_is_mate(position, context), expected);
    assert_eq!(*position, before);
}

/// RULES.md第21条第3項a・第5項、第15条第9項。
/// 王駒1枚への全逃走先の利きと、先獅子で取れない獅子による王駒2枚取りは詰みとなる。
/// 王将と太子が離れていれば1手では両方を取れず、回避手が残る。
#[test]
fn article_21_mate_handles_one_two_and_separated_royals_and_restores_position() {
    let rules =
        Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2]).unwrap();
    let generator = MoveGenerator::new(rules.moves);
    let mut single = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(0, 11), piece(Color::White, PieceKind::Rook)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
            (sq(10, 9), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&single);
    assert_last_royal(
        &single.clone_with_side_to_move(Color::White),
        &generator,
        true,
    );
    assert_mate(
        &mut single,
        &AdjudicationContext::new(rules, &state, &generator),
        true,
    );

    let mut double = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 0), promoted(Color::Black, PieceKind::CrownPrince)),
            (sq(1, 1), piece(Color::Black, PieceKind::Pawn)),
            (sq(2, 1), piece(Color::Black, PieceKind::Pawn)),
            (sq(2, 0), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 1), piece(Color::White, PieceKind::Lion)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    double.set_lion_capture(Some(sq(0, 5))).unwrap();
    let state = r1_state(&double);
    assert_last_royal(
        &double.clone_with_side_to_move(Color::White),
        &generator,
        true,
    );
    assert_mate(
        &mut double,
        &AdjudicationContext::new(rules, &state, &generator),
        true,
    );

    let mut separated = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(11, 0), promoted(Color::Black, PieceKind::CrownPrince)),
            (sq(0, 2), piece(Color::White, PieceKind::Lion)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&separated);
    assert_mate(
        &mut separated,
        &AdjudicationContext::new(rules, &state, &generator),
        false,
    );
}

/// RULES.md第21条第3項c・d、第31条R1。反復による即時勝利と引き分けは詰みを回避し、
/// 自分の反復負けは回避にならない。既存の固定例と同じ合法な往復で履歴を作る。
#[test]
fn articles_21_31_immediate_win_draw_and_loss_have_distinct_mate_effects() {
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    for (black_pawn, white_pawn, result, is_mated) in [
        (
            true,
            false,
            GameResult::Win {
                winner: Color::Black,
                reason: WinReason::Repetition,
            },
            false,
        ),
        (
            true,
            true,
            GameResult::Draw {
                reason: DrawReason::Repetition,
            },
            false,
        ),
        (
            false,
            true,
            GameResult::Win {
                winner: Color::White,
                reason: WinReason::Repetition,
            },
            true,
        ),
    ] {
        let mut pieces = vec![
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ];
        if black_pawn {
            pieces.push((sq(2, 9), piece(Color::Black, PieceKind::Pawn)));
        }
        if white_pawn {
            pieces.push((sq(7, 9), piece(Color::White, PieceKind::Pawn)));
        }
        let mut board = position(Color::White, &pieces);
        let mut state = r1_state(&board);
        let w_out = step(sq(0, 5), sq(2, 5));
        let w_back = step(sq(2, 5), sq(0, 5));
        let z_out = step(sq(1, 8), sq(1, 2));
        let z_back = step(sq(1, 2), sq(1, 8));
        let y_out = step(sq(3, 9), sq(3, 10));
        record_sequence(
            &mut board,
            &mut state,
            &generator,
            &[
                w_out, z_out, w_back, z_back, w_out, z_out, w_back, z_back, w_out, y_out, w_back,
            ],
        );
        let fresh = r1_state(&board);
        assert_mate(
            &mut board,
            &AdjudicationContext::new(rules, &fresh, &generator),
            true,
        );
        let context = AdjudicationContext::new(rules, &state, &generator);
        assert_mate(&mut board, &context, is_mated);
        let candidate = step(sq(3, 10), sq(3, 9));
        assert_r1_candidate(board.clone(), &state, &generator, candidate, Some(result));
        let before = board.clone();
        let undo = board.make_move_unchecked(candidate, rules.moves);
        assert_eq!(
            oracle_candidate_immediate_result(&context, &board, candidate, &undo),
            Some(result)
        );
        assert_eq!(
            context.candidate_immediate_result(&board, candidate, &undo),
            Some(result)
        );
        assert_last_royal(&board, &generator, true);
        board.unmake_move(undo);
        assert_eq!(board, before);
    }
}

/// RULES.md第21条第3項、第22条第1項。王駒の捕獲を回避しても、
/// その手で駒枯れ負けが確定する場合は詰みから逃れられない。
#[test]
fn articles_21_22_immediate_exhaustion_loss_is_not_a_mate_escape() {
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let mut board = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&board);
    let context = AdjudicationContext::new(rules, &state, &generator);
    assert_mate(&mut board, &context, true);
    let candidate = step(sq(0, 0), sq(1, 0));
    let before = board.clone();
    let undo = board.make_move_unchecked(candidate, rules.moves);
    assert_last_royal(&board, &generator, false);
    let expected = Some(GameResult::Win {
        winner: Color::White,
        reason: WinReason::PieceExhaustion,
    });
    assert_eq!(
        oracle_candidate_immediate_result(&context, &board, candidate, &undo),
        expected
    );
    assert_eq!(
        context.candidate_immediate_result(&board, candidate, &undo),
        expected
    );
    board.unmake_move(undo);
    assert_eq!(board, before);
}

/// RULES.md第23条。王手がなくても合法手がなければ負けとなり、詰みとは区別する。
#[test]
fn article_23_no_legal_move_is_distinct_from_mate() {
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let mut board = position(
        Color::Black,
        &[
            (sq(0, 11), piece(Color::Black, PieceKind::King)),
            (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(10, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&board);
    let before = board.clone();
    assert!(oracle_has_no_legal_move(
        &mut board,
        &generator,
        state.repetition()
    ));
    assert_eq!(board, before);
    assert!(production_has_no_legal_move(
        &mut board,
        &AdjudicationContext::new(rules, &state, &generator)
    ));
    assert_eq!(board, before);
    assert_mate(
        &mut board,
        &AdjudicationContext::new(rules, &state, &generator),
        false,
    );
}

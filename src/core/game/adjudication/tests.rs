//! 終局裁定の回避条件と反復による合法手の絞り込みの試験。

use std::collections::HashSet;

use super::super::repetition::retain_repetition_allowed_moves;
use super::mate::can_capture_last_royal;
use super::*;
use crate::core::game::{Game, GameStatus};
use crate::core::piece::{PieceCode, PieceKind};
use crate::core::rules::RuleCode;
use crate::test_util::{position_from_codes as position, sq};

fn piece(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new(color, kind).expect("fixture uses an unpromoted-capable kind")
}

fn step(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn r1_state(position: &Position) -> AdjudicationState {
    AdjudicationState::new(RepetitionRule::R1, position)
}

#[test]
fn article_31_r1_candidate_at_eleven_reversible_plies_does_not_rescue_mate() {
    // D3-031-13: 4回目の同一局面を生じさせる受けでも、仮想着手を含めて
    // 可逆手が11手なら即時勝利にならず詰みとなる
    // (lishogi-bot.md「反復裁定の整合」、第21条第3項c・第31条R1)。
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 8), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::Black, PieceKind::Rook)),
            (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
            (sq(10, 10), piece(Color::Black, PieceKind::Lion)),
            (sq(0, 5), piece(Color::White, PieceKind::Rook)),
            (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(7, 10), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 3), piece(Color::White, PieceKind::Lion)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules = Rules::ENGINE_DEFAULT;
    let generator = MoveGenerator::new(rules.moves);
    let mut position = start;
    let mut state = r1_state(&position);
    let plies = [
        step(sq(10, 10), sq(10, 9)),
        step(sq(8, 3), sq(8, 3)),
        step(sq(10, 9), sq(10, 9)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(1, 8), sq(1, 2)),
        step(sq(2, 5), sq(0, 5)),
        step(sq(1, 2), sq(1, 8)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(3, 9), sq(3, 10)),
        step(sq(2, 5), sq(0, 5)),
    ];
    // 1・3・7手目の局面へ11手目の候補(3,10)→(3,9)で戻る。
    // 局面合法な着手を審判履歴へ直接記録し、仮想着手の裁定を単独で検証する。
    for mv in plies {
        let mover = position.side_to_move();
        let undo = position.try_make_move_with_undo(mv, &generator).unwrap();
        assert_eq!(
            state.record_move(&position, &generator, mover, mv, &undo),
            None
        );
    }
    let before = position.clone();
    let candidate = step(sq(3, 10), sq(3, 9));
    let undo = position
        .try_make_move_with_undo(candidate, &generator)
        .unwrap();
    let context = AdjudicationContext::new(rules, &state, &generator);
    assert!(!context.candidate_is_immediate_win(&position, candidate, &undo));
    position.unmake_move(undo);
    assert!(is_mate(&mut position, &context));
    assert_eq!(position, before);
}

#[test]
fn article_21_3_mate_requires_every_escape_clause_to_fail() {
    // D3-021-02: 3項a(回避)・b(相手王駒の先取り)のいずれかが残れば詰みは
    // 成立しない。仮想着手の評価後に局面は完全に復元される
    // (adjudication-refactor.md「検証」)。
    let generator = MoveGenerator::standard();

    // 受けが尽きた局面は詰みである(第21条2項)。
    let mut mated = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(0, 11), piece(Color::White, PieceKind::Rook)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
            (sq(10, 9), piece(Color::White, PieceKind::King)),
        ],
    );
    let mated_before = mated.clone();
    let state = r1_state(&mated);
    assert!(is_mate(
        &mut mated,
        &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
    ));
    assert_eq!(mated, mated_before);

    // 3項a: (0,1)への逃げが残れば詰みではない。
    let mut escapable = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
            (sq(10, 9), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&escapable);
    assert!(!is_mate(
        &mut escapable,
        &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
    ));

    // 3項b: 相手の最後の王駒を先に取れる着手が残れば詰みではない(第21条4項)。
    let mut counter_capture = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&counter_capture);
    assert!(can_capture_last_royal(&counter_capture, &generator));
    assert!(!is_mate(
        &mut counter_capture,
        &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
    ));

    // 3項b境界: 先取りした王駒の升へ相手の取り返しの利き(飛車)が残っていても、
    // 最後の王駒を取った時点で勝ちが確定する(第21条4項)ため詰みではない。
    // 変異検証(フェーズ4)で検出したオラクル欠落の補強。
    let mut counter_capture_with_retaliation = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(1, 1), piece(Color::White, PieceKind::King)),
            (sq(1, 11), piece(Color::White, PieceKind::Rook)),
        ],
    );
    let state = r1_state(&counter_capture_with_retaliation);
    assert!(!is_mate(
        &mut counter_capture_with_retaliation,
        &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
    ));

    // 第23条境界: 着手が1つもない局面は合法手なしであり、詰みではない。
    let mut stuck = position(
        Color::Black,
        &[
            (sq(4, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(0, 11), piece(Color::Black, PieceKind::Lance)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
        ],
    );
    let state = r1_state(&stuck);
    assert!(has_no_legal_move(
        &mut stuck,
        &generator,
        state.repetition()
    ));
    assert!(!is_mate(
        &mut stuck,
        &AdjudicationContext::new(Rules::ENGINE_DEFAULT, &state, &generator)
    ));
}

#[test]
fn plan_adjudication_r2_filter_matches_game_legal_moves_and_restores_the_position() {
    // D3-PRP-03: 対局合法手の絞り込み(R2禁止フィルタ)は、共有関数の経路と
    // Game::legal_movesの経路で一致し、仮想評価の前後で局面が復元される。
    // D3-031-06: 既出局面を再現する着手だけが局面合法手から除かれる。
    let start = position(
        Color::Black,
        &[
            (sq(3, 3), piece(Color::Black, PieceKind::King)),
            (sq(8, 8), piece(Color::White, PieceKind::King)),
        ],
    );
    let rules =
        Rules::from_codes(&[RuleCode::L0, RuleCode::P0, RuleCode::R2, RuleCode::E2]).unwrap();
    let generator = MoveGenerator::new(rules.moves);
    let cycle = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
    ];

    let mut game = Game::from_position(rules, start.clone());
    let mut low_level = start.clone();
    let mut state = AdjudicationState::new(RepetitionRule::R2, &low_level);
    for mv in cycle {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        let mover = low_level.side_to_move();
        let undo = low_level.try_make_move_with_undo(mv, &generator).unwrap();
        state.record_move(&low_level, &generator, mover, mv, &undo);
    }

    let mut moves = Vec::new();
    generator.generate_moves(&low_level, &mut moves);
    let repeating = step(sq(8, 7), sq(8, 8));
    assert!(moves.contains(&repeating));

    let snapshot = low_level.clone();
    retain_repetition_allowed_moves(&mut low_level, &generator, state.repetition(), &mut moves);
    assert_eq!(low_level, snapshot);
    assert!(!moves.contains(&repeating));
    assert_eq!(
        moves.into_iter().collect::<HashSet<_>>(),
        game.legal_moves().into_iter().collect::<HashSet<_>>()
    );
}

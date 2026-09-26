//! 第26〜27条の不合法な着手とその効果の試験。

use std::collections::HashSet;

use super::*;

// 王将2枚だけの非攻撃的4手周期(R2・R3の拒否フィクスチャ。E2併用で駒枯れを外す)。
fn king_cycle_position() -> Position {
    position(
        Color::Black,
        &[
            (sq(3, 3), piece(Color::Black, PieceKind::King)),
            (sq(8, 8), piece(Color::White, PieceKind::King)),
        ],
    )
}

fn king_cycle() -> [Move; 4] {
    [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ]
}

#[test]
fn article_26_1_moving_the_opponents_piece_is_rejected() {
    // D3-026-01: 手番側が所有しない駒を動かす入力は着手として成立しない。
    let mut game = Game::with_default_rules();
    let white_pawn_move = step(sq(0, 8), sq(0, 7));

    assert!(matches!(
        game.play(white_pawn_move),
        Err(GameError::IllegalMove {
            cause: IllegalMoveCause::Movement,
            ..
        })
    ));
    assert_eq!(game.position(), &Position::initial());
    assert_eq!(game.ply_count(), 0);

    // 自分の駒を動かす合法手はそのまま受理される。
    let legal = game.legal_moves()[0];
    assert_eq!(game.play(legal), Ok(GameStatus::Ongoing));
}

#[test]
fn article_26_12_no_moves_are_accepted_after_the_game_ends() {
    // D3-026-02: 終局済みの対局への着手入力は理由によらず拒否され、
    // 記録済みの結果は変化しない。終局後の対局合法手は空である。
    let mut agreed = Game::with_default_rules();
    let legal = agreed.legal_moves()[0];
    agreed.agree_draw().unwrap();
    assert_eq!(agreed.play(legal), Err(GameError::GameAlreadyOver));
    assert_eq!(
        agreed.result(),
        Some(GameResult::Draw {
            reason: DrawReason::Agreement,
        })
    );
    assert!(agreed.legal_moves().is_empty());
    assert_eq!(agreed.ply_count(), 0);

    let mut captured = game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Rook)),
            (sq(5, 8), piece(Color::White, PieceKind::King)),
        ],
    ));
    let result = GameResult::Win {
        winner: Color::Black,
        reason: WinReason::RoyalCapture,
    };
    assert_eq!(
        captured.play(step(sq(5, 5), sq(5, 8))),
        Ok(GameStatus::Finished(result))
    );
    assert_eq!(
        captured.play(step(sq(0, 0), sq(0, 1))),
        Err(GameError::GameAlreadyOver)
    );
    assert_eq!(captured.result(), Some(result));
    assert!(captured.legal_moves().is_empty());
}

fn assert_rejection_is_pure(game: &mut Game, rejected: Move, cause: IllegalMoveCause) {
    let position_before = game.position().clone();
    let ply_before = game.ply_count();
    let keys_before = game.search_key_history().to_vec();
    let legal_before: HashSet<Move> = game.legal_moves().into_iter().collect();

    assert_eq!(
        game.play(rejected),
        Err(GameError::IllegalMove {
            mv: rejected,
            cause
        })
    );

    assert_eq!(game.position(), &position_before);
    assert_eq!(game.ply_count(), ply_before);
    assert_eq!(game.search_key_history(), keys_before.as_slice());
    assert_eq!(
        game.legal_moves().into_iter().collect::<HashSet<_>>(),
        legal_before
    );
    assert_eq!(game.status(), GameStatus::Ongoing);
}

#[test]
fn article_27_4_r2_r3_reject_before_acceptance_while_r1_adjudicates_after() {
    // D3-026-03/D3-027-02: 同じ着手列でも、R1は⑫を受理してから裁定し、
    // R2・R3は禁止対象の着手を受理前に不合法として拒否する。拒否後は
    // 別の着手で対局を継続できる。
    // D3-031-06: R2は2回目の再現から直ちに禁止する。
    // D3-031-10: R3は2回目・3回目の再現を合法とし4回目の出現だけを禁止する。
    let cycle = king_cycle();
    let alternative = step(sq(8, 7), sq(9, 7));

    let mut r1 = game_with_codes(king_cycle_position(), &[RuleCode::R1, RuleCode::E2]);
    for ply in 1..=11 {
        assert_eq!(
            r1.play(cycle[(ply - 1) % cycle.len()]),
            Ok(GameStatus::Ongoing),
            "R1 ply {ply}"
        );
    }
    assert_eq!(r1.play(cycle[3]), draw(DrawReason::Repetition));

    let mut r2 = game_with_codes(king_cycle_position(), &[RuleCode::R2, RuleCode::E2]);
    for mv in cycle.into_iter().take(3) {
        assert_eq!(r2.play(mv), Ok(GameStatus::Ongoing));
    }
    assert_rejection_is_pure(&mut r2, cycle[3], IllegalMoveCause::Repetition);
    assert_eq!(r2.play(alternative), Ok(GameStatus::Ongoing));

    let mut r3 = game_with_codes(king_cycle_position(), &[RuleCode::R3, RuleCode::E2]);
    for ply in 1..=11 {
        assert_eq!(
            r3.play(cycle[(ply - 1) % cycle.len()]),
            Ok(GameStatus::Ongoing),
            "R3 ply {ply}"
        );
    }
    assert_rejection_is_pure(&mut r3, cycle[3], IllegalMoveCause::Repetition);
    assert_eq!(r3.play(alternative), Ok(GameStatus::Ongoing));
}

#[test]
fn article_27_1_rejected_moves_leave_the_game_state_unchanged() {
    // D3-027-01: 不合法な着手の拒否の前後で、局面・手番・手数・探索局面
    // キー履歴・対局合法手集合・対局状態がすべて不変である。

    // 駒の動きに反する入力(第26条2号)。
    let mut movement = Game::with_default_rules();
    assert_rejection_is_pure(
        &mut movement,
        step(sq(5, 5), sq(5, 6)),
        IllegalMoveCause::Movement,
    );
    assert_eq!(
        movement.play(movement.legal_moves()[0]),
        Ok(GameStatus::Ongoing)
    );
}

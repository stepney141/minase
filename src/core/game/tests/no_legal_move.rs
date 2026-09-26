//! 第23条の合法手がない場合の試験。

use super::*;

#[test]
fn article_23_1_no_move_at_all_loses_even_without_check() {
    // D3-023-01: 手番側に着手が1つも存在しなければ手番側の負けとなる。
    // F3では王手がかかっていない構成でも成立する(詰みとは独立の敗北条件)。
    let mut game = game(f3_no_move_position());

    assert_eq!(
        game.play(step(sq(10, 0), sq(10, 1))),
        win(Color::White, WinReason::Stalemate)
    );
}

#[test]
fn article_23_2_unsafe_moves_still_count_as_legal_moves() {
    // D3-023-02: 王駒を相手の利きへ移す着手や王手を解消しない着手も
    // 合法手であり、これらが存在すれば第23条は適用されない。
    // 変形a: 香車の前方の駒を後手駒に差し替えると捕獲の着手が残る。
    let mut capture_variant = game(position(
        Color::White,
        &[
            (sq(0, 11), piece(Color::Black, PieceKind::King)),
            (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
            (sq(0, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(10, 0), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        capture_variant.play(step(sq(10, 0), sq(10, 1))),
        Ok(GameStatus::Ongoing)
    );

    // 変形b: 王将を後手の利き(角の照準)へ移す着手も受理される(第8条3項)。
    let mut suicidal_variant = game(position(
        Color::Black,
        &[
            (sq(0, 11), piece(Color::Black, PieceKind::King)),
            (sq(1, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(1, 10), piece(Color::Black, PieceKind::Lance)),
            (sq(5, 5), piece(Color::White, PieceKind::Bishop)),
            (sq(10, 0), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        suicidal_variant.play(step(sq(0, 11), sq(0, 10))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_23_3_r2_filtered_out_moves_cause_a_stalemate_loss() {
    // D3-023-03: 局面合法手がすべてR2で禁止されると対局合法手が空になり、
    // 第23条1項により手番側の負けとなる。E1併用でも同じ(D3-032-02)。
    let mut pieces = vec![
        (sq(0, 0), piece(Color::Black, PieceKind::GoBetween)),
        (sq(11, 11), piece(Color::Black, PieceKind::King)),
        (sq(10, 10), piece(Color::Black, PieceKind::Pawn)),
        (sq(10, 11), piece(Color::Black, PieceKind::Pawn)),
        (sq(11, 10), piece(Color::Black, PieceKind::Pawn)),
        (sq(5, 5), piece(Color::White, PieceKind::King)),
    ];
    pieces.extend((2..=11).map(|rank| (sq(0, rank), piece(Color::Black, PieceKind::Pawn))));

    for codes in [
        &[RuleCode::R2, RuleCode::E2][..],
        &[RuleCode::R2, RuleCode::E1, RuleCode::E2][..],
    ] {
        let mut game = game_with_codes(position(Color::White, &pieces), codes);
        assert_eq!(game.play(step(sq(5, 5), sq(5, 4))), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
        assert_eq!(
            game.play(step(sq(5, 4), sq(5, 5))),
            win(Color::White, WinReason::Stalemate),
            "codes={codes:?}"
        );
    }
}

//! 第20条の王駒の試験。

use super::*;

#[test]
fn article_20_1_game_starts_ongoing_with_one_royal_each() {
    // D3-020-01: 開始時は各対局者の王駒が王将・玉将の1枚だけで、終局していない。
    let game = Game::new(Rules::ENGINE_DEFAULT);

    assert_eq!(game.status(), GameStatus::Ongoing);
    assert_eq!(game.result(), None);
    for color in Color::ALL {
        assert_eq!(game.position().royal_pieces(color).popcount(), 1);
    }
}

#[test]
fn article_20_2_promoted_elephant_becomes_a_royal_prince() {
    // D3-020-02/D3-020-04: 成った醉象は太子=王駒であり、王将を取られても
    // 太子が残る限り継続し、2枚目の王駒の捕獲で初めて終局する。
    let mut game = game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(4, 7), piece(Color::Black, PieceKind::DrunkElephant)),
            (sq(9, 2), piece(Color::Black, PieceKind::Pawn)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(4, 11), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));

    // 醉象が敵陣(段8)へ入り太子へ成る(第18条1項、第20条2項)。
    assert_eq!(
        game.play(promoting(sq(4, 7), sq(4, 8))),
        Ok(GameStatus::Ongoing)
    );
    // 王将を取られても太子が残るため終局せず、勝敗理由も記録されない。
    assert_eq!(
        game.play(step(sq(11, 0), sq(0, 0))),
        Ok(GameStatus::Ongoing)
    );
    assert_eq!(game.result(), None);
    assert!(!game.legal_moves().is_empty());
    assert_eq!(game.play(step(sq(9, 2), sq(9, 3))), Ok(GameStatus::Ongoing));
    // 最後の王駒である太子の捕獲で第21条1項の勝ちになる。
    assert_eq!(
        game.play(step(sq(4, 11), sq(4, 8))),
        win(Color::White, WinReason::RoyalCapture)
    );

    // 境界: 成る前の醉象は王駒ではなく、王将の捕獲だけで直ちに終局する。
    let mut unpromoted = game_with_codes(
        position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 7), piece(Color::Black, PieceKind::DrunkElephant)),
                (sq(11, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );
    assert_eq!(
        unpromoted.play(step(sq(11, 0), sq(0, 0))),
        win(Color::White, WinReason::RoyalCapture)
    );
}

#[test]
fn article_20_3_capturing_the_prince_alone_continues_the_game() {
    // D3-020-03: 王将が残る側は太子を取られても敗北しない。太子喪失直後に
    // 王手がかかっていても、対局は継続し合法手も通常どおり生成される。
    let mut game = game(position(
        Color::White,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), prince(Color::Black)),
            (sq(5, 11), piece(Color::White, PieceKind::Rook)),
            (sq(11, 0), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));

    // (11,0)の飛車が黒王将へ王手をかけたまま、太子だけを取る。
    assert_eq!(
        game.play(step(sq(5, 11), sq(5, 5))),
        Ok(GameStatus::Ongoing)
    );
    assert_eq!(game.result(), None);
    assert!(!game.legal_moves().is_empty());
}

#[test]
fn plan_referee_from_position_defers_adjudication_to_the_first_move() {
    // D3-020-05: 任意局面からの構築では自動裁定せず、開始局面を履歴の
    // 第1回として記録する(adjudication-refactor.md「任意局面と履歴」)。
    // 直後の着手完了時からは通常の裁定が働く。
    let missing_royal = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::GoldGeneral)),
            (sq(9, 9), piece(Color::White, PieceKind::Pawn)),
        ],
    );
    let mut no_royal_game = game(missing_royal);
    assert_eq!(no_royal_game.status(), GameStatus::Ongoing);
    assert_eq!(
        no_royal_game.play(step(sq(5, 5), sq(5, 6))),
        win(Color::Black, WinReason::RoyalCapture)
    );

    // 駒枯れ条件(第22条1項)を満たす開始局面も構築時には裁定されない。
    let mut exhausted = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );
    assert_eq!(exhausted.status(), GameStatus::Ongoing);
    assert_eq!(
        exhausted.play(step(sq(0, 0), sq(0, 1))),
        win(Color::White, WinReason::PieceExhaustion)
    );
}

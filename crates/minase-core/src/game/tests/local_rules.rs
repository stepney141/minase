//! 第32条の終局に関するローカルルールの試験。

use super::*;

fn bare_king_game(position: Position) -> Game {
    game_with_codes(
        position,
        &[
            RuleCode::L0,
            RuleCode::P0,
            RuleCode::R1,
            RuleCode::E1,
            RuleCode::E3,
        ],
    )
}

#[test]
fn article_32_e1_defers_the_win_to_actual_royal_capture() {
    // E1採用時は詰み局面でも終局せず、王手放置を含む合法手を
    // 指し続けられる。実際に最後の王駒が取られた着手の完了時に第21条1項で
    // 終局する。
    let (position, mv) = mate_predecessor();
    let mut game = game_with_codes(position, &[RuleCode::R1, RuleCode::E1, RuleCode::E2]);

    assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    assert_eq!(game.result(), None);
    assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.play(step(sq(0, 11), sq(0, 1))),
        win(Color::White, WinReason::RoyalCapture)
    );
}

#[test]
fn article_32_e1_leaves_other_adjudications_active() {
    // E1が無効化するのは詰み裁定だけであり、駒枯れと合法手なし
    // の裁定はE1採用下でも通常どおり働く。
    let mut exhaustion = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1, RuleCode::E1],
    );
    assert_eq!(
        exhaustion.play(step(sq(4, 4), sq(4, 5))),
        win(Color::Black, WinReason::PieceExhaustion)
    );

    let mut stalemate = game_with_codes(f3_no_move_position(), &[RuleCode::R1, RuleCode::E1]);
    assert_eq!(
        stalemate.play(step(sq(10, 0), sq(10, 1))),
        win(Color::White, WinReason::Stalemate)
    );
}

#[test]
fn article_32_e2_disables_all_piece_exhaustion_adjudication() {
    // E2採用時は第22条の裁定を一切行わず対局を継続する。
    let e2_game = |position| game_with_codes(position, &[RuleCode::R1, RuleCode::E2]);

    // 即時勝ちになるはずの局面(第22条1項)でも裁定しない。
    let mut immediate = e2_game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        immediate.play(step(sq(4, 4), sq(4, 5))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_32_e3_enough_effective_pieces_adjudicate_the_bare_king_loss() {
    // (a)王駒1枚以上を含む価値ある駒2枚以上、(b)裸玉側が相手のいずれの王駒も
    // 取れない、(c)3枚以上または全て非隣接、がすべて成立すると裸玉側の負け。
    // 価値ある駒が3枚以上あれば隣接の有無は問わない。
    let mut adjacent_three = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(6, 5), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(9, 0), piece(Color::White, PieceKind::Bishop)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        adjacent_three.play(step(sq(5, 4), sq(5, 5))),
        win(Color::White, WinReason::BareKing)
    );

    // 2枚でも裸玉に隣接していなければ(c)が成立して負けになる。
    let mut distant_two = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(9, 9), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        distant_two.play(step(sq(5, 4), sq(5, 5))),
        win(Color::White, WinReason::BareKing)
    );

    // 境界(a): 歩兵は価値ある駒でないため(第3条11項)、王将+歩兵では
    // 2枚に届かず裁定しない。
    let mut king_and_pawn = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(8, 8), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        king_and_pawn.play(step(sq(5, 4), sq(5, 5))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_32_e3_two_pieces_with_adjacency_defer_the_loss() {
    // 価値ある駒が2枚でその一方が裸玉に隣接していれば(c)が
    // 不成立となり裁定を保留する。以後の着手完了時に隣接が解消されれば
    // 改めて成立する。
    let mut game = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(6, 5), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));

    assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    // 白金が離れて隣接が解消された着手の完了時に、裸玉側の負けが確定する。
    assert_eq!(
        game.play(step(sq(6, 5), sq(7, 5))),
        win(Color::White, WinReason::BareKing)
    );
}

#[test]
fn article_32_e3_bare_side_threatening_a_royal_defers_the_loss() {
    // 裸玉側が相手王駒を取れる間は(b)が不成立となり裁定を保留する。
    // 捕獲できなくなった後の着手完了時に改めて判定する。
    let mut game = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(0, 10), piece(Color::White, PieceKind::Rook)),
            (sq(6, 6), piece(Color::White, PieceKind::King)),
            (sq(10, 0), piece(Color::White, PieceKind::Rook)),
        ],
    ));

    // 黒王将が白玉将に隣接して捕獲できるため保留される。
    assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    // 白玉将が捕獲を回避した着手の完了時に裁定が成立する。
    assert_eq!(
        game.play(step(sq(6, 6), sq(7, 7))),
        win(Color::White, WinReason::BareKing)
    );
}

#[test]
fn article_32_e3_threatening_either_of_two_royals_defers_the_loss() {
    // E3の条件(b)は王将と太子のいずれか1枚を取れるだけでも不成立となる。
    // 相手の価値ある駒を3枚にして、隣接による条件(c)の保留と区別する。
    for bare_color in Color::ALL {
        let opponent = bare_color.opposite();
        let king = piece(opponent, PieceKind::King);
        let prince = prince(opponent);
        for (threatened, remaining) in [(king, prince), (prince, king)] {
            let mut game = bare_king_game(position(
                bare_color,
                &[
                    (sq(5, 4), piece(bare_color, PieceKind::King)),
                    (sq(6, 6), threatened),
                    (sq(11, 11), remaining),
                    (sq(0, 10), piece(opponent, PieceKind::Rook)),
                ],
            ));
            assert_eq!(
                game.play(step(sq(5, 4), sq(5, 5))),
                Ok(GameStatus::Ongoing),
                "bare side: {bare_color:?}, threatened: {threatened:?}"
            );
            assert_eq!(
                game.play(step(sq(6, 6), sq(7, 7))),
                win(opponent, WinReason::BareKing)
            );
        }
    }
}

#[test]
fn article_32_e3_immobile_pawns_and_lances_are_excluded_from_counts() {
    // 最奥段で移動不能となった歩兵・香車は、裸玉側の判定からも
    // 相手側の価値ある駒の数えからも除外する。
    let mut bare_side_dead_pieces = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(4, 11), piece(Color::Black, PieceKind::Lance)),
            (sq(9, 9), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        bare_side_dead_pieces.play(step(sq(5, 4), sq(5, 5))),
        win(Color::White, WinReason::BareKing)
    );

    // 相手側の死に香車を数えると3枚で(c)成立になるが、除外により2枚+隣接
    // として保留される。
    let mut opponent_dead_pieces = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(3, 0), piece(Color::White, PieceKind::Pawn)),
            (sq(4, 0), piece(Color::White, PieceKind::Lance)),
            (sq(6, 5), piece(Color::White, PieceKind::Rook)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        opponent_dead_pieces.play(step(sq(5, 4), sq(5, 5))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_32_e3_double_bare_kings_draw_unless_a_royal_can_be_captured() {
    // 双方が裸玉(死に駒を除く)で、いずれの王駒も相手の次の着手で取られなければ引き分け。
    // 取られ得る王駒がある間は裁定せず継続する。
    let mut safe = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(3, 0), piece(Color::White, PieceKind::Lance)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        safe.play(step(sq(5, 4), sq(5, 5))),
        draw(DrawReason::BareKing)
    );

    let mut threatened = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(6, 6), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        threatened.play(step(sq(5, 4), sq(5, 5))),
        Ok(GameStatus::Ongoing)
    );
}

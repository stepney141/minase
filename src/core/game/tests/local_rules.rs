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
    // D3-032-01: E1採用時は詰み局面でも終局せず、王手放置を含む合法手を
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
    // D3-032-02: E1が無効化するのは詰み裁定だけであり、駒枯れと合法手なし
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
    // D3-032-03: E2採用時は第22条の裁定を一切行わず対局を継続する。
    let e2_game = |position| game_with_codes(position, &[RuleCode::R1, RuleCode::E2]);

    // 双方王駒のみ(8項相当)でも自動引き分けにしない。
    let mut only_royals = e2_game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        only_royals.play(step(sq(0, 0), sq(0, 1))),
        Ok(GameStatus::Ongoing)
    );

    // 猶予が始まるはずの局面(第22条5項)でも裁定しない。
    let (grace_position, establishes_condition) = grace_predecessor();
    let mut condition = e2_game(grace_position);
    assert_eq!(
        condition.play(establishes_condition),
        Ok(GameStatus::Ongoing)
    );
    assert_eq!(
        condition.play(step(sq(4, 7), sq(3, 7))),
        Ok(GameStatus::Ongoing)
    );

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
    // D3-032-04: (a)王駒1枚以上を含む価値ある駒2枚以上、(b)裸玉側の王手
    // なし、(c)3枚以上または全て非隣接、がすべて成立すると裸玉側の負け。
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

    // 2枚でも裸玉に隣接していなければ(c)が成立して負けになる(D3-032-05境界)。
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
    // D3-032-05: 価値ある駒が2枚でその一方が裸玉に隣接していれば(c)が
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
fn article_32_e3_bare_side_giving_check_defers_the_loss() {
    // D3-032-06: 裸玉側が相手王駒へ王手をかけている間は(b)が不成立となり
    // 裁定を保留する。王手が解消された後の着手完了時に改めて判定する。
    let mut game = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(0, 10), piece(Color::White, PieceKind::Rook)),
            (sq(6, 6), piece(Color::White, PieceKind::King)),
            (sq(10, 0), piece(Color::White, PieceKind::Rook)),
        ],
    ));

    // 黒王将が白玉将に隣接して王手をかけるため保留される。
    assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    // 白玉将が王手を外れた着手の完了時に裁定が成立する。
    assert_eq!(
        game.play(step(sq(6, 6), sq(7, 7))),
        win(Color::White, WinReason::BareKing)
    );
}

#[test]
fn article_32_e3_immobile_pawns_and_lances_are_excluded_from_counts() {
    // D3-032-07: 最奥段で移動不能となった歩兵・香車は、裸玉側の判定からも
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
fn article_32_e3_double_bare_kings_draw_unless_checked() {
    // D3-032-08: 双方が裸玉(死に駒を除く)で、いずれの王駒にも王手が
    // かかっていなければ引き分け。王手がかかっている間は裁定せず継続する。
    let mut unchecked = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(3, 11), piece(Color::Black, PieceKind::Pawn)),
            (sq(3, 0), piece(Color::White, PieceKind::Lance)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        unchecked.play(step(sq(5, 4), sq(5, 5))),
        draw(DrawReason::BareKing)
    );

    let mut checked = bare_king_game(position(
        Color::Black,
        &[
            (sq(5, 4), piece(Color::Black, PieceKind::King)),
            (sq(6, 6), piece(Color::White, PieceKind::King)),
        ],
    ));
    assert_eq!(
        checked.play(step(sq(5, 4), sq(5, 5))),
        Ok(GameStatus::Ongoing)
    );
}

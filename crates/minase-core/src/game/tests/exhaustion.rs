//! 第22条の駒枯れの試験。

use super::*;

#[test]
fn article_22_1_bare_side_without_a_capture_loses_immediately() {
    // D3-022-01: 条件成立時に裸側の手番でも、余分な駒を取れなければ
    // 余分な駒を持つ側の勝ちとなる。
    let mut game = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 4), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );

    // 黒金の着手完了で条件が成立し、白(裸側)は金を取れないため黒の勝ち。
    assert_eq!(
        game.play(step(sq(4, 4), sq(4, 5))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn plan_referee_5_extra_side_to_move_wins_without_grace() {
    // D3-022-09: 条件成立時に余分な駒を持つ側が手番なら、5項の猶予を
    // 適用せず第22条1項で直ちに裁定する(game-referee.md 5節「猶予の適用条件」)。
    let mut game = game_with_codes(
        position(
            Color::White,
            &[
                (sq(4, 4), piece(Color::Black, PieceKind::King)),
                (sq(8, 8), piece(Color::Black, PieceKind::Pawn)),
                (sq(4, 5), piece(Color::White, PieceKind::Pawn)),
                (sq(8, 11), piece(Color::White, PieceKind::Rook)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );

    // 白飛が黒歩を取っても非王駒は2枚残り、条件は未成立。
    assert_eq!(
        game.play(step(sq(8, 11), sq(8, 8))),
        Ok(GameStatus::Ongoing)
    );
    // 黒王将が白歩を取った時点で条件が成立し、手番は白(余分側)。猶予なし。
    assert_eq!(
        game.play(step(sq(4, 4), sq(4, 5))),
        win(Color::White, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_2_pawn_extra_piece_waits_until_promotion() {
    // D3-022-02: 余分な駒が歩兵なら金将へ成るまで裁定せず対局を継続する。
    // D3-022-10: 非捕獲の成りの時点では猶予を与えず直ちに勝ちとする
    // (game-referee.md 5節「成りの時点の裁定」)。白王将が(3,10)から成歩を
    // 取れる配置でも、成った時点で黒の勝ちが確定する。
    let mut game = game_with_codes(
        position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 9), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );

    assert_eq!(
        game.play(step(sq(3, 9), sq(3, 10))),
        Ok(GameStatus::Ongoing)
    );
    assert_eq!(
        game.play(promoting(sq(4, 10), sq(4, 11))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_2_repetition_still_adjudicates_during_the_wait() {
    // D3-022-02境界: 歩兵の成り待機中も反復など他の裁定は通常どおり働く。
    let mut game = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 10), piece(Color::Black, PieceKind::Pawn)),
                (sq(3, 7), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );
    let cycle = [
        step(sq(0, 0), sq(0, 1)),
        step(sq(3, 7), sq(3, 8)),
        step(sq(0, 1), sq(0, 0)),
        step(sq(3, 8), sq(3, 7)),
    ];

    for ply in 1..=11 {
        assert_eq!(
            game.play(cycle[(ply - 1) % cycle.len()]),
            Ok(GameStatus::Ongoing),
            "ply {ply}"
        );
    }
    assert_eq!(game.play(cycle[3]), draw(DrawReason::Repetition));
}

#[test]
fn article_22_3_go_between_extra_piece_waits_until_promotion() {
    // D3-022-03: 余分な駒が仲人なら醉象へ成った時点で勝ちとなる。
    let mut game = game_with_codes(
        position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(4, 7), piece(Color::Black, PieceKind::GoBetween)),
                (sq(3, 7), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );

    assert_eq!(game.play(step(sq(3, 7), sq(3, 8))), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.play(promoting(sq(4, 7), sq(4, 8))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_4_immobile_pawn_and_lance_are_not_winning_extras() {
    // D3-022-04: 最奥段で移動不能となった歩兵・香車は勝利を成立させる
    // 余分な駒として数えず、裁定を行わず対局を継続する。
    for kind in [PieceKind::Pawn, PieceKind::Lance] {
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 11), piece(Color::Black, kind)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        assert_eq!(
            game.play(step(sq(11, 11), sq(10, 11))),
            Ok(GameStatus::Ongoing),
            "{kind:?}"
        );
    }
}

#[test]
fn articles_22_5_and_22_6_grace_capture_leads_to_a_draw() {
    // D3-022-05/D3-022-06: 裸側が余分な駒を次の1手で取れる場合は裁定を
    // 保留し、取れば駒枯れ不成立。双方王駒のみとなり8項の引き分けになる。
    let (position, establishes_condition) = grace_predecessor();
    let mut game = game_with_codes(position, &[RuleCode::R1]);

    assert_eq!(game.play(establishes_condition), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.play(step(sq(4, 7), sq(5, 6))),
        draw(DrawReason::PieceExhaustion)
    );
}

#[test]
fn article_22_7_declining_the_grace_loses_immediately() {
    // D3-022-07: 猶予の1手で余分な駒を取らなければ、その着手の完了時に
    // 余分な駒を持つ側の勝ちとなる。保留は再付与されない。
    let (position, establishes_condition) = grace_predecessor();
    let mut game = game_with_codes(position, &[RuleCode::R1]);

    assert_eq!(game.play(establishes_condition), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.play(step(sq(4, 7), sq(3, 7))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_8_bare_royals_draw_but_prince_pairs_continue() {
    // D3-022-08: 双方に王駒1枚ずつだけが残れば自動的に引き分けとする。
    let mut kings = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );
    assert_eq!(
        kings.play(step(sq(0, 0), sq(0, 1))),
        draw(DrawReason::PieceExhaustion)
    );

    // 境界: 王将と太子が併存する側がある間は第22条の条件自体が成立しない
    // (game-referee.md 5節)。
    let mut with_prince = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(2, 0), prince(Color::Black)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );
    assert_eq!(
        with_prince.play(step(sq(0, 0), sq(0, 1))),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn plan_referee_5_capturing_promotion_that_creates_the_condition_takes_the_grace_path() {
    // D3-022-11: 捕獲を伴う成りで初めて条件が成立した場合は待機局面が
    // 存在しないため、2項ではなく1項・5項の通常の流れで猶予を判定する
    // (game-referee.md 5節但し書き、9節のS1レビュー境界)。
    let capturing_promotion = promoting(sq(5, 9), sq(5, 10));
    let condition_arises = |first_reply: Move| {
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(5, 9), piece(Color::Black, PieceKind::Pawn)),
                    (sq(5, 10), piece(Color::White, PieceKind::GoldGeneral)),
                    (sq(4, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(game.play(capturing_promotion), Ok(GameStatus::Ongoing));
        game.play(first_reply)
    };

    // 猶予の1手で成歩を取れば駒枯れ不成立、8項により引き分け。
    assert_eq!(
        condition_arises(step(sq(4, 11), sq(5, 10))),
        draw(DrawReason::PieceExhaustion)
    );
    // 取らなければ7項により余分な駒を持つ側の勝ち。
    assert_eq!(
        condition_arises(step(sq(4, 11), sq(3, 11))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn articles_22_2_and_22_4_waiting_pawn_that_becomes_immobile_stops_counting() {
    // D3-022-12: 待機中の歩兵が最奥段で移動不能になると余分な駒として
    // 数えなくなり対局は継続する。その歩兵が取られれば双方王駒のみとなり
    // 8項の引き分けへ進む。
    let mut game = game_with_codes(
        position(
            Color::White,
            &[
                (sq(7, 1), piece(Color::Black, PieceKind::King)),
                (sq(8, 1), piece(Color::White, PieceKind::Pawn)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );

    // 白歩が段0(白の最奥段)へ不成のまま到達して移動不能になる。
    assert_eq!(game.play(step(sq(8, 1), sq(8, 0))), Ok(GameStatus::Ongoing));
    assert_eq!(
        game.play(step(sq(7, 1), sq(8, 0))),
        draw(DrawReason::PieceExhaustion)
    );
}

//! 第22条の駒枯れの試験。

use super::*;

#[test]
fn article_22_1_bare_side_without_a_capture_loses_immediately() {
    // 条件成立時に裸側の手番でも、余分な駒も最後の王駒も取れなければ
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

/// 第22条第1項・第5項により、条件成立時に余分な駒を持つ側が手番なら直ちに勝ちとなる。
#[test]
fn article_22_1_extra_side_to_move_wins_without_grace() {
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

/// 第22条第2項により、成るまでは勝ちにならず、成った後は第5項から第7項の猶予を適用する。
#[test]
fn article_22_2_pawn_extra_piece_waits_until_promotion() {
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
        Ok(GameStatus::Ongoing)
    );
    let mut declined = game.clone();
    assert_eq!(
        game.play(step(sq(3, 10), sq(4, 11))),
        draw(DrawReason::PieceExhaustion)
    );
    assert_eq!(
        declined.play(step(sq(3, 10), sq(2, 10))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_2_repetition_still_adjudicates_during_the_wait() {
    // 歩兵の成り待機中も反復など他の裁定は通常どおり働く。
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
    // 第3項により、仲人が醉象へ成った後にも第5項から第7項の猶予を適用する。
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
        Ok(GameStatus::Ongoing)
    );
    let mut declined = game.clone();
    assert_eq!(
        game.play(step(sq(3, 8), sq(4, 8))),
        draw(DrawReason::PieceExhaustion)
    );
    assert_eq!(
        declined.play(step(sq(3, 8), sq(2, 8))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_2_and_3_uncapturable_non_capturing_promotion_wins_immediately() {
    // 第2項・第3項・第5項により、成った駒も最後の王駒も取れなければ猶予はない。
    for kind in [PieceKind::Pawn, PieceKind::GoBetween] {
        let mut game = game_with_codes(
            position(
                Color::White,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(4, 7), piece(Color::Black, kind)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );

        assert_eq!(
            game.play(step(sq(11, 11), sq(10, 11))),
            Ok(GameStatus::Ongoing)
        );
        assert_eq!(
            game.play(promoting(sq(4, 7), sq(4, 8))),
            win(Color::Black, WinReason::PieceExhaustion),
            "{kind:?}"
        );
    }
}

#[test]
fn article_22_4_immobile_pawn_and_lance_are_not_winning_extras() {
    // 最奥段で移動不能となった歩兵・香車は勝利を成立させる
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
fn article_22_5_and_6_grace_capture_leads_to_a_draw() {
    // 裸側が余分な駒を次の1手で取れる場合は裁定を
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
    // 猶予の1手で余分な駒を取らなければ、その着手の完了時に
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
fn article_22_5_capturing_the_last_royal_wins_during_grace() {
    // 余分な金将には届かなくても最後の王駒を取れるなら第5項の猶予がある。
    // 捕獲すれば第21条第1項で勝ち、見送れば第22条第7項で負けとなる。
    for extra_color in Color::ALL {
        for last_royal in [piece(extra_color, PieceKind::King), prince(extra_color)] {
            let bare_color = extra_color.opposite();
            let mut game = game_with_codes(
                position(
                    extra_color,
                    &[
                        (sq(4, 4), last_royal),
                        (sq(0, 0), piece(extra_color, PieceKind::GoldGeneral)),
                        (sq(4, 6), piece(bare_color, PieceKind::King)),
                        (sq(4, 5), piece(bare_color, PieceKind::Pawn)),
                    ],
                ),
                &[RuleCode::R1],
            );

            assert_eq!(game.play(step(sq(4, 4), sq(4, 5))), Ok(GameStatus::Ongoing));
            let mut declined = game.clone();
            assert_eq!(
                game.play(step(sq(4, 6), sq(4, 5))),
                win(bare_color, WinReason::RoyalCapture)
            );
            assert_eq!(
                declined.play(step(sq(4, 6), sq(4, 7))),
                win(extra_color, WinReason::PieceExhaustion)
            );
        }
    }
}

#[test]
fn article_22_8_one_royal_each_draws() {
    // 双方に王駒1枚ずつだけが残れば自動的に引き分けとする。
    for black_royal in [piece(Color::Black, PieceKind::King), prince(Color::Black)] {
        for white_royal in [piece(Color::White, PieceKind::King), prince(Color::White)] {
            let mut game = game_with_codes(
                position(
                    Color::Black,
                    &[(sq(0, 0), black_royal), (sq(11, 11), white_royal)],
                ),
                &[RuleCode::R1],
            );
            assert_eq!(
                game.play(step(sq(0, 0), sq(0, 1))),
                draw(DrawReason::PieceExhaustion)
            );
        }
    }
}

#[test]
fn article_22_9_royal_pair_to_move_wins_without_grace() {
    // 第9項では王将と太子の一方を余分な駒と数える。
    // 王駒1枚の側が歩兵を取って条件が成立すると、2枚の側に手番があるので第1項の勝ち。
    for extra_color in Color::ALL {
        let bare_color = extra_color.opposite();
        let mut game = game_with_codes(
            position(
                bare_color,
                &[
                    (sq(0, 0), piece(extra_color, PieceKind::King)),
                    (sq(2, 0), prince(extra_color)),
                    (sq(8, 9), piece(extra_color, PieceKind::Pawn)),
                    (sq(8, 8), piece(bare_color, PieceKind::King)),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(
            game.play(step(sq(8, 8), sq(8, 9))),
            win(extra_color, WinReason::PieceExhaustion)
        );
    }
}

#[test]
fn article_22_9_uncapturable_royal_pair_wins_immediately() {
    // 王駒1枚の側に手番があっても、2枚のいずれも取れなければ第5項の猶予はない。
    let mut game = game_with_codes(
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
        game.play(step(sq(0, 0), sq(0, 1))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_9_capturing_either_royal_during_grace_draws() {
    // 第9項では王将と太子のどちらを取っても余分な駒を取り除いたことになる。
    // 第5項の猶予で取れば第6項・第8項の引き分け、見送れば第7項の勝ちとなる。
    for extra_color in Color::ALL {
        let bare_color = extra_color.opposite();
        let king = piece(extra_color, PieceKind::King);
        let prince = prince(extra_color);
        for (capturable, remaining) in [(king, prince), (prince, king)] {
            let mut game = game_with_codes(
                position(
                    extra_color,
                    &[
                        (sq(5, 5), capturable),
                        (sq(0, 0), remaining),
                        (sq(4, 7), piece(bare_color, PieceKind::King)),
                        (sq(5, 6), piece(bare_color, PieceKind::Pawn)),
                    ],
                ),
                &[RuleCode::R1],
            );

            assert_eq!(game.play(step(sq(5, 5), sq(5, 6))), Ok(GameStatus::Ongoing));
            let mut declined = game.clone();
            assert_eq!(
                game.play(step(sq(4, 7), sq(5, 6))),
                draw(DrawReason::PieceExhaustion)
            );
            assert_eq!(
                declined.play(step(sq(4, 7), sq(3, 7))),
                win(extra_color, WinReason::PieceExhaustion)
            );
        }
    }
}

#[test]
fn article_22_9_grace_allows_capturing_a_defended_royal() {
    // 第5項・第9項の猶予は、王将と太子が互いを守っていても与える。
    // 捕獲後は第8項で終局するため、残る王駒の利きによって猶予を詰みと裁定してはならない。
    let mut game = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(2, 0), piece(Color::Black, PieceKind::King)),
                (sq(1, 1), prince(Color::Black)),
                (sq(0, 0), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R1],
    );
    assert_eq!(game.play(step(sq(2, 0), sq(1, 0))), Ok(GameStatus::Ongoing));
    for captured in [sq(1, 0), sq(1, 1)] {
        assert_eq!(
            game.clone().play(step(sq(0, 0), captured)),
            draw(DrawReason::PieceExhaustion)
        );
    }
    assert_eq!(
        game.play(step(sq(0, 0), sq(0, 1))),
        win(Color::Black, WinReason::PieceExhaustion)
    );
}

#[test]
fn article_22_9_two_extra_pieces_do_not_meet_the_condition() {
    // 第9項で数えた余分な駒が双方に1枚ずつ、または片側に2枚あれば第1項は成立しない。
    for additional_piece in [
        piece(Color::White, PieceKind::GoldGeneral),
        piece(Color::Black, PieceKind::GoldGeneral),
        prince(Color::White),
    ] {
        let mut game = game_with_codes(
            position(
                Color::Black,
                &[
                    (sq(0, 0), piece(Color::Black, PieceKind::King)),
                    (sq(2, 0), prince(Color::Black)),
                    (sq(11, 11), piece(Color::White, PieceKind::King)),
                    (sq(9, 9), additional_piece),
                ],
            ),
            &[RuleCode::R1],
        );
        assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
    }
}

/// 第22条第2項・第5項から第7項により、捕獲を伴う成りの後も取り返しの猶予がある。
#[test]
fn article_22_2_capturing_promotion_that_creates_the_condition_takes_the_grace_path() {
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
fn article_22_2_and_4_waiting_pawn_that_becomes_immobile_stops_counting() {
    // 待機中の歩兵が最奥段で移動不能になると余分な駒として
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

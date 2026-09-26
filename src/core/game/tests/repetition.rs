//! 第24〜25条の同一局面と反復規則の選択、および第31条の反復に関するローカルルールの試験。

use super::*;

#[test]
fn articles_24_1_and_31_r1_f1_cycle_draws_at_ply_12() {
    // D3-024-01/D3-031-01: F1(初期局面の仲人往復)は④⑧⑫の完了時に初期
    // 局面の2〜4回目を生じさせ、全12手が非攻撃的なため⑫で引き分けになる。
    // 対局開始局面は履歴上の第1回として数える。
    let mut game = Game::new(Rules::ENGINE_DEFAULT);
    let f1 = [
        step(sq(3, 4), sq(3, 5)),
        step(sq(8, 7), sq(8, 6)),
        step(sq(3, 5), sq(3, 4)),
        step(sq(8, 6), sq(8, 7)),
    ];

    for ply in 1..=11 {
        assert_eq!(
            game.play(f1[(ply - 1) % f1.len()]),
            Ok(GameStatus::Ongoing),
            "ended at ply {ply}"
        );
    }
    assert_eq!(game.play(f1[3]), draw(DrawReason::Repetition));
    assert_eq!(game.ply_count(), 12);
}

#[test]
fn article_24_1_b_side_to_move_distinguishes_identical_boards() {
    // D3-024-02: 盤面が同じでも手番側が異なる2局面は同一局面ではない。
    // 黒金の往復2周(4手)と白王将の三角移動(3手)で、7手目に開始盤面が
    // 白手番として再現される。R2はこれを既出局面の再現として禁止しない。
    let mut game = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(8, 8), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R2, RuleCode::E2],
    );

    let plies = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(7, 8)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(7, 8), sq(7, 7)),
        step(sq(3, 3), sq(3, 4)),
        step(sq(7, 7), sq(8, 8)),
        step(sq(3, 4), sq(3, 3)),
    ];
    for (index, mv) in plies.into_iter().enumerate() {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
    }
}

#[test]
fn article_24_1_c_lion_trigger_state_distinguishes_positions() {
    // D3-024-03: 先獅子による直後の捕獲禁止の有無が異なる2局面は同一局面
    // ではない。先獅子あり局面の既出は、後日の先獅子なし同一盤面の再現を
    // 禁止しない。
    let mut game = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(11, 11), piece(Color::Black, PieceKind::King)),
                (sq(5, 4), piece(Color::Black, PieceKind::Pawn)),
                (sq(8, 8), piece(Color::Black, PieceKind::Lion)),
                (sq(8, 7), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(2, 2), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(5, 5), piece(Color::White, PieceKind::Lion)),
                (sq(0, 0), piece(Color::White, PieceKind::Rook)),
                (sq(11, 0), piece(Color::White, PieceKind::King)),
            ],
        ),
        &[RuleCode::R2],
    );

    // 黒歩が白獅子を取る。残る黒獅子(8,8)には金(8,7)の足があり、先獅子の
    // 捕獲禁止状態(第15条)を持つ局面が既出として記録される。
    assert_eq!(game.play(step(sq(5, 4), sq(5, 5))), Ok(GameStatus::Ongoing));
    assert_eq!(game.play(step(sq(0, 0), sq(0, 1))), Ok(GameStatus::Ongoing));
    assert_eq!(game.play(step(sq(2, 2), sq(2, 3))), Ok(GameStatus::Ongoing));
    assert_eq!(game.play(step(sq(0, 1), sq(0, 0))), Ok(GameStatus::Ongoing));
    // 同じ盤面へ先獅子なしで戻る着手は、同一局面の再現ではないため受理される。
    assert_eq!(game.play(step(sq(2, 3), sq(2, 2))), Ok(GameStatus::Ongoing));
    // 対照: 先獅子と無関係な既出局面(2手目完了時)の再現は禁止される。
    let forbidden = step(sq(0, 0), sq(0, 1));
    assert_eq!(
        game.play(forbidden),
        Err(GameError::IllegalMove {
            mv: forbidden,
            cause: IllegalMoveCause::Repetition,
        })
    );
}

#[test]
fn article_24_1_occurrences_accumulate_across_different_paths() {
    // D3-024-05: 同一性は局面に基づき到達手順に依存しない。異なる経路
    // (黒金2枚を交互に往復)による出現も合算され、4回目で裁定される。
    let mut game = game_with_codes(
        position(
            Color::Black,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(3, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(5, 3), piece(Color::Black, PieceKind::GoldGeneral)),
                (sq(11, 11), piece(Color::White, PieceKind::King)),
                (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            ],
        ),
        &[RuleCode::R1],
    );

    let cycle_a = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];
    let cycle_b = [
        step(sq(5, 3), sq(5, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(5, 4), sq(5, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];
    let plies: Vec<Move> = cycle_a
        .into_iter()
        .chain(cycle_b)
        .chain(cycle_a.into_iter().take(3))
        .collect();
    for (index, mv) in plies.into_iter().enumerate() {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
    }
    assert_eq!(game.play(cycle_a[3]), draw(DrawReason::Repetition));
}

#[test]
fn article_31_r1_requires_twelve_reversible_plies_from_start() {
    // D3-031-11: 対局開始から可逆手が12手続くまでは、4回以上同じ局面が
    // 現れても裁定しない(lishogi-bot.md「反復裁定の整合」、第31条R1)。
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Lion)),
            (sq(8, 8), piece(Color::White, PieceKind::Lion)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let cycle = [step(sq(3, 3), sq(3, 3)), step(sq(8, 8), sq(8, 8))];
    let mut game = game_with_codes(start.clone(), &[RuleCode::R1]);
    for ply in 1..=11 {
        assert_eq!(
            game.play(cycle[(ply - 1) % 2]),
            Ok(GameStatus::Ongoing),
            "ply {ply}"
        );
        if ply % 2 == 0 {
            assert_eq!(game.position(), &start);
        }
    }
    assert_eq!(game.play(cycle[1]), draw(DrawReason::Repetition));
    assert_eq!(game.position(), &start);
}

#[test]
fn article_31_r1_requires_twelve_reversible_plies_after_an_irreversible_move() {
    // D3-031-12: 不可逆手の前に可逆手を11手指していても、その後の可逆手が
    // 12手になるまで裁定しない(lishogi-bot.md「反復裁定の整合」、第31条R1)。
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Lion)),
            (sq(8, 8), piece(Color::White, PieceKind::Lion)),
            (sq(6, 8), piece(Color::White, PieceKind::Pawn)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    );
    let cycle = [step(sq(3, 3), sq(3, 3)), step(sq(8, 8), sq(8, 8))];
    let mut game = game_with_codes(start, &[RuleCode::R1]);
    for ply in 1..=11 {
        assert_eq!(game.play(cycle[(ply - 1) % 2]), Ok(GameStatus::Ongoing));
    }
    assert_eq!(game.play(step(sq(6, 8), sq(6, 7))), Ok(GameStatus::Ongoing));
    for ply in 1..=11 {
        assert_eq!(
            game.play(cycle[(ply - 1) % 2]),
            Ok(GameStatus::Ongoing),
            "reversible ply {ply} after pawn move"
        );
    }
    assert_eq!(game.play(cycle[1]), draw(DrawReason::Repetition));
}

#[test]
fn article_31_r1_sole_continuous_checker_loses() {
    // D3-031-02: 反復区間を通じて一方(黒)のすべての着手だけが攻撃的着手
    // (連続王手)なら、4回目の同一局面の出現時に攻撃側の負けとなる。
    let mut game = game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::FreeKing)),
            (sq(3, 10), piece(Color::White, PieceKind::King)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
        ],
    ));
    let cycle = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];

    for ply in 1..=11 {
        assert_eq!(
            game.play(cycle[(ply - 1) % cycle.len()]),
            Ok(GameStatus::Ongoing),
            "ended at ply {ply}"
        );
    }
    assert_eq!(
        game.play(cycle[3]),
        win(Color::White, WinReason::Repetition)
    );
}

#[test]
fn article_31_r1_capture_threats_count_and_standing_threats_do_not() {
    // D3-031-03: 攻撃的着手は王手に限らず、動かした駒が到達升から相手の
    // いずれかの駒を直ちに捕獲できる状態にする着手を含む。一方、動かさな
    // かった駒による既存の捕獲脅威は含めない。
    // (i) 黒飛が両往復升から白歩を照準し続ける: 黒が継続攻撃側となり負け。
    let mut threatening = game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
            (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    let cycle = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];
    for ply in 1..=11 {
        assert_eq!(
            threatening.play(cycle[(ply - 1) % cycle.len()]),
            Ok(GameStatus::Ongoing),
            "ended at ply {ply}"
        );
    }
    assert_eq!(
        threatening.play(cycle[3]),
        win(Color::White, WinReason::Repetition)
    );

    // (ii) 据え置きの黒飛が白歩を照準したまま、黒は金だけを往復する:
    // 既存の脅威は数えず双方非攻撃的なので引き分けになる。
    let mut standing = game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::Rook)),
            (sq(5, 5), piece(Color::Black, PieceKind::GoldGeneral)),
            (sq(3, 9), piece(Color::White, PieceKind::Pawn)),
            (sq(8, 8), piece(Color::White, PieceKind::GoldGeneral)),
            (sq(11, 11), piece(Color::White, PieceKind::King)),
        ],
    ));
    let gold_cycle = [
        step(sq(5, 5), sq(5, 6)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(5, 6), sq(5, 5)),
        step(sq(8, 7), sq(8, 8)),
    ];
    for ply in 1..=11 {
        assert_eq!(
            standing.play(gold_cycle[(ply - 1) % gold_cycle.len()]),
            Ok(GameStatus::Ongoing),
            "ended at ply {ply}"
        );
    }
    assert_eq!(standing.play(gold_cycle[3]), draw(DrawReason::Repetition));
}

#[test]
fn article_31_r1_mutual_perpetual_attacks_draw() {
    // D3-031-04: 双方のすべての着手が攻撃的着手なら、4回目の同一局面の
    // 出現時点で引き分けとなる。黒奔王は連続王手、白飛は黒歩の照準継続。
    let mut game = game(position(
        Color::Black,
        &[
            (sq(0, 0), piece(Color::Black, PieceKind::King)),
            (sq(3, 3), piece(Color::Black, PieceKind::FreeKing)),
            (sq(8, 0), piece(Color::Black, PieceKind::Pawn)),
            (sq(3, 10), piece(Color::White, PieceKind::King)),
            (sq(8, 8), piece(Color::White, PieceKind::Rook)),
        ],
    ));
    let cycle = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];

    for ply in 1..=11 {
        assert_eq!(
            game.play(cycle[(ply - 1) % cycle.len()]),
            Ok(GameStatus::Ongoing),
            "ended at ply {ply}"
        );
    }
    assert_eq!(game.play(cycle[3]), draw(DrawReason::Repetition));
}

#[test]
fn article_31_r2_applies_to_the_checked_side_but_never_to_captures() {
    // D3-031-07: 王手を受けている側にもR2の禁止は適用される(適用除外の
    // 不採用)。D3-031-08: 捕獲を含む着手は既出局面を再現し得ず、決して
    // 禁止されない(game-referee.md 6節の補題)。
    let mut game = game_with_codes(
        position(
            Color::White,
            &[
                (sq(0, 1), piece(Color::Black, PieceKind::King)),
                (sq(1, 0), piece(Color::White, PieceKind::King)),
                (sq(10, 9), piece(Color::White, PieceKind::GoBetween)),
            ],
        ),
        &[RuleCode::R2, RuleCode::E2],
    );
    for mv in [
        step(sq(10, 9), sq(10, 8)),
        step(sq(0, 1), sq(0, 0)),
        step(sq(10, 8), sq(10, 9)),
    ] {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }

    // 白王将の利きの中で王手を受けたままでも、非捕獲の戻りは禁止される。
    let forbidden_noncapture = step(sq(0, 0), sq(0, 1));
    assert_eq!(
        game.play(forbidden_noncapture),
        Err(GameError::IllegalMove {
            mv: forbidden_noncapture,
            cause: IllegalMoveCause::Repetition,
        })
    );
    // 捕獲の着手は履歴と独立に合法であり、王駒捕獲の勝ちが成立する。
    assert_eq!(
        game.play(step(sq(0, 0), sq(1, 0))),
        win(Color::Black, WinReason::RoyalCapture)
    );
}

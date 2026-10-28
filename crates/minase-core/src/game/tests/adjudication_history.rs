//! 公開APIで着手列を再生する、反復と詰み・合法手なしの境界の試験。

use super::*;

// 白飛(0,5)が黒王(0,0)と逃げ先(0,1)に利き、白金(2,1)が(1,0)と(1,1)を押さえる。
// 黒飛が(1,2)にいれば(0,2)への合駒で逃れられるが、(1,8)では間に合わない。
// 遠方の獅子はじっとで手番だけを進めるために置き、黒王の受けには使えない。
fn r1_pass_mate_pieces() -> Vec<(Square, PieceCode)> {
    vec![
        (sq(0, 0), piece(Color::Black, PieceKind::King)),
        (sq(1, 2), piece(Color::Black, PieceKind::Rook)),
        (sq(2, 9), piece(Color::Black, PieceKind::Pawn)),
        (sq(10, 10), piece(Color::Black, PieceKind::Lion)),
        (sq(0, 5), piece(Color::White, PieceKind::Rook)),
        (sq(2, 1), piece(Color::White, PieceKind::GoldGeneral)),
        (sq(1, 11), piece(Color::White, PieceKind::Pawn)),
        (sq(8, 3), piece(Color::White, PieceKind::Lion)),
        (sq(11, 0), piece(Color::White, PieceKind::King)),
    ]
}

// 黒飛を(1,8)へ戻した白手番の局面を、この9手の1・5・9手目に作る。
// 白飛は毎回(2,5)へ退いて黒王への王手を解き、その間に黒飛が(1,2)へ出る。
// 白飛が(0,5)へ戻るときには合駒が残るので、途中では詰まない。
fn r1_pass_mate_sequence() -> [Move; 9] {
    [
        step(sq(1, 2), sq(1, 8)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(1, 8), sq(1, 2)),
        step(sq(2, 5), sq(0, 5)),
        step(sq(1, 2), sq(1, 8)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(1, 8), sq(1, 2)),
        step(sq(2, 5), sq(0, 5)),
        step(sq(1, 2), sq(1, 8)),
    ]
}

/// 第31条R1、第21条第3項c。4回目を作る候補手を含めて11手なら反復裁定はなく詰みとなる。
/// 12手なら白の継続攻撃による黒の即時勝利が成立し、その候補手は詰みの逃れになる。
#[test]
fn article_21_3_r1_eleven_and_twelve_reversible_plies_change_mate() {
    let white_pass = step(sq(8, 3), sq(8, 3));
    let black_pass = step(sq(10, 10), sq(10, 10));
    for prefix_pass in [false, true] {
        let mut game = game_with_codes(
            position(
                if prefix_pass {
                    Color::White
                } else {
                    Color::Black
                },
                &r1_pass_mate_pieces(),
            ),
            &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2],
        );
        if prefix_pass {
            // 黒飛がまだ(1,2)にいるので、最初の白じっとでは詰まない。
            assert_eq!(game.play(white_pass), Ok(GameStatus::Ongoing));
        }
        for mv in r1_pass_mate_sequence() {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "{mv:?}");
        }

        // 白じっとの後は合駒がなく、黒じっとによる反復だけが逃れの候補となる。
        // 前置きなしでは1・5・9手目を11手目で、ありでは2・6・10手目を12手目で再現する。
        // 白飛は退避先でも黒歩(2,9)を狙うので白の全着手が攻撃的であり、黒じっとは非攻撃的。
        if prefix_pass {
            assert_eq!(game.play(white_pass), Ok(GameStatus::Ongoing));
            assert_eq!(game.ply_count(), 11);
            assert!(game.legal_moves().contains(&black_pass));
            assert_eq!(
                game.play(black_pass),
                win(Color::Black, WinReason::Repetition)
            );
        } else {
            assert_eq!(game.play(white_pass), win(Color::White, WinReason::Mate));
            assert_eq!(game.ply_count(), 10);
        }
    }
}

/// 第31条R1、第21条第3項c・d。可逆手が12手に達しても、3回目の再現は詰みの逃れにならない。
#[test]
fn article_21_3_r1_third_occurrence_at_twelve_plies_does_not_rescue_mate() {
    let mut game = game_with_codes(
        position(Color::White, &r1_pass_mate_pieces()),
        &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2],
    );
    let white_pass = step(sq(8, 3), sq(8, 3));
    // 白飛と黒飛が途中で1段ずつ遠回りするため、黒飛が(1,8)に戻る白手番の局面は
    // 2・10手目だけに現れる。通常の往復で生じる6手目の出現を、この遠回りで避ける。
    for mv in [
        white_pass,
        step(sq(1, 2), sq(1, 8)),
        step(sq(0, 5), sq(2, 5)),
        step(sq(1, 8), sq(1, 2)),
        step(sq(2, 5), sq(2, 6)),
        step(sq(1, 2), sq(1, 3)),
        step(sq(2, 6), sq(2, 5)),
        step(sq(1, 3), sq(1, 2)),
        step(sq(2, 5), sq(0, 5)),
        step(sq(1, 2), sq(1, 8)),
    ] {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "{mv:?}");
    }
    // 11手目の白じっとで受けが尽きる。黒じっとは12手目だが、再現はまだ3回目である。
    assert_eq!(game.ply_count(), 10);
    assert_eq!(game.play(white_pass), win(Color::White, WinReason::Mate));
}

/// 第31条R1、第21条第3項c・d。不可逆手より前の可逆手は、反復による詰み回避に数えない。
/// 捕獲、成り、成らない歩兵の移動、成らない香車の移動のいずれも、12手の前提を解除する。
#[test]
fn article_21_3_r1_irreversible_move_resets_the_mate_escape_prerequisite() {
    for (name, kind, irreversible, captured) in [
        ("pawn", PieceKind::Pawn, step(sq(6, 8), sq(6, 7)), None),
        ("lance", PieceKind::Lance, step(sq(6, 8), sq(6, 6)), None),
        (
            "promotion",
            PieceKind::SilverGeneral,
            promoting(sq(6, 4), sq(6, 3)),
            None,
        ),
        (
            "capture",
            PieceKind::Rook,
            step(sq(4, 8), sq(4, 7)),
            Some(piece(Color::Black, PieceKind::Pawn)),
        ),
    ] {
        let mut pieces = r1_pass_mate_pieces();
        pieces.push((irreversible.from, piece(Color::White, kind)));
        if let Some(captured) = captured {
            pieces.push((irreversible.to, captured));
        }
        let mut game = game_with_codes(
            position(Color::Black, &pieces),
            &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2],
        );
        let black_pass = step(sq(10, 10), sq(10, 10));
        let white_pass = step(sq(8, 3), sq(8, 3));
        // 両獅子のじっとで可逆手を11手蓄積する。この間は黒飛の合駒が残る。
        for ply in 0..11 {
            let mv = if ply % 2 == 0 { black_pass } else { white_pass };
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "{name}: ply {ply}");
        }
        // 12手目の白の不可逆手は王周辺の配置を変えないが、ここから可逆手数を数え直す。
        assert_eq!(game.play(irreversible), Ok(GameStatus::Ongoing), "{name}");
        for mv in r1_pass_mate_sequence() {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "{name}: {mv:?}");
        }
        assert_eq!(game.ply_count(), 21);
        // 13・17・21手目の局面を黒じっとで23手目に再現しても、不可逆手の後は11手しかない。
        // したがって22手目の白じっとを受ける反復勝ちは成立せず、黒は詰む。
        assert_eq!(
            game.play(white_pass),
            win(Color::White, WinReason::Mate),
            "{name}"
        );
    }
}

/// 第21条第3項c・d、第31条R1。受けの側の反復負けは、詰みの逃れに数えない。
/// 第32条E1を併用すると詰みでは終わらず、同じ候補手を実際に指した時点で反復負けとなる。
#[test]
fn article_21_3_r1_repetition_loss_is_not_an_escape_through_play() {
    let mut pieces = r1_pass_mate_pieces();
    // 黒歩(2,9)を除いて白飛の退避を非攻撃的にし、白歩(9,10)を足して黒獅子のじっとを
    // 攻撃的にする。黒飛の全往復も白歩(1,11)を狙うので、黒だけが攻撃を継続する。
    pieces.retain(|&(square, _)| square != sq(2, 9));
    pieces.push((sq(9, 10), piece(Color::White, PieceKind::Pawn)));
    for e1 in [false, true] {
        let mut codes = vec![RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E2];
        if e1 {
            codes.push(RuleCode::E1);
        }
        let mut game = game_with_codes(position(Color::White, &pieces), &codes);
        let white_pass = step(sq(8, 3), sq(8, 3));
        assert_eq!(game.play(white_pass), Ok(GameStatus::Ongoing));
        for mv in r1_pass_mate_sequence() {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "e1={e1}: {mv:?}");
        }
        // 黒じっとは2・6・10手目の局面の4回目を12手目に作るが、その裁定は黒の負けである。
        if e1 {
            assert_eq!(game.play(white_pass), Ok(GameStatus::Ongoing));
            assert_eq!(game.result(), None);
            assert_eq!(
                game.play(step(sq(10, 10), sq(10, 10))),
                win(Color::White, WinReason::Repetition)
            );
        } else {
            assert_eq!(game.play(white_pass), win(Color::White, WinReason::Mate));
        }
    }
}

/// 第21条第3項a、第31条R3、第27条第4項。3回目までは唯一の逃げを指せるが、4回目は禁止される。
/// 王駒を取られる着手は残っているので、禁止後の終局理由は合法手なしではなく詰みである。
#[test]
fn articles_21_3_and_31_r3_fourth_occurrence_removes_the_only_escape() {
    let pieces = [
        (sq(0, 1), piece(Color::Black, PieceKind::King)),
        (sq(11, 0), piece(Color::White, PieceKind::Rook)),
        (sq(11, 11), piece(Color::White, PieceKind::Bishop)),
        (sq(10, 9), piece(Color::White, PieceKind::King)),
    ];
    let cycle = [
        step(sq(10, 9), sq(10, 8)),
        step(sq(0, 1), sq(0, 0)),
        step(sq(10, 8), sq(10, 9)),
        step(sq(0, 0), sq(0, 1)),
    ];
    for e1 in [false, true] {
        let mut codes = vec![RuleCode::L0, RuleCode::P0, RuleCode::R3, RuleCode::E2];
        if e1 {
            codes.push(RuleCode::E1);
        }
        let mut game = game_with_codes(position(Color::White, &pieces), &codes);
        // 白王が往復する間に、黒王は飛車の利く(0,0)と安全な(0,1)を往復する。
        // 4・8手目の初期局面への帰還は、それぞれ2・3回目なので許される。
        for _ in 0..2 {
            for mv in &cycle[..3] {
                assert_eq!(game.play(*mv), Ok(GameStatus::Ongoing));
            }
            assert!(game.legal_moves().contains(&cycle[3]));
            assert_eq!(game.play(cycle[3]), Ok(GameStatus::Ongoing));
        }
        assert_eq!(game.play(cycle[0]), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(cycle[1]), Ok(GameStatus::Ongoing));
        // 11手目の後、(0,1)への帰還だけが初期局面の4回目を生じさせる。
        // 残る(1,0)は白飛、(1,1)は白角に取られるので、第21条第3項の逃れにはならない。
        if e1 {
            assert_eq!(game.play(cycle[2]), Ok(GameStatus::Ongoing));
            let legal = game.legal_moves();
            assert_eq!(legal.len(), 2);
            assert!(legal.contains(&step(sq(0, 0), sq(1, 0))));
            assert!(legal.contains(&step(sq(0, 0), sq(1, 1))));
            assert_eq!(
                game.play(cycle[3]),
                Err(GameError::IllegalMove {
                    mv: cycle[3],
                    cause: IllegalMoveCause::Repetition,
                })
            );
            assert_eq!(game.result(), None);
        } else {
            assert_eq!(game.play(cycle[2]), win(Color::White, WinReason::Mate));
        }
    }
}

/// 第23条第1項・第3項、第31条R3、第27条第4項。4回目を作る唯一の合法手が禁止されると負ける。
/// 第32条E1は詰みだけを省くので、この合法手なしによる敗北はE1でも成立する。
#[test]
fn articles_23_and_31_r3_fourth_occurrence_removes_the_last_legal_move() {
    let mut pieces = vec![
        (sq(0, 0), piece(Color::Black, PieceKind::GoBetween)),
        (sq(11, 11), piece(Color::Black, PieceKind::King)),
        (sq(10, 10), piece(Color::Black, PieceKind::Pawn)),
        (sq(10, 11), piece(Color::Black, PieceKind::Pawn)),
        (sq(11, 10), piece(Color::Black, PieceKind::Pawn)),
        (sq(5, 5), piece(Color::White, PieceKind::King)),
    ];
    // 0筋の歩兵を最奥段まで並べて、仲人の往復先を(0,0)と(0,1)に限る。
    // 黒王とその周囲の歩兵も互いに塞がって動けず、黒の可動駒は仲人だけである。
    pieces.extend((2..=11).map(|rank| (sq(0, rank), piece(Color::Black, PieceKind::Pawn))));
    let cycle = [
        step(sq(5, 5), sq(5, 4)),
        step(sq(0, 0), sq(0, 1)),
        step(sq(5, 4), sq(5, 5)),
        step(sq(0, 1), sq(0, 0)),
    ];
    for e1 in [false, true] {
        let mut codes = vec![RuleCode::L0, RuleCode::P0, RuleCode::R3, RuleCode::E2];
        if e1 {
            codes.push(RuleCode::E1);
        }
        let mut game = game_with_codes(position(Color::White, &pieces), &codes);
        for _ in 0..2 {
            for mv in &cycle[..3] {
                assert_eq!(game.play(*mv), Ok(GameStatus::Ongoing));
            }
            // 2・3回目の再現では、仲人の帰還が唯一の合法手として残る。
            assert_eq!(game.legal_moves(), vec![cycle[3]]);
            assert_eq!(game.play(cycle[3]), Ok(GameStatus::Ongoing));
        }
        assert_eq!(game.play(cycle[0]), Ok(GameStatus::Ongoing));
        assert_eq!(game.play(cycle[1]), Ok(GameStatus::Ongoing));
        // 次の黒の帰還は4回目の再現となり禁止される。白王は黒王に利いていない。
        assert_eq!(game.play(cycle[2]), win(Color::White, WinReason::Stalemate));
        assert_eq!(
            game.result(),
            Some(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Stalemate,
            })
        );
    }
}

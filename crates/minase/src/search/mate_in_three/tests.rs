//! 学習技法の導入の設計書「フェーズ2」とRULES.md第21条・第22条・第31条R1の固定例。

use super::{Error, Verdict, all_replies_lose, mate_in_three};
use Color::{Black, White};
use PieceKind::{CrownPrince, DrunkElephant, FreeKing, GoldGeneral, King, Pawn, Rook};
use minase_core::{
    Color, Game, GameError, GameResult, GameStatus, Move, PieceCode, PieceKind, PositionBuilder,
    Rules, Square, WinReason,
};

fn sq(file: u8, rank: u8) -> Square {
    Square::new(file, rank).unwrap()
}

fn step(from: (u8, u8), to: (u8, u8)) -> Move {
    Move {
        from: sq(from.0, from.1),
        mid: None,
        to: sq(to.0, to.1),
        promote: false,
    }
}

fn game(side: Color, pieces: &[(u8, u8, Color, PieceKind)]) -> Game {
    let mut builder = PositionBuilder::new(side);
    for &(file, rank, color, kind) in pieces {
        let piece = match kind {
            CrownPrince => PieceCode::new_promoted(color, kind),
            _ => PieceCode::new(color, kind),
        };
        builder.put(sq(file, rank), piece.unwrap()).unwrap();
    }
    Game::from_position(Rules::ENGINE_DEFAULT, builder.finish().unwrap())
}

fn win(winner: Color, reason: WinReason) -> GameStatus {
    GameStatus::Finished(GameResult::Win { winner, reason })
}

// 黒飛(3,1)が白歩(0,1)を取って王手。白王(0,0)がこの飛車を取る以外に受けはない。
// 黒飛(2,5)が(0,5)へ回れば、0筋を押さえ、1筋の逃げ先は黒王と白の歩兵が塞ぐ。
// 遠方の白歩は、飛車を取った時点で駒枯れが成立するのを防ぐ。
fn three_ply_pieces() -> Vec<(u8, u8, Color, PieceKind)> {
    vec![
        (2, 2, Black, King),
        (3, 1, Black, Rook),
        (2, 5, Black, Rook),
        (0, 0, White, King),
        (0, 1, White, Pawn),
        (1, 0, White, Pawn),
        (11, 5, White, Pawn),
    ]
}

#[test]
fn last_royal_capture_wins_at_m1_without_changing_the_input() {
    // 第21条第1項。王駒の実捕獲は王手と別に候補へ含める。
    for royal in [King, CrownPrince] {
        let game = game(
            Black,
            &[
                (11, 11, Black, King),
                (0, 5, Black, Rook),
                (0, 0, White, royal),
                (11, 5, White, Pawn),
            ],
        );
        let original = game.clone();
        let mut played = game.clone();
        assert_eq!(
            played.play(step((0, 5), (0, 0))).unwrap(),
            win(Black, WinReason::RoyalCapture)
        );
        assert_eq!(mate_in_three(&game).unwrap(), Verdict::Win);
        assert!(game.same_state(&original));
    }
}

#[test]
fn checking_move_can_win_by_the_referees_mate_adjudication() {
    // 第21条第2項。黒奔王(1,1)を黒王(2,2)が守り、白王の全逃げ先を押さえる。
    let game = game(
        Black,
        &[
            (2, 2, Black, King),
            (1, 2, Black, FreeKing),
            (0, 0, White, King),
            (11, 5, White, Pawn),
        ],
    );
    let mut played = game.clone();
    assert_eq!(
        played.play(step((1, 2), (1, 1))).unwrap(),
        win(Black, WinReason::Mate)
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::Win);
}

#[test]
fn m1_repetition_loss_is_not_a_win() {
    // 第31条R1。黒王の横移動へ白飛が追従して連続王手をかけ、12手目で白が負ける。
    // 白王は遠く、他の飛車の王手には黒王の逃げがあり、3手では詰まない。
    let mut game = game(
        Black,
        &[
            (0, 0, Black, King),
            (10, 5, Black, Pawn),
            (11, 11, White, King),
            (0, 3, White, Rook),
        ],
    );
    let cycle = [
        step((0, 0), (1, 0)),
        step((0, 3), (1, 3)),
        step((1, 0), (0, 0)),
        step((1, 3), (0, 3)),
    ];
    for ply in 0..11 {
        assert_eq!(game.play(cycle[ply % 4]).unwrap(), GameStatus::Ongoing);
    }
    let original = game.clone();
    let mut played = game.clone();
    assert_eq!(
        played.play(cycle[3]).unwrap(),
        win(Black, WinReason::Repetition)
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
    assert!(game.same_state(&original));
}

#[test]
fn losing_m2_is_not_a_defense() {
    // 第31条R1。白飛は黒歩を狙い続けるが、黒飛は王手と非攻撃的な退避を交互に指す。
    // 11手目の黒の王手に対し、12手目の白飛の帰還は白自身の反復負けになる。
    // 白王が黒飛を取る受けには、もう1枚の黒飛が(0,5)へ回って詰める。
    let mut game = game(
        Black,
        &[
            (2, 2, Black, King),
            (0, 1, Black, Rook),
            (2, 5, Black, Rook),
            (8, 11, Black, Pawn),
            (0, 0, White, King),
            (1, 0, White, Pawn),
            (8, 8, White, Rook),
        ],
    );
    let cycle = [
        step((0, 1), (3, 1)),
        step((8, 8), (8, 9)),
        step((3, 1), (0, 1)),
        step((8, 9), (8, 8)),
    ];
    for ply in 0..10 {
        assert_eq!(game.play(cycle[ply % 4]).unwrap(), GameStatus::Ongoing);
    }
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::Win);
    assert_eq!(game.play(cycle[2]).unwrap(), GameStatus::Ongoing);
    let mut reply = game.clone();
    assert_eq!(
        reply.play(cycle[3]).unwrap(),
        win(Black, WinReason::Repetition)
    );
    // このm1の応手全体についても確かめ、別のm1の勝ちで反復の枝の誤りを隠さない。
    assert!(all_replies_lose(&game, Black).unwrap());
}

#[test]
fn checking_moves_with_escapes_do_not_win() {
    // 支えのない飛車の王手は、離れた王と歩兵を3手では詰められない。
    let game = game(
        Black,
        &[
            (11, 11, Black, King),
            (3, 3, Black, Rook),
            (0, 0, White, King),
            (10, 5, White, Pawn),
        ],
    );
    let mut line = game.clone();
    assert_eq!(
        line.play(step((3, 3), (0, 3))).unwrap(),
        GameStatus::Ongoing
    );
    assert_eq!(
        line.play(step((0, 0), (1, 0))).unwrap(),
        GameStatus::Ongoing
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
}

#[test]
fn winning_without_check_is_outside_the_move_set() {
    // 第22条第1項。白の最後の歩兵を金将で取れば駒枯れで勝つが、王手ではない。
    let game = game(
        Black,
        &[
            (0, 0, Black, King),
            (4, 4, Black, GoldGeneral),
            (11, 11, White, King),
            (4, 5, White, Pawn),
        ],
    );
    let mut played = game.clone();
    assert_eq!(
        played.play(step((4, 4), (4, 5))).unwrap(),
        win(Black, WinReason::PieceExhaustion)
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
}

#[test]
fn two_defender_royals_are_not_examined() {
    let game = game(
        Black,
        &[
            (0, 0, Black, King),
            (10, 10, White, King),
            (11, 11, White, CrownPrince),
        ],
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::TwoRoyals);
}

#[test]
fn promotion_to_a_second_royal_is_a_defense() {
    // 黒奔王(1,1)と黒王(2,2)で白王の全移動先を押さえる。
    // 白醉象が王手を放置して(10,3)で成れば、王将を取られても太子が残る。
    // 成り以外のm2にはm3で王将を実際に取れるので、3手目の詰み回避とは区別できる。
    let game = game(
        Black,
        &[
            (2, 2, Black, King),
            (1, 2, Black, FreeKing),
            (0, 0, White, King),
            (11, 5, White, Pawn),
            (10, 4, White, DrunkElephant),
        ],
    );
    let mut line = game.clone();
    assert_eq!(
        line.play(step((1, 2), (1, 1))).unwrap(),
        GameStatus::Ongoing
    );
    let promotion = Move {
        promote: true,
        ..step((10, 4), (10, 3))
    };
    assert_eq!(line.play(promotion).unwrap(), GameStatus::Ongoing);
    assert_eq!(line.position().royal_pieces(White).popcount(), 2);
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
}

#[test]
fn a_checking_sacrifice_forces_a_win_at_m3() {
    let game = game(Black, &three_ply_pieces());
    // 王手になるのは飛車の捕獲と王の接近だけで、どちらも1手では終わらない。
    // 王の接近には白王による先取りがあるため、勝つ手順は飛車の捨て駒から始まる。
    for mv in [step((3, 1), (0, 1)), step((2, 2), (1, 1))] {
        assert_eq!(
            game.clone().play(mv).unwrap(),
            GameStatus::Ongoing,
            "{mv:?}"
        );
    }
    let mut line = game.clone();
    assert_eq!(
        line.play(step((3, 1), (0, 1))).unwrap(),
        GameStatus::Ongoing
    );
    // 王手放置も合法なので(1,1)へ出る手があるが、黒王が直ちに取れる。
    let mut ignored_check = line.clone();
    assert_eq!(
        ignored_check.play(step((0, 0), (1, 1))).unwrap(),
        GameStatus::Ongoing
    );
    assert_eq!(
        ignored_check.play(step((2, 2), (1, 1))).unwrap(),
        win(Black, WinReason::RoyalCapture)
    );
    let mut pawn_reply = line.clone();
    assert_eq!(
        pawn_reply.play(step((11, 5), (11, 4))).unwrap(),
        GameStatus::Ongoing
    );
    assert_eq!(
        pawn_reply.play(step((0, 1), (0, 0))).unwrap(),
        win(Black, WinReason::RoyalCapture)
    );
    assert_eq!(
        line.play(step((0, 0), (0, 1))).unwrap(),
        GameStatus::Ongoing
    );
    assert_eq!(
        line.play(step((2, 5), (0, 5))).unwrap(),
        win(Black, WinReason::Mate)
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::Win);
}

#[test]
fn m2_can_win_by_capturing_the_attackers_royal() {
    // 黒王が(1,1)へ近づく唯一の王手には、白王が黒王を先に取る受けがある。
    let game = game(
        Black,
        &[
            (0, 0, Black, King),
            (11, 1, Black, Pawn),
            (2, 2, White, King),
            (10, 10, White, GoldGeneral),
        ],
    );
    let mut line = game.clone();
    assert_eq!(
        line.play(step((0, 0), (1, 1))).unwrap(),
        GameStatus::Ongoing
    );
    assert_eq!(
        line.play(step((2, 2), (1, 1))).unwrap(),
        win(White, WinReason::RoyalCapture)
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
}

#[test]
fn m2_draw_is_a_defense_even_when_other_replies_lose() {
    // 第22条第5〜8項。黒金の王手で捕獲の猶予が始まり、白王が金を取れば引き分ける。
    // 金を取らずに逃げると黒が駒枯れで勝つが、引き分ける応手が1つあるので強制勝ちはない。
    let game = game(
        Black,
        &[
            (11, 11, Black, King),
            (0, 2, Black, GoldGeneral),
            (0, 0, White, King),
        ],
    );
    let mut line = game.clone();
    assert_eq!(
        line.play(step((0, 2), (0, 1))).unwrap(),
        GameStatus::Ongoing
    );
    let mut declined = line.clone();
    assert_eq!(
        declined.play(step((0, 0), (1, 0))).unwrap(),
        win(Black, WinReason::PieceExhaustion)
    );
    assert_eq!(
        line.play(step((0, 0), (0, 1))).unwrap(),
        GameStatus::Finished(GameResult::Draw {
            reason: minase_core::DrawReason::PieceExhaustion
        })
    );
    assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
}

#[test]
fn repetition_draw_at_m1_or_m2_changes_the_same_boards_verdict() {
    // 白王の移動先は自分の歩兵で塞がれており、黒飛(0,3)の王手への受けは白飛の(0,2)への合駒だけ。
    // 履歴がなければ、黒飛が合駒を取って3手目に詰む。双方が王手と退避を繰り返した履歴が
    // あると、12手目に引き分ける手が受けになる。反復の発生をm1とm2の両方で確かめる。
    let attack = step((2, 3), (0, 3));
    let block = step((1, 2), (0, 2));
    let retreat = step((0, 3), (2, 3));
    let unblock = step((0, 2), (1, 2));
    for (side, rook_file, plies, cycle) in [
        (Black, 0, 10, [retreat, unblock, attack, block]),
        (White, 1, 11, [block, retreat, unblock, attack]),
    ] {
        let mut game = game(
            side,
            &[
                (11, 11, Black, King),
                (0, 3, Black, Rook),
                (0, 0, White, King),
                (1, 0, White, Pawn),
                (1, 1, White, Pawn),
                (rook_file, 2, White, Rook),
            ],
        );
        for ply in 0..plies {
            assert_eq!(game.play(cycle[ply % 4]).unwrap(), GameStatus::Ongoing);
        }
        let fresh = Game::from_position(Rules::ENGINE_DEFAULT, game.position().clone());
        let original = game.clone();
        assert_eq!(mate_in_three(&fresh).unwrap(), Verdict::Win);
        assert_eq!(mate_in_three(&game).unwrap(), Verdict::NoWin);
        assert!(game.same_state(&original));
        let draw = GameStatus::Finished(GameResult::Draw {
            reason: minase_core::DrawReason::Repetition,
        });
        if side == Black {
            assert_eq!(game.play(attack).unwrap(), GameStatus::Ongoing);
            assert_eq!(game.play(block).unwrap(), draw);
        } else {
            assert_eq!(game.play(attack).unwrap(), draw);
        }
    }
}

#[test]
fn finished_games_and_missing_defender_royals_are_errors() {
    let mut finished = Game::with_default_rules();
    finished.agree_draw().unwrap();
    assert!(matches!(
        mate_in_three(&finished),
        Err(Error::Game(GameError::GameAlreadyOver))
    ));
    let missing = game(Black, &[(0, 0, Black, King)]);
    assert!(matches!(
        mate_in_three(&missing),
        Err(Error::RoyalCount {
            color: White,
            count: 0
        })
    ));
}

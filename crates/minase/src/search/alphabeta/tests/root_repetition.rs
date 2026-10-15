//! 第31条R1による根の候補手の選別を、対局履歴と同期探索で検査する。
//! 升は`sq`の0始まりの内部座標で示す。

use super::*;
use minase_core::Rules;
use minase_core::game::{Game, GameResult, GameStatus, WinReason};

fn step(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn rook_cycle_position(side_to_move: Color, rook: Square) -> Position {
    position(
        side_to_move,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (rook, Color::Black, PieceKind::Rook),
            (sq(3, 9), Color::White, PieceKind::Pawn),
            (sq(5, 9), Color::White, PieceKind::Rook),
            (sq(8, 8), Color::White, PieceKind::GoldGeneral),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    )
}

fn rook_cycle() -> [Move; 4] {
    [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ]
}

fn after_plies(start: Position, cycle: &[Move], plies: usize) -> Game {
    let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start);
    for (index, &mv) in cycle.iter().cycle().take(plies).enumerate() {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "ply {}", index + 1);
    }
    game
}

fn unfiltered_snapshot(game: &Game, repetition: SearchRepetition) -> SearchSnapshot {
    SearchSnapshot::from_parts(
        game.position().clone(),
        game.rules().moves,
        repetition,
        game.repetition_key_history().to_vec(),
        game.legal_moves(),
    )
    .unwrap()
}

/// 固定深さ・1ワーカーで、置換表と探索の履歴表を毎回新しくして比較する。
fn search_snapshot(nnue: &Nnue, snapshot: &SearchSnapshot) -> SearchResult {
    let threads = worker_count(1);
    crate::search::search(
        nnue,
        snapshot,
        &depth_limits(2),
        threads,
        &mut small_tt(),
        &mut crate::search::HistoryTables::new(threads),
    )
    .unwrap()
}

#[test]
fn article_31_r1_search_avoids_the_unfiltered_repetition_losing_move() {
    // 第31条R1。黒飛の往復は白歩を攻撃し続け、白飛(5,9)が白歩(3,9)を守る。
    // 黒は金と歩の駒損なので、選別前の探索は周回を引き分けと見込んで選ぶ。
    // 黒飛の往路直後から始め、次の黒の着手が4回目の出現を完成させる。
    let cycle = rook_cycle();
    let game = after_plies(
        rook_cycle_position(Color::White, sq(3, 4)),
        &[cycle[1], cycle[2], cycle[3], cycle[0]],
        11,
    );
    let losing = cycle[0];
    assert_eq!(
        game.clone().play(losing),
        Ok(GameStatus::Finished(GameResult::Win {
            winner: Color::White,
            reason: WinReason::Repetition,
        }))
    );
    let classified = game.repetition_root_moves();
    assert!(classified.winning().is_empty());
    assert_eq!(classified.losing(), &[losing]);

    let nnue = epoch_zero_nnue();
    // 攻撃連続数を0にした対照は、周回を引き分けとして読む。
    let unfiltered = search_snapshot(
        &nnue,
        &unfiltered_snapshot(
            &game,
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
        ),
    );
    assert_eq!(unfiltered.best_move, losing);
    assert_eq!(unfiltered.score, DRAW_SCORE);

    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    let expected: Vec<_> = game
        .legal_moves()
        .into_iter()
        .filter(|&mv| mv != losing)
        .collect();
    assert_eq!(snapshot.root_moves(), expected);
    let filtered = search_snapshot(&nnue, &snapshot);
    assert_ne!(filtered.best_move, losing);
    assert!(expected.contains(&filtered.best_move));
    assert!(filtered.score < unfiltered.score);
    assert_eq!(
        game.clone().play(filtered.best_move),
        Ok(GameStatus::Ongoing)
    );
}

#[test]
fn article_31_r1_search_selects_the_repetition_winning_move() {
    // 第31条R1。白金の戻りだけが黒の連続攻撃による反復負けを確定する。
    let cycle = rook_cycle();
    let game = after_plies(rook_cycle_position(Color::Black, sq(3, 3)), &cycle, 11);
    let winning = cycle[3];
    assert_eq!(
        game.clone().play(winning),
        Ok(GameStatus::Finished(GameResult::Win {
            winner: Color::White,
            reason: WinReason::Repetition,
        }))
    );
    assert!(game.legal_moves().len() > 1);
    assert_eq!(game.repetition_root_moves().winning(), &[winning]);
    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    assert_eq!(snapshot.root_moves(), &[winning]);
    let result = search_snapshot(&weights().unwrap(), &snapshot);
    assert_eq!(result.best_move, winning);
}

#[test]
fn article_31_r1_search_keeps_all_moves_when_every_move_loses() {
    // 第31条R1。黒金は(0,11)→(1,11)→(1,10)→(0,11)で白酔象を追う。
    // 白酔象は(2,11)→(2,10)→(1,11)→(2,11)と動き、最後の手は非攻撃的になる。
    // 黒王と歩兵を動けなくし、根の合法手を黒金(0,11)→(1,11)だけにする。
    // 全手が着手直後に負ける状態は第21条の詰みで終局するため、白の応手で負ける履歴を作る。
    let start = position(
        Color::Black,
        &[
            (sq(10, 11), Color::Black, PieceKind::King),
            (sq(9, 11), Color::Black, PieceKind::Pawn),
            (sq(11, 11), Color::Black, PieceKind::Pawn),
            (sq(9, 10), Color::Black, PieceKind::Pawn),
            (sq(10, 10), Color::Black, PieceKind::Pawn),
            (sq(11, 10), Color::Black, PieceKind::Pawn),
            (sq(0, 10), Color::Black, PieceKind::Pawn),
            (sq(1, 11), Color::Black, PieceKind::GoldGeneral),
            (sq(2, 10), Color::White, PieceKind::DrunkElephant),
            (sq(11, 0), Color::White, PieceKind::King),
        ],
    );
    let losing = step(sq(0, 11), sq(1, 11));
    let reply = step(sq(2, 11), sq(2, 10));
    let cycle = [
        step(sq(1, 11), sq(1, 10)),
        step(sq(2, 10), sq(1, 11)),
        step(sq(1, 10), sq(0, 11)),
        step(sq(1, 11), sq(2, 11)),
        losing,
        reply,
    ];
    let game = after_plies(start, &cycle, 16);
    let legal = game.legal_moves();
    assert_eq!(legal, vec![losing]);
    let mut continued = game.clone();
    assert_eq!(continued.play(losing), Ok(GameStatus::Ongoing));
    assert_eq!(
        continued.play(reply),
        Ok(GameStatus::Finished(GameResult::Win {
            winner: Color::White,
            reason: WinReason::Repetition,
        }))
    );
    let classified = game.repetition_root_moves();
    assert!(classified.winning().is_empty());
    assert_eq!(classified.losing(), legal);
    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    assert_eq!(snapshot.root_moves(), legal);
    let result = search_snapshot(&weights().unwrap(), &snapshot);
    assert!(legal.contains(&result.best_move));
}

#[test]
fn article_31_r1_search_is_unchanged_without_three_occurrences() {
    // 第31条R1。初出だけの履歴と、2回出現した局面を含む履歴を両方検査する。
    let nnue = weights().unwrap();
    for plies in [0, 7] {
        let game = after_plies(
            rook_cycle_position(Color::Black, sq(3, 3)),
            &rook_cycle(),
            plies,
        );
        let history = game.repetition_key_history();
        assert!(
            history
                .iter()
                .all(|key| { history.iter().filter(|other| *other == key).count() < 3 })
        );
        let classified = game.repetition_root_moves();
        assert!(classified.winning().is_empty());
        assert!(classified.losing().is_empty());
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        assert_eq!(snapshot.root_moves(), game.legal_moves());
        let actual = search_snapshot(&nnue, &snapshot);
        let expected = search_snapshot(&nnue, &unfiltered_snapshot(&game, snapshot.repetition()));
        assert_eq!(actual.best_move, expected.best_move);
        assert_eq!(actual.score, expected.score);
        assert_eq!(actual.nodes, expected.nodes);
    }
}

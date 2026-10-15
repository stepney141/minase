//! RULES.md第31条R2・R3の反復禁止と、第23条の合法手なしの負けを検査する。

use super::*;
use crate::search::REPETITION_WIN;
use crate::search::alphabeta::searcher::MoveLoopProbe;
use minase_core::rules::parse_rule_set;
use minase_core::{Game, GameResult, GameStatus, RepetitionRule, Rules, WinReason};

fn step(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

fn rules(codes: &str) -> Rules {
    Rules::from_codes(&parse_rule_set(codes).unwrap()).unwrap()
}

/// 捕獲も成りもない金将の往復。白は金将2枚分の駒損をしている。
fn cycle_fixture(rule: RepetitionRule, prefix: bool) -> (Game, [Move; 4]) {
    let rules = Rules {
        repetition: rule,
        ..rules("L0,P0,R1,E1,E2")
    };
    let start = position(
        if prefix { Color::White } else { Color::Black },
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(3, 3), Color::Black, PieceKind::GoldGeneral),
            (sq(5, 3), Color::Black, PieceKind::GoldGeneral),
            (sq(6, 3), Color::Black, PieceKind::GoldGeneral),
            (sq(7, 3), Color::Black, PieceKind::GoldGeneral),
            (sq(2, 8), Color::White, PieceKind::GoldGeneral),
            (
                sq(8, if prefix { 9 } else { 8 }),
                Color::White,
                PieceKind::GoldGeneral,
            ),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let mut game = Game::from_position(rules, start);
    if prefix {
        play(&mut game, &[step(sq(8, 9), sq(8, 8))]);
    }
    (
        game,
        [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ],
    )
}

fn play(game: &mut Game, moves: &[Move]) {
    for &mv in moves {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing), "{mv:?}");
    }
}

fn with_snapshot(snapshot: &SearchSnapshot, nnue: &Nnue, test: impl FnOnce(&mut Searcher<'_>)) {
    with_repetition_root_searcher(
        nnue,
        snapshot.position(),
        snapshot.rules(),
        snapshot.repetition(),
        snapshot.history_keys(),
        test,
    );
}

fn probe(ply: u32, excluded: Vec<Move>) -> MoveLoopProbe {
    MoveLoopProbe {
        ply,
        excluded,
        visits: Vec::new(),
        path_keys: Vec::new(),
    }
}

/// 静止探索に捕獲がないことを確認し、禁止手を除く1手読みの値を静的評価から求める。
fn best_allowed_leaf(searcher: &Searcher<'_>, board: &Position, forbidden: Move) -> i32 {
    let mut moves = Vec::new();
    searcher.generator.generate_moves(board, &mut moves);
    moves
        .into_iter()
        .filter(|&mv| mv != forbidden)
        .map(|mv| {
            let mut child = board.clone();
            child.make_move_unchecked(mv, searcher.rules);
            let mut replies = Vec::new();
            searcher.generator.generate_moves(&child, &mut replies);
            assert!(
                replies
                    .iter()
                    .all(|&reply| child.captured_squares(reply).iter().all(Option::is_none))
            );
            -evaluate(searcher.nnue, &child)
        })
        .max()
        .unwrap()
}

#[test]
fn articles_31_r2_and_23_skip_repetitions_of_start_history_and_path() {
    let nnue = epoch_zero_nnue();
    for prefix in [false, true] {
        for history_plies in [0, 2, 3] {
            let (mut game, cycle) = cycle_fixture(RepetitionRule::R2, prefix);
            let repeated = game.position().clone();
            play(&mut game, &cycle[..history_plies]);
            let snapshot = SearchSnapshot::from_game(&game).unwrap();
            with_snapshot(&snapshot, &nnue, |searcher| {
                let mut current = snapshot.position().clone();
                for &mv in &cycle[history_plies..3] {
                    push_path_move(searcher, &mut current, Some(mv));
                }
                let ply = (3 - history_plies) as u32;
                let before = current.clone();
                assert!(searcher.move_is_forbidden(&mut current, cycle[3]));
                assert_eq!(current, before);
                let expected = best_allowed_leaf(searcher, &current, cycle[3]);
                assert!(expected < DRAW_SCORE);
                searcher.move_loop_probe = Some(probe(ply, vec![]));
                assert_eq!(
                    searcher.negamax(&mut current, 1, -INFINITY, INFINITY, ply),
                    Some(expected)
                );
                assert!(
                    !searcher
                        .move_loop_probe
                        .as_ref()
                        .unwrap()
                        .visits
                        .iter()
                        .any(|&(mv, _, _)| mv == cycle[3])
                );
                assert!(searcher.tt.probe(search_key(&repeated), ply + 1).is_none());
                assert_ne!(
                    searcher
                        .tt
                        .probe(search_key(&current), ply)
                        .unwrap()
                        .best_move,
                    Some(cycle[3])
                );
                assert_eq!(current, before);
            });
        }
    }
}

#[test]
fn articles_31_r3_and_23_count_root_once_and_allow_only_second_and_third_occurrences() {
    let nnue = epoch_zero_nnue();
    for occurrences in 1..=3 {
        for history_plies in [0, 2, 3] {
            let (mut game, cycle) = cycle_fixture(RepetitionRule::R3, false);
            for _ in 1..occurrences {
                play(&mut game, &cycle);
            }
            let root_key = repetition_key(RepetitionRule::R3, game.position());
            play(&mut game, &cycle[..history_plies]);
            let snapshot = SearchSnapshot::from_game(&game).unwrap();
            assert_eq!(
                snapshot
                    .history_keys()
                    .iter()
                    .filter(|&&key| key == root_key)
                    .count(),
                occurrences
            );
            with_snapshot(&snapshot, &nnue, |searcher| {
                let mut current = snapshot.position().clone();
                for &mv in &cycle[history_plies..3] {
                    push_path_move(searcher, &mut current, Some(mv));
                }
                let ply = (3 - history_plies) as u32;
                assert_eq!(
                    searcher.move_is_forbidden(&mut current, cycle[3]),
                    occurrences == 3
                );
                if occurrences < 3 {
                    assert_eq!(
                        searcher.search_move(
                            &mut current,
                            cycle[3],
                            1,
                            -INFINITY,
                            INFINITY,
                            ply,
                            true,
                            0
                        ),
                        Some(DRAW_SCORE)
                    );
                    assert!(
                        searcher
                            .path_moves
                            .iter()
                            .all(|entry| entry.attacking.is_none())
                    );
                } else {
                    let expected = best_allowed_leaf(searcher, &current, cycle[3]);
                    assert!(expected < DRAW_SCORE);
                    searcher.move_loop_probe = Some(probe(ply, vec![]));
                    assert_eq!(
                        searcher.negamax(&mut current, 1, -INFINITY, INFINITY, ply),
                        Some(expected)
                    );
                    assert!(
                        !searcher
                            .move_loop_probe
                            .as_ref()
                            .unwrap()
                            .visits
                            .iter()
                            .any(|&(mv, _, _)| mv == cycle[3])
                    );
                }
            });
        }
    }
}

#[test]
fn articles_31_r3_and_23_add_occurrences_across_history_and_path() {
    let nnue = epoch_zero_nnue();
    for history_cycles in 0..=2 {
        let (mut game, cycle) = cycle_fixture(RepetitionRule::R3, false);
        for _ in 0..history_cycles {
            play(&mut game, &cycle);
        }
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        with_snapshot(&snapshot, &nnue, |searcher| {
            let mut current = snapshot.position().clone();
            for index in 0..(2 - history_cycles) * 4 + 3 {
                let mv = cycle[index % 4];
                assert!(!searcher.move_is_forbidden(&mut current, mv));
                push_path_move(searcher, &mut current, Some(mv));
            }
            assert!(searcher.move_is_forbidden(&mut current, cycle[3]));
        });
    }
}

fn search_snapshot(snapshot: &SearchSnapshot, nnue: &Nnue, depth: u32) -> SearchResult {
    let threads = worker_count(1);
    crate::search::search(
        nnue,
        snapshot,
        &depth_limits(depth),
        threads,
        &mut small_tt(),
        &mut crate::search::HistoryTables::new(threads),
    )
    .unwrap()
}

#[test]
fn articles_31_r2_and_23_do_not_mix_the_forbidden_draw_into_the_best_legal_score() {
    let nnue = epoch_zero_nnue();
    let mut results = Vec::new();
    for rule in [RepetitionRule::R1, RepetitionRule::R2] {
        let (mut game, cycle) = cycle_fixture(rule, false);
        play(&mut game, &cycle[..3]);
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        let result = search_snapshot(&snapshot, &nnue, 1);
        with_snapshot(&snapshot, &nnue, |searcher| {
            let mut current = snapshot.position().clone();
            searcher
                .tt
                .store(search_key(&current), 0, 0, Bound::Upper, Some(cycle[3]), 0);
            let expected = if rule == RepetitionRule::R1 {
                DRAW_SCORE
            } else {
                best_allowed_leaf(searcher, &current, cycle[3])
            };
            assert_eq!(
                searcher.negamax(&mut current, 1, -INFINITY, INFINITY, 0),
                Some(expected)
            );
            assert_eq!(result.score, expected);
            if rule == RepetitionRule::R1 {
                assert_eq!(searcher.pv[0][0], cycle[3]);
                assert_eq!(result.best_move, cycle[3]);
            } else {
                assert!(expected < DRAW_SCORE);
                assert_ne!(searcher.pv[0][0], cycle[3]);
                assert_ne!(result.best_move, cycle[3]);
                assert!(game.legal_moves().contains(&result.best_move));
            }
        });
        results.push(result.score);
    }
    assert!(results[1] < results[0]);
}

/// 黒の王と歩を塞ぎ、仲人の戻りだけを残す。白王が戻るとその1手が禁止される。
fn trapped_fixture(rule: RepetitionRule) -> (Game, [Move; 4]) {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::GoBetween),
        (sq(11, 11), Color::Black, PieceKind::King),
        (sq(10, 10), Color::Black, PieceKind::Pawn),
        (sq(10, 11), Color::Black, PieceKind::Pawn),
        (sq(11, 10), Color::Black, PieceKind::Pawn),
        (sq(5, 5), Color::White, PieceKind::King),
    ];
    pieces.extend((2..=11).map(|rank| (sq(0, rank), Color::Black, PieceKind::Pawn)));
    let rules = Rules {
        repetition: rule,
        ..rules("L0,P0,R2,E1,E2")
    };
    (
        Game::from_position(rules, position(Color::White, &pieces)),
        [
            step(sq(5, 5), sq(5, 4)),
            step(sq(0, 0), sq(0, 1)),
            step(sq(5, 4), sq(5, 5)),
            step(sq(0, 1), sq(0, 0)),
        ],
    )
}

#[test]
fn articles_31_r2_r3_and_23_all_forbidden_moves_lose_in_the_repetition_band() {
    check_no_generated_moves_keep_the_mate_score();
    let nnue = epoch_zero_nnue();
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let (mut game, cycle) = trapped_fixture(rule);
        if rule == RepetitionRule::R3 {
            play(&mut game, &cycle);
            play(&mut game, &cycle);
        }
        play(&mut game, &cycle[..2]);
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        assert_eq!(
            game.clone().play(cycle[2]),
            Ok(GameStatus::Finished(GameResult::Win {
                winner: Color::White,
                reason: WinReason::Stalemate
            }))
        );
        let result = search_snapshot(&snapshot, &nnue, 2);
        assert_eq!(result.best_move, cycle[2]);
        // R3は合法な再現で先に引き分けと近似するため、再現後の選択肢の減少までは読まない。
        let expected = if rule == RepetitionRule::R2 {
            REPETITION_WIN - 1
        } else {
            DRAW_SCORE
        };
        assert_eq!(result.score, expected);
        with_snapshot(&snapshot, &nnue, |searcher| {
            let mut current = snapshot.position().clone();
            assert_eq!(
                searcher.search_move(&mut current, cycle[2], 2, -INFINITY, INFINITY, 0, true, 0),
                Some(expected)
            );
            push_path_move(searcher, &mut current, Some(cycle[2]));
            assert_eq!(legal_moves(&current), vec![cycle[3]]);
            let nodes = searcher.nodes;
            assert_eq!(
                searcher.negamax(&mut current, 1, -INFINITY, INFINITY, 1),
                Some(-(REPETITION_WIN - 1))
            );
            assert_eq!(searcher.nodes, nodes);
            assert!(searcher.tt.probe(search_key(&current), 1).is_none());
        });
    }
}

fn check_no_generated_moves_keep_the_mate_score() {
    let game = Game::from_position(
        rules("L0,P0,R2,E1,E2"),
        position(
            Color::White,
            &[
                (sq(0, 11), Color::Black, PieceKind::King),
                (sq(1, 11), Color::Black, PieceKind::Pawn),
                (sq(0, 10), Color::Black, PieceKind::Lance),
                (sq(1, 10), Color::Black, PieceKind::Lance),
                (sq(10, 0), Color::White, PieceKind::King),
            ],
        ),
    );
    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    with_snapshot(&snapshot, &epoch_zero_nnue(), |searcher| {
        let mut current = snapshot.position().clone();
        push_path_move(searcher, &mut current, Some(step(sq(10, 0), sq(10, 1))));
        assert!(legal_moves(&current).is_empty());
        assert_eq!(
            searcher.negamax(&mut current, 1, -INFINITY, INFINITY, 1),
            Some(-MATE + 1)
        );
        assert!(searcher.tt.probe(search_key(&current), 1).is_none());
    });
}

#[test]
fn articles_31_r2_and_23_forbidden_tt_killer_and_quiet_moves_do_not_consume_ranks_or_nodes() {
    let nnue = epoch_zero_nnue();
    let (mut game, cycle) = cycle_fixture(RepetitionRule::R2, false);
    play(&mut game, &cycle[..3]);
    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    let forbidden = cycle[3];
    for source in ["tt", "killer", "quiet"] {
        for mode in ["lmp", "lmr", "cutoff"] {
            let mut outcomes = Vec::new();
            for reference in [false, true] {
                let repetition = if reference {
                    SearchRepetition::R1 {
                        consecutive_attacking_moves: [0; 2],
                    }
                } else {
                    snapshot.repetition()
                };
                let history = if reference {
                    &[][..]
                } else {
                    snapshot.history_keys()
                };
                with_repetition_root_searcher(
                    &nnue,
                    snapshot.position(),
                    snapshot.rules(),
                    repetition,
                    history,
                    |searcher| {
                        let mut current = snapshot.position().clone();
                        searcher.move_loop_probe =
                            Some(probe(0, if reference { vec![forbidden] } else { vec![] }));
                        // 両者に同じ候補順を与え、対照だけは候補列から禁止手を取り除く。
                        searcher.history[current.side_to_move().index()]
                            [forbidden.from.dense_index()][forbidden.to.dense_index()] = 1000;
                        if source == "tt" {
                            searcher.tt.store(
                                search_key(&current),
                                0,
                                0,
                                Bound::Upper,
                                Some(forbidden),
                                0,
                            );
                        } else if source == "killer" {
                            searcher.killers[0][0] = Some(forbidden);
                        }
                        let static_eval = evaluate(&nnue, &current);
                        let (depth, alpha, beta, child_score) = match mode {
                            "lmp" => (2, static_eval + 1, static_eval + 2, -(static_eval + 1)),
                            "lmr" => (5, -INFINITY, INFINITY, -static_eval),
                            "cutoff" => (2, -INFINITY, 100, -200),
                            _ => unreachable!(),
                        };
                        // 子の値を固定し、順位以外の深い反復や静止探索の差を比較から除く。
                        for mv in legal_moves(&current)
                            .into_iter()
                            .filter(|&mv| mv != forbidden)
                        {
                            let mut child = current.clone();
                            child.make_move_unchecked(mv, searcher.rules);
                            searcher.tt.store(
                                search_key(&child),
                                10,
                                child_score,
                                Bound::Exact,
                                None,
                                1,
                            );
                        }
                        let score = searcher
                            .negamax(&mut current, depth, alpha, beta, 0)
                            .unwrap();
                        let visits = searcher.move_loop_probe.as_ref().unwrap().visits.clone();
                        assert!(!visits.is_empty());
                        assert_eq!(visits[0].1, 0);
                        assert!(!visits.iter().any(|&(mv, _, _)| mv == forbidden));
                        assert_eq!(
                            searcher.history[current.side_to_move().index()]
                                [forbidden.from.dense_index()][forbidden.to.dense_index()],
                            1000
                        );
                        if mode == "lmr" {
                            let effective_depth = if source == "tt" { depth } else { depth - 1 };
                            assert!(visits.iter().any(|&(_, index, _)| lmr_reduction(
                                effective_depth,
                                index,
                                0
                            ) > 0));
                        }
                        if mode == "lmp" {
                            assert!(searcher.nodes < visits.len() as u64);
                        }
                        if mode == "cutoff" {
                            assert_eq!(searcher.nodes, 1);
                        }
                        let histories: Vec<_> = legal_moves(&current)
                            .into_iter()
                            .map(|mv| {
                                searcher.history[current.side_to_move().index()]
                                    [mv.from.dense_index()][mv.to.dense_index()]
                            })
                            .collect();
                        outcomes.push((
                            score,
                            searcher.nodes,
                            visits,
                            histories,
                            searcher.killers[0],
                        ));
                    },
                );
            }
            assert_eq!(outcomes[0], outcomes[1], "source={source}, mode={mode}");
        }
    }
}

fn p2_fixture(rule: RepetitionRule) -> (Game, Position, Move) {
    let rules = Rules {
        repetition: rule,
        ..rules("L1,L3,P2,P5,P6,R2,E1,E2")
    };
    let mut game = Game::from_position(
        rules,
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(4, 7), Color::Black, PieceKind::SilverGeneral),
                (sq(10, 10), Color::White, PieceKind::GoldGeneral),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    );
    play(&mut game, &[step(sq(4, 7), sq(4, 8))]);
    let waiting = game.position().clone();
    play(
        &mut game,
        &[
            step(sq(10, 10), sq(10, 9)),
            step(sq(4, 8), sq(5, 9)),
            step(sq(10, 9), sq(10, 10)),
        ],
    );
    (game, waiting, step(sq(5, 9), sq(4, 8)))
}

#[test]
fn articles_31_r2_r3_and_23_p2_waiting_rights_do_not_hide_a_repetition() {
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let (game, waiting, returning) = p2_fixture(rule);
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        assert!(waiting.promotion_deferred().contains(sq(4, 8)));
        assert_eq!(snapshot.history_keys()[1], waiting.zobrist());
        with_snapshot(&snapshot, &epoch_zero_nnue(), |searcher| {
            let mut current = snapshot.position().clone();
            let before = current.clone();
            assert_eq!(
                searcher.move_is_forbidden(&mut current, returning),
                rule == RepetitionRule::R2
            );
            assert_eq!(current, before);
            assert_eq!(
                game.legal_moves().contains(&returning),
                rule == RepetitionRule::R3
            );
            if rule == RepetitionRule::R3 {
                assert_eq!(
                    searcher.search_move(
                        &mut current,
                        returning,
                        1,
                        -INFINITY,
                        INFINITY,
                        0,
                        true,
                        0
                    ),
                    Some(DRAW_SCORE)
                );
            }
            current.make_move_unchecked(returning, searcher.rules);
            assert!(current.promotion_deferred().is_empty());
            assert_eq!(current.zobrist(), waiting.zobrist());
            assert_ne!(search_key(&current), search_key(&waiting));
        });
        // 待機状態を持つ根も、成り権を含めず経路の先頭へ積む。
        let root_game = Game::from_position(game.rules(), waiting.clone());
        let root_snapshot = SearchSnapshot::from_game(&root_game).unwrap();
        with_snapshot(&root_snapshot, &epoch_zero_nnue(), |searcher| {
            assert_eq!(searcher.path_keys, vec![waiting.zobrist()]);
        });
    }
}

#[test]
fn articles_31_r2_r3_and_23_p2_live_paths_use_repetition_keys_and_tt_keeps_rights() {
    let nnue = epoch_zero_nnue();
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let (game, waiting, _) = p2_fixture(rule);
        let root_game = Game::from_position(game.rules(), waiting.clone());
        let snapshot = SearchSnapshot::from_game(&root_game).unwrap();
        for null in [false, true] {
            with_snapshot(&snapshot, &nnue, |searcher| {
                let mut current = waiting.clone();
                let mv = step(sq(10, 10), sq(10, 9));
                let mut child = current.clone();
                if null {
                    child.make_null_move();
                } else {
                    child.make_move_unchecked(mv, searcher.rules);
                }
                assert_ne!(search_key(&child), child.zobrist());
                searcher.move_loop_probe = Some(probe(1, vec![]));
                if null {
                    assert!(
                        searcher
                            .search_null_move(&mut current, 1, INFINITY, 0)
                            .is_some()
                    );
                } else {
                    assert!(
                        searcher
                            .search_move(&mut current, mv, 2, -INFINITY, INFINITY, 0, true, 0)
                            .is_some()
                    );
                }
                let observed = searcher.move_loop_probe.as_ref().unwrap();
                assert!(!observed.visits.is_empty());
                assert_eq!(observed.path_keys, vec![waiting.zobrist(), child.zobrist()]);
                assert!(searcher.tt.probe(search_key(&child), 1).is_some());
                assert!(searcher.tt.probe(child.zobrist(), 1).is_none());
                assert_eq!(current, waiting);
                assert_eq!(searcher.path_keys, vec![waiting.zobrist()]);
                assert!(searcher.path_moves.is_empty());
                assert_eq!(searcher.null_move_boundary, None);
            });
        }
    }
}

#[test]
fn articles_31_r2_r3_and_23_search_prohibitions_equal_referee_prohibitions() {
    let nnue = epoch_zero_nnue();
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let (mut game, cycle) = cycle_fixture(rule, true);
        for occurrence in 1..=if rule == RepetitionRule::R3 { 3 } else { 1 } {
            play(&mut game, &cycle[..3]);
            let snapshot = SearchSnapshot::from_game(&game).unwrap();
            with_snapshot(&snapshot, &nnue, |searcher| {
                let mut current = snapshot.position().clone();
                let generated = legal_moves(&current);
                let allowed = game.legal_moves();
                let expected: Vec<_> = generated
                    .iter()
                    .copied()
                    .filter(|mv| !allowed.contains(mv))
                    .collect();
                let actual: Vec<_> = generated
                    .into_iter()
                    .filter(|&mv| searcher.move_is_forbidden(&mut current, mv))
                    .collect();
                assert_eq!(actual, expected);
                if rule == RepetitionRule::R2 || occurrence == 3 {
                    assert_eq!(actual, vec![cycle[3]]);
                } else {
                    assert!(actual.is_empty());
                }
            });
            if rule == RepetitionRule::R3 && occurrence < 3 {
                play(&mut game, &cycle[3..]);
            }
        }
    }
}

#[test]
fn articles_31_r2_r3_and_23_null_move_counts_only_the_latest_subtree() {
    let nnue = epoch_zero_nnue();
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let (mut game, cycle) = cycle_fixture(rule, false);
        if rule == RepetitionRule::R3 {
            play(&mut game, &cycle);
            play(&mut game, &cycle);
        }
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        with_snapshot(&snapshot, &nnue, |searcher| {
            let mut current = snapshot.position().clone();
            for &mv in &cycle[..3] {
                push_path_move(searcher, &mut current, Some(mv));
            }
            assert!(searcher.move_is_forbidden(&mut current, cycle[3]));
            // 2回の仮想手番交代で盤面と手番を保ち、最後の境界より前の履歴を照合から外す。
            push_path_move(searcher, &mut current, None);
            push_path_move(searcher, &mut current, None);
            assert!(!searcher.move_is_forbidden(&mut current, cycle[3]));
            push_path_move(searcher, &mut current, Some(cycle[3]));
            for occurrence in 1..=if rule == RepetitionRule::R3 { 3 } else { 1 } {
                for &mv in &cycle[..3] {
                    push_path_move(searcher, &mut current, Some(mv));
                }
                let forbidden = rule == RepetitionRule::R2 || occurrence == 3;
                assert_eq!(
                    searcher.move_is_forbidden(&mut current, cycle[3]),
                    forbidden
                );
                if !forbidden {
                    push_path_move(searcher, &mut current, Some(cycle[3]));
                }
            }
        });
    }
}

#[test]
fn articles_31_r1_r2_and_23_keep_r1_keys_and_history_equal_to_search_keys() {
    let (mut game, cycle) = cycle_fixture(RepetitionRule::R1, false);
    let mut keys = vec![search_key(game.position())];
    for mv in cycle {
        play(&mut game, &[mv]);
        assert_eq!(
            repetition_key(RepetitionRule::R1, game.position()),
            search_key(game.position())
        );
        keys.push(search_key(game.position()));
    }
    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    assert_eq!(snapshot.history_keys(), keys);
    let (game, waiting, _) = p2_fixture(RepetitionRule::R1);
    let snapshot = SearchSnapshot::from_game(&game).unwrap();
    assert_eq!(
        repetition_key(RepetitionRule::R1, &waiting),
        search_key(&waiting)
    );
    assert_eq!(snapshot.history_keys()[1], search_key(&waiting));
    assert_ne!(snapshot.history_keys()[1], waiting.zobrist());
    with_snapshot(&snapshot, &epoch_zero_nnue(), |searcher| {
        // R1は適用不能な手であっても局面を変更せず、禁止の判定だけを直ちに終える。
        let mut current = Position::empty(Color::Black);
        let before = current.clone();
        assert!(!searcher.move_is_forbidden(&mut current, step(sq(0, 0), sq(0, 1))));
        assert_eq!(current, before);
        assert_eq!(searcher.path_keys, vec![search_key(snapshot.position())]);
    });
}

//! 第31条R1の周回の攻撃側と、探索経路・対局履歴・null moveの区間を検査する。

use super::*;
use crate::search::{REPETITION_WIN, in_repetition_band};
use minase_core::{Game, GameStatus, RepetitionRule, Rules};

fn step(from: Square, to: Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

/// 黒飛は3筋の白歩、白飛は8筋の黒歩を、それぞれ往復する2升から照準する。
fn rook_cycle(black_attacks: bool, white_attacks: bool) -> (Position, [Move; 4]) {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(3, 3), Color::Black, PieceKind::Rook),
        (sq(8, 8), Color::White, PieceKind::Rook),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    if black_attacks {
        pieces.push((sq(3, 9), Color::White, PieceKind::Pawn));
    }
    if white_attacks {
        pieces.push((sq(8, 1), Color::Black, PieceKind::Pawn));
    }
    (
        position(Color::Black, &pieces),
        [
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 4), sq(3, 3)),
            step(sq(8, 7), sq(8, 8)),
        ],
    )
}

fn complete_cycle(searcher: &mut Searcher<'_>, current: &mut Position, mv: Move) -> i32 {
    let before = current.clone();
    let keys = searcher.path_keys.clone();
    let ply = searcher.path_moves.len() as u32;
    let score = searcher
        .search_move(current, mv, 1, -INFINITY, INFINITY, ply, true, 0)
        .unwrap();
    assert_eq!(*current, before);
    assert_eq!(searcher.path_keys, keys);
    assert_eq!(searcher.path_moves.len() + 1, searcher.path_keys.len());
    score
}

#[test]
fn article_31_r1_path_cycle_scores_the_sole_attackers_loss_from_either_side() {
    for attacker in Color::ALL {
        let (mut root, cycle) = rook_cycle(attacker == Color::Black, attacker == Color::White);
        for offset in 0..4 {
            with_root_searcher(&root, &[search_key(&root)], |searcher| {
                let mut current = root.clone();
                for index in 0..3 {
                    push_path_move(searcher, &mut current, Some(cycle[(offset + index) % 4]));
                }
                assert!(
                    searcher
                        .path_moves
                        .iter()
                        .all(|entry| entry.attacking.is_none())
                );
                let mover = current.side_to_move();
                let expected = if mover == attacker {
                    -(REPETITION_WIN - 4)
                } else {
                    REPETITION_WIN - 4
                };
                assert_eq!(
                    complete_cycle(searcher, &mut current, cycle[(offset + 3) % 4]),
                    expected
                );
                assert!(
                    searcher
                        .path_moves
                        .iter()
                        .all(|entry| entry.attacking.is_some())
                );
                assert!(searcher.tt.probe(search_key(&root), 4).is_none());
            });
            root.make_move_unchecked(cycle[offset], engine_rules());
        }
    }
}

#[test]
fn article_31_r1_mutual_attacks_draw_and_cache_the_path() {
    let (root, cycle) = rook_cycle(true, true);
    with_root_searcher(&root, &[], |searcher| {
        let mut current = root.clone();
        for &mv in &cycle[..3] {
            push_path_move(searcher, &mut current, Some(mv));
        }
        assert_eq!(complete_cycle(searcher, &mut current, cycle[3]), DRAW_SCORE);
        assert!(
            searcher
                .path_moves
                .iter()
                .all(|entry| entry.attacking == Some(true))
        );
        // 記録済みの判定を使う契約を検査するため、黒の判定だけを非攻撃へ差し替える。
        searcher.path_moves[0].attacking = Some(false);
        assert_eq!(
            complete_cycle(searcher, &mut current, cycle[3]),
            -(REPETITION_WIN - 4)
        );
    });
}

#[test]
fn article_31_r1_quiet_cycles_stop_classifying_after_both_sides_are_quiet() {
    let (root, cycle) = rook_cycle(false, false);
    with_root_searcher(&root, &[], |searcher| {
        let mut current = root.clone();
        for &mv in &cycle[..3] {
            push_path_move(searcher, &mut current, Some(mv));
        }
        assert_eq!(complete_cycle(searcher, &mut current, cycle[3]), DRAW_SCORE);
        let cached: Vec<_> = searcher
            .path_moves
            .iter()
            .map(|entry| entry.attacking)
            .collect();
        assert_eq!(cached, [Some(false), Some(false), None]);
    });
}

#[test]
fn article_31_r1_history_attack_counts_cover_exactly_the_history_part() {
    for attacker in Color::ALL {
        let (start, cycle) = rook_cycle(attacker == Color::Black, attacker == Color::White);
        for plies in [2, 3] {
            let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start.clone());
            for &mv in &cycle[..plies] {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
            let snapshot = SearchSnapshot::from_game(&game).unwrap();
            let counts = game.consecutive_attacking_moves().unwrap();
            assert_eq!(
                snapshot.repetition(),
                SearchRepetition::R1 {
                    consecutive_attacking_moves: counts
                }
            );
            for enough in [true, false] {
                let mut counters = counts;
                if !enough {
                    counters[attacker.index()] -= 1;
                }
                with_root_searcher(game.position(), game.search_key_history(), |searcher| {
                    searcher.repetition = SearchRepetition::R1 {
                        consecutive_attacking_moves: counters,
                    };
                    let mut current = game.position().clone();
                    for &mv in &cycle[plies..3] {
                        push_path_move(searcher, &mut current, Some(mv));
                    }
                    assert_eq!(
                        searcher.find_repetition(search_key(&start)),
                        Some(RepetitionMatch::History(0))
                    );
                    let distance = (4 - plies) as i32;
                    let expected = if !enough {
                        DRAW_SCORE
                    } else if attacker == Color::White {
                        -(REPETITION_WIN - distance)
                    } else {
                        REPETITION_WIN - distance
                    };
                    assert_eq!(complete_cycle(searcher, &mut current, cycle[3]), expected);
                });
            }
        }
    }
}

#[test]
fn article_31_r1_nearest_occurrence_excludes_an_earlier_quiet_move() {
    let (root, cycle) = rook_cycle(true, false);
    let detour = [
        step(sq(3, 3), sq(4, 3)),
        cycle[1],
        step(sq(4, 3), sq(3, 3)),
        cycle[3],
    ];
    for in_history in [false, true] {
        let mut game = Game::from_position(Rules::ENGINE_DEFAULT, root.clone());
        if in_history {
            for &mv in detour.iter().chain(&cycle[..3]) {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
        }
        with_root_searcher(game.position(), game.search_key_history(), |searcher| {
            searcher.repetition = SearchSnapshot::from_game(&game).unwrap().repetition();
            let mut current = game.position().clone();
            if !in_history {
                for &mv in detour.iter().chain(&cycle[..3]) {
                    push_path_move(searcher, &mut current, Some(mv));
                }
            }
            let expected_match = if in_history {
                RepetitionMatch::History(4)
            } else {
                RepetitionMatch::Path(4)
            };
            assert_eq!(
                searcher.find_repetition(search_key(&root)),
                Some(expected_match)
            );
            let distance = if in_history { 1 } else { 8 };
            assert_eq!(
                complete_cycle(searcher, &mut current, cycle[3]),
                REPETITION_WIN - distance
            );
            if !in_history {
                assert!(
                    searcher.path_moves[..4]
                        .iter()
                        .all(|entry| entry.attacking.is_none())
                );
            }
        });
    }
}

/// 黒獅子のじっとは白歩を照準し、白獅子のじっとには捕獲の脅威がない。
fn lion_cycle(side: Color, quiet_black_lion: bool) -> Position {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(3, 3), Color::Black, PieceKind::Lion),
        (sq(3, 5), Color::White, PieceKind::Pawn),
        (sq(8, 8), Color::White, PieceKind::Lion),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    if quiet_black_lion {
        pieces.push((sq(6, 1), Color::Black, PieceKind::Lion));
    }
    position(side, &pieces)
}

#[test]
fn article_31_r1_search_can_switch_to_a_quiet_move_inside_the_tree() {
    for has_quiet_move in [false, true] {
        let root = lion_cycle(Color::White, has_quiet_move);
        let root_move = step(sq(8, 8), sq(8, 8));
        let attacking = step(sq(3, 3), sq(3, 3));
        let mut child = root.clone();
        child.make_move_unchecked(root_move, engine_rules());
        let mut table = small_tt();
        // 周回を外れる手を王駒捕獲の負けと確定し、黒の2種類のじっとの比較に絞る。
        // これらは深さ2以上では探索窓にかかわらず再利用できる、履歴に依存しない詰み値である。
        for mv in legal_moves(&child) {
            if mv.from != mv.to {
                let mut other = child.clone();
                other.make_move_unchecked(mv, engine_rules());
                table.store(search_key(&other), 8, MATE - 32, Bound::Exact, None, 2);
            }
        }
        table.store(search_key(&child), 0, 0, Bound::Upper, Some(attacking), 1);
        let snapshot = SearchSnapshot::from_parts(
            root.clone(),
            engine_rules(),
            SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            vec![search_key(&root)],
            vec![root_move],
        )
        .unwrap();
        let result = crate::search::search(
            &weights().unwrap(),
            &snapshot,
            &depth_limits(3),
            DEFAULT_THREADS,
            &mut table,
            &mut crate::search::HistoryTables::new(DEFAULT_THREADS),
        )
        .unwrap();
        assert_eq!(
            result.score,
            if has_quiet_move {
                DRAW_SCORE
            } else {
                REPETITION_WIN - 2
            }
        );
        let chosen = table.probe(search_key(&child), 1).unwrap().best_move;
        assert_eq!(
            chosen,
            Some(if has_quiet_move {
                step(sq(6, 1), sq(6, 1))
            } else {
                attacking
            })
        );
    }
}

#[test]
fn article_31_r1_attacking_cycle_does_not_match_before_null_move() {
    let root = lion_cycle(Color::White, false);
    with_root_searcher(&root, &[search_key(&root)], |searcher| {
        let mut current = root.clone();
        push_path_move(searcher, &mut current, None);
        let attacking = step(sq(3, 3), sq(3, 3));
        let mut returned = current.clone();
        returned.make_move_unchecked(attacking, engine_rules());
        assert_eq!(search_key(&returned), search_key(&root));
        assert_eq!(searcher.find_repetition(search_key(&returned)), None);
        let score = complete_cycle(searcher, &mut current, attacking);
        assert!(!in_repetition_band(score));
        assert!(searcher.path_moves[0].attacking.is_none());
    });
}

#[test]
fn article_31_r1_cycle_to_post_null_position_starts_after_the_null_move() {
    let (root, cycle) = rook_cycle(true, false);
    with_root_searcher(&root, &[search_key(&root)], |searcher| {
        let mut current = root.clone();
        push_path_move(searcher, &mut current, None);
        let post_null = search_key(&current);
        for mv in [cycle[1], cycle[0], cycle[3]] {
            push_path_move(searcher, &mut current, Some(mv));
        }
        assert_eq!(
            searcher.find_repetition(post_null),
            Some(RepetitionMatch::Path(1))
        );
        assert_eq!(
            complete_cycle(searcher, &mut current, cycle[2]),
            -(REPETITION_WIN - 5)
        );
        assert!(searcher.path_moves[0].attacking.is_none());
        assert!(
            searcher.path_moves[1..]
                .iter()
                .all(|entry| entry.attacking.is_some())
        );
    });
}

#[test]
fn article_31_r1_replay_includes_null_moves_before_the_cycle() {
    let (root, cycle) = rook_cycle(true, false);
    with_root_searcher(&root, &[], |searcher| {
        let mut current = root.clone();
        push_path_move(searcher, &mut current, None);
        for mv in [cycle[1], cycle[0]] {
            push_path_move(searcher, &mut current, Some(mv));
        }
        let cycle_start = search_key(&current);
        for mv in [
            step(sq(8, 7), sq(8, 6)),
            step(sq(3, 4), sq(3, 5)),
            step(sq(8, 6), sq(8, 7)),
        ] {
            push_path_move(searcher, &mut current, Some(mv));
        }
        assert_eq!(
            searcher.find_repetition(cycle_start),
            Some(RepetitionMatch::Path(3))
        );
        assert_eq!(
            complete_cycle(searcher, &mut current, step(sq(3, 5), sq(3, 4))),
            -(REPETITION_WIN - 7)
        );
        assert!(
            searcher.path_moves[..3]
                .iter()
                .all(|entry| entry.attacking.is_none())
        );
        assert_eq!(searcher.path_moves[4].attacking, Some(true));
        assert_eq!(searcher.path_moves[3].attacking, Some(false));
    });
}

#[test]
fn article_31_r1_current_quiet_move_breaks_the_attack_sequence() {
    let root = position(
        Color::White,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(3, 3), Color::Black, PieceKind::Rook),
            (sq(5, 4), Color::White, PieceKind::Pawn),
            (sq(8, 8), Color::White, PieceKind::GoldGeneral),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    with_root_searcher(&root, &[], |searcher| {
        let mut current = root.clone();
        for mv in [
            step(sq(8, 8), sq(8, 7)),
            step(sq(3, 3), sq(3, 4)),
            step(sq(8, 7), sq(8, 8)),
        ] {
            push_path_move(searcher, &mut current, Some(mv));
        }
        assert_eq!(
            complete_cycle(searcher, &mut current, step(sq(3, 4), sq(3, 3))),
            DRAW_SCORE
        );
        assert_eq!(searcher.path_moves[1].attacking, Some(true));
    });
}

#[test]
fn article_31_r1_two_workers_score_the_path_cycle_ten_times() {
    let root = lion_cycle(Color::Black, false);
    let attack = step(sq(3, 3), sq(3, 3));
    let snapshot = SearchSnapshot::from_parts(
        root.clone(),
        engine_rules(),
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        vec![search_key(&root)],
        vec![attack],
    )
    .unwrap();
    let nnue = weights().unwrap();
    for attempt in 0..10 {
        let threads = worker_count(2);
        let result = crate::search::search(
            &nnue,
            &snapshot,
            &depth_limits(2),
            threads,
            &mut small_tt(),
            &mut crate::search::HistoryTables::new(threads),
        )
        .unwrap();
        assert_eq!(result.best_move, attack, "attempt {attempt}");
        assert_eq!(result.score, -(REPETITION_WIN - 2), "attempt {attempt}");
        assert_eq!(result.depth, 2);
    }
}

#[test]
fn article_31_r1_path_stacks_restore_after_moves_null_moves_and_interruptions() {
    let (root, cycle) = rook_cycle(true, false);
    for null in [false, true] {
        for interrupt in [None, Some(0), Some(1), Some(2)] {
            with_root_searcher(&root, &[], |searcher| {
                let mut current = root.clone();
                searcher.interrupt_at = interrupt.map(|node| (node, StopReason::ExternalStop));
                let score = if null {
                    let beta = evaluate(searcher.nnue, &root);
                    searcher.search_null_move(&mut current, 2, beta, 0)
                } else {
                    searcher.search_move(&mut current, cycle[0], 3, -INFINITY, INFINITY, 0, true, 0)
                };
                assert_eq!(
                    score.is_none(),
                    interrupt.is_some(),
                    "null={null}, interrupt={interrupt:?}"
                );
                assert_eq!(current, root);
                assert_eq!(searcher.path_keys, [search_key(&root)]);
                assert!(searcher.path_moves.is_empty());
                assert_eq!(searcher.null_move_boundary, None);
                assert_eq!(searcher.null_move_ply, None);
            });
        }
    }
}

#[test]
fn article_31_r1_snapshot_selects_the_rule_and_r2_r3_keep_draws() {
    let (root, cycle) = rook_cycle(true, false);
    for rule in [RepetitionRule::R1, RepetitionRule::R2, RepetitionRule::R3] {
        let rules = Rules {
            repetition: rule,
            ..Rules::ENGINE_DEFAULT
        };
        let game = Game::from_position(rules, root.clone());
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        let expected = match rule {
            RepetitionRule::R1 => SearchRepetition::R1 {
                consecutive_attacking_moves: [0; 2],
            },
            RepetitionRule::R2 => SearchRepetition::R2,
            RepetitionRule::R3 => SearchRepetition::R3,
        };
        assert_eq!(snapshot.rules(), rules.moves);
        assert_eq!(snapshot.repetition(), expected);
        with_root_searcher(&root, &[], |searcher| {
            searcher.repetition = snapshot.repetition();
            let mut current = root.clone();
            for &mv in &cycle[..3] {
                push_path_move(searcher, &mut current, Some(mv));
            }
            let score = complete_cycle(searcher, &mut current, cycle[3]);
            assert_eq!(
                score,
                if rule == RepetitionRule::R1 {
                    REPETITION_WIN - 4
                } else {
                    DRAW_SCORE
                }
            );
        });
    }
}

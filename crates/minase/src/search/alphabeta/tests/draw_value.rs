//! RULES.md第31条R1の引き分けの値と、R2・R3への非適用を検査する。

use super::*;
use crate::search::{REPETITION_WIN, in_repetition_band};
use minase_core::{Game, GameStatus, RepetitionRule, Rules};

#[test]
fn article_31_r1_draw_value_defaults_and_validation() {
    let game = Game::new(Rules::ENGINE_DEFAULT);
    for (source, snapshot) in [
        (
            "article_31_r1_game",
            SearchSnapshot::from_game(&game).unwrap(),
        ),
        ("article_31_r1_position", snapshot_for(&Position::initial())),
    ] {
        for (value, valid, default) in [
            (-28_999, true, DRAW_SCORE),
            (-50, true, DRAW_SCORE),
            (0, true, DRAW_SCORE),
            (50, true, DRAW_SCORE),
            (28_999, true, DRAW_SCORE),
            (i32::MIN, false, DRAW_SCORE),
            (-29_000, false, DRAW_SCORE),
            (29_000, false, DRAW_SCORE),
            (i32::MAX, false, DRAW_SCORE),
        ] {
            assert_eq!(snapshot.draw_value(), default, "{source}, value={value}");
            let result = snapshot.clone().with_draw_value(value);
            if valid {
                assert_eq!(
                    result.unwrap().draw_value(),
                    value,
                    "{source}, value={value}"
                );
            } else {
                assert!(
                    matches!(result, Err(SearchError::InvalidDrawValue { value: rejected }) if rejected == value),
                    "{source}, value={value}"
                );
            }
        }
    }
}

fn play(game: &mut Game, moves: &[Move]) {
    for &mv in moves {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }
}

fn with_snapshot(snapshot: &SearchSnapshot, nnue: &Nnue, test: impl FnOnce(&mut Searcher<'_>)) {
    with_draw_value_root_searcher(
        nnue,
        snapshot.position(),
        snapshot.rules(),
        snapshot.repetition(),
        snapshot.draw_value(),
        snapshot.history_keys(),
        test,
    );
}

/// 根の開始位置をずらし、先手番と後手番の両方で同じ周回を作る。
fn rotated_cycle(black_attacks: bool, white_attacks: bool, offset: usize) -> (Position, [Move; 4]) {
    let (mut root, mut cycle) = rook_cycle(black_attacks, white_attacks);
    for &mv in &cycle[..offset] {
        root.make_move_unchecked(mv, engine_rules());
    }
    cycle.rotate_left(offset);
    (root, cycle)
}

#[test]
fn article_31_r1_draw_value_signs_path_cycles_from_both_root_colors() {
    let nnue = epoch_zero_nnue();
    for attacks in [false, true] {
        for offset in [0, 1] {
            let (root, cycle) = rotated_cycle(attacks, attacks, offset);
            for prefix in [0, 1] {
                for draw_value in [-50, 0] {
                    let snapshot = snapshot_for(&root).with_draw_value(draw_value).unwrap();
                    with_snapshot(&snapshot, &nnue, |searcher| {
                        let mut current = root.clone();
                        for index in 0..prefix + 3 {
                            push_path_move(searcher, &mut current, Some(cycle[index % 4]));
                        }
                        let mv = cycle[(prefix + 3) % 4];
                        let mut returned = current.clone();
                        returned.make_move_unchecked(mv, engine_rules());
                        assert_eq!(
                            searcher.find_repetition(search_key(&returned)),
                            Some(RepetitionMatch::Path(prefix))
                        );
                        // 4手目は相手、5手目は根と同じ側の着手になる。
                        let expected = if prefix == 0 { -draw_value } else { draw_value };
                        assert_eq!(complete_cycle(searcher, &mut current, mv), expected);
                    });
                }
            }
        }
    }
}

#[test]
fn article_31_r1_draw_value_signs_history_cycles_from_both_root_colors() {
    let nnue = epoch_zero_nnue();
    for attacks in [false, true] {
        for offset in [0, 1] {
            let (start, cycle) = rotated_cycle(attacks, attacks, offset);
            for history_plies in [2, 3] {
                let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start.clone());
                play(&mut game, &cycle[..history_plies]);
                for draw_value in [-50, 0] {
                    let snapshot = SearchSnapshot::from_game(&game)
                        .unwrap()
                        .with_draw_value(draw_value)
                        .unwrap();
                    with_snapshot(&snapshot, &nnue, |searcher| {
                        let mut current = game.position().clone();
                        for &mv in &cycle[history_plies..3] {
                            push_path_move(searcher, &mut current, Some(mv));
                        }
                        assert_eq!(
                            searcher.find_repetition(search_key(&start)),
                            Some(RepetitionMatch::History(0))
                        );
                        // 履歴が3手なら根自身の手、2手なら相手の応手で周回を閉じる。
                        let expected = if history_plies == 3 {
                            draw_value
                        } else {
                            -draw_value
                        };
                        assert_eq!(complete_cycle(searcher, &mut current, cycle[3]), expected);
                    });
                }
            }
        }
    }
}

#[test]
fn article_31_r1_draw_value_keeps_repetition_win_and_loss_in_their_band() {
    let nnue = epoch_zero_nnue();
    for attacker in Color::ALL {
        for offset in [0, 1] {
            let (start, cycle) =
                rotated_cycle(attacker == Color::Black, attacker == Color::White, offset);
            for history_plies in [0, 2, 3] {
                let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start.clone());
                play(&mut game, &cycle[..history_plies]);
                for draw_value in [-50, 0, 50] {
                    let snapshot = SearchSnapshot::from_game(&game)
                        .unwrap()
                        .with_draw_value(draw_value)
                        .unwrap();
                    with_snapshot(&snapshot, &nnue, |searcher| {
                        let mut current = game.position().clone();
                        for &mv in &cycle[history_plies..3] {
                            push_path_move(searcher, &mut current, Some(mv));
                        }
                        let distance = (4 - history_plies) as i32;
                        let expected = if current.side_to_move() == attacker {
                            -(REPETITION_WIN - distance)
                        } else {
                            REPETITION_WIN - distance
                        };
                        let score = complete_cycle(searcher, &mut current, cycle[3]);
                        assert_eq!(score, expected);
                        assert!(in_repetition_band(score));
                    });
                }
            }
        }
    }
}

#[test]
fn article_31_r1_negative_draw_value_selects_a_quiet_move_over_a_history_draw() {
    let nnue = epoch_zero_nnue();
    let (root, cycle) = rook_cycle(false, false);
    let mut game = Game::from_position(Rules::ENGINE_DEFAULT, root);
    play(&mut game, &cycle[..3]);
    assert_eq!(game.consecutive_attacking_moves(), Some([0; 2]));
    let returning = cycle[3];
    let quiet = step(sq(8, 7), sq(9, 7));
    assert!(game.legal_moves().contains(&quiet));
    let snapshot = SearchSnapshot::from_parts(
        game.position().clone(),
        game.rules().moves,
        SearchRepetition::R1 {
            consecutive_attacking_moves: [0; 2],
        },
        game.repetition_key_history().to_vec(),
        vec![returning, quiet],
    )
    .unwrap();
    for draw_value in [-50, 0] {
        let input = snapshot.clone().with_draw_value(draw_value).unwrap();
        let mut table = small_tt();
        let mut child = game.position().clone();
        child.make_move_unchecked(quiet, game.rules().moves);
        // 根から見て-50 < -20 < 0。子の手番側から見た値は+20となる。
        table.store(search_key(&child), 8, 20, Bound::Exact, None, 1);
        let result = crate::search::search(
            &nnue,
            &input,
            &depth_limits(2),
            DEFAULT_THREADS,
            &mut table,
            &mut crate::search::HistoryTables::new(DEFAULT_THREADS),
        )
        .unwrap();
        let expected = if draw_value == -50 {
            (quiet, -20)
        } else {
            (returning, DRAW_SCORE)
        };
        assert_eq!((result.best_move, result.score), expected);
        assert_eq!(result.depth, 2);
    }
}

#[test]
fn article_31_r1_draw_value_does_not_change_r2_or_r3_search() {
    let nnue = epoch_zero_nnue();
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        for occurrences in 1..=if rule == RepetitionRule::R3 { 2 } else { 1 } {
            let (root, cycle) = rook_cycle(false, false);
            let mut game = Game::from_position(
                Rules {
                    repetition: rule,
                    ..Rules::ENGINE_DEFAULT
                },
                root,
            );
            for _ in 1..occurrences {
                play(&mut game, &cycle);
            }
            play(&mut game, &cycle[..3]);
            let snapshot = SearchSnapshot::from_game(&game).unwrap();
            // R2の再現は禁止手であり、R3では2回目・3回目の出現が許される。
            assert_eq!(
                snapshot.root_moves().contains(&cycle[3]),
                rule == RepetitionRule::R3
            );
            let mut outcomes = Vec::new();
            for draw_value in [0, -50] {
                let input = snapshot.clone().with_draw_value(draw_value).unwrap();
                with_snapshot(&input, &nnue, |searcher| {
                    let mut current = game.position().clone();
                    assert_eq!(
                        searcher.move_is_forbidden(&mut current, cycle[3]),
                        rule == RepetitionRule::R2
                    );
                    if rule == RepetitionRule::R3 {
                        assert_eq!(complete_cycle(searcher, &mut current, cycle[3]), DRAW_SCORE);
                    }
                });
                let result = crate::search::search(
                    &nnue,
                    &input,
                    &depth_limits(2),
                    DEFAULT_THREADS,
                    &mut small_tt(),
                    &mut crate::search::HistoryTables::new(DEFAULT_THREADS),
                )
                .unwrap();
                outcomes.push((result.score, result.best_move, result.nodes));
            }
            assert_eq!(outcomes[0], outcomes[1]);
        }
    }
}

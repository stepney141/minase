//! 反復による投了の抑止、誤投了の分類、および自己対局の引き分け値の契約。

use super::*;
use minase::nn::{Evaluator, NnError, Query};
use minase::search::mcts::tree::{Child, Outcome as ProofOutcome, Proof, Tree};
use minase_core::test_util::{position, sq};
use minase_core::{DrawReason, ExhaustionRule, GameStatus, PieceKind, WinReason};

#[test]
fn move_temperature_matches_the_schedule_at_specified_plies() {
    // 設計書のT(t)=0.15+0.85×0.5^(t/12)から求めた倍精度の参照値。
    for (ply, expected) in [
        (0, 1.0),
        (8, 0.6854664462053212),
        (12, 0.575),
        (16, 0.48732272354324235),
        (50, 0.19732899440120552),
    ] {
        let actual = f64::from(move_temperature(ply));
        assert!(
            (actual - expected).abs() <= expected * 1e-6,
            "t={ply}: {actual} != {expected}"
        );
    }
}

#[test]
fn search_decision_counts_the_random_opening_in_its_temperature() {
    let playing = PlayingGame::new(
        1,
        Settings {
            rules: Rules::ENGINE_DEFAULT,
            seed: 7,
            simulations: 2,
            playthrough_percent: 100,
            max_ply: 20,
        },
    )
    .unwrap();
    let game = &playing.game;
    assert!((8..=16).contains(&game.ply_count()));
    let snapshot = SearchSnapshot::from_game(game).unwrap();
    let moves = snapshot.root_moves().to_vec();
    let mut search = Search::new(snapshot, &mut XorShift64::new(derive_seed(7, 1)), 2).unwrap();
    let mut noise = vec![0.0; moves.len()];
    noise[0] = 1.0;
    search.gumbel = Gumbel::new(noise, 2, 1.0).unwrap();
    for simulation in 0..=2 {
        let Step::Evaluate(request) = search
            .driver
            .select_leaf(SelectionRule::Gumbel(&search.gumbel))
            .unwrap()
        else {
            panic!("the root and its two children need evaluation");
        };
        let mut logits = vec![0.0; request.query().moves.len()];
        if simulation == 0 {
            logits.fill(-1000.0);
            logits[0] = 0.0;
            logits[1] = 0.75;
        }
        search.driver.evaluate(&request, &logits, 0.5).unwrap();
    }
    assert!(search.finished());
    let root = search.driver.tree().root();
    assert_eq!(root.edges[0].visits, 1);
    assert_eq!(root.edges[1].visits, 1);
    // 全Qが1/2なのでσは0。倍率1ではg=1の第1辺、8〜16手後の倍率ではlogit=0.75の第2辺。
    assert_eq!(search.gumbel.action(root, 1.0).unwrap(), 0);
    assert_eq!(search.decision(game).unwrap().mv, moves[1]);
}

#[test]
fn search_uses_value_scale_one() {
    let snapshot = SearchSnapshot::from_game(&Game::new(Rules::ENGINE_DEFAULT)).unwrap();
    let mut search = Search::new(snapshot, &mut XorShift64::new(derive_seed(7, 1)), 2).unwrap();
    for (simulation, value) in [0.5, 0.25, 0.75].into_iter().enumerate() {
        let Step::Evaluate(request) = search
            .driver
            .select_leaf(SelectionRule::Gumbel(&search.gumbel))
            .unwrap()
        else {
            panic!("the root and its two children need evaluation");
        };
        let mut logits = vec![0.0; request.query().moves.len()];
        if simulation == 0 {
            logits.fill(-1000.0);
            logits[..2].fill(0.0);
        }
        search.driver.evaluate(&request, &logits, value).unwrap();
    }
    // Q=3/4と1/4の2辺を1回ずつ訪問したので、変換したQの最大値は(50+1)×1.0=51。
    let completed = search
        .gumbel
        .completed_qvalues(search.driver.tree().root())
        .unwrap();
    assert_eq!(
        completed.into_iter().fold(f32::NEG_INFINITY, f32::max),
        51.0
    );
}

#[test]
fn gumbel_noise_is_finite_at_both_uniform_endpoints() {
    // 設計書の中点による乱数では、最小・最大の入力も有限なGumbel乱数に写る。
    let minimum = gumbel_noise(0);
    let maximum = gumbel_noise(u64::MAX);
    assert!(minimum.is_finite() && minimum < 0.0);
    assert!(maximum.is_finite() && maximum > 30.0);
    assert!(minimum < gumbel_noise(1_u64 << 63));
    assert!(gumbel_noise(1_u64 << 63) < maximum);
}

fn small_tree(count: usize) -> Tree {
    let moves = Game::new(Rules::ENGINE_DEFAULT).legal_moves();
    let mut tree = Tree::new(&moves[..count], TREE_CAPACITY).unwrap();
    tree.evaluate(&[], &vec![0.0; count], 0.001).unwrap();
    tree
}

fn winning_child(repetition: bool) -> Child<'static> {
    Child::Proven(Proof {
        outcome: ProofOutcome::Win,
        distance: 0,
        repetition,
        approximate: repetition,
    })
}

#[test]
fn repetition_proven_loss_never_resigns_at_zero_q() {
    let mut tree = small_tree(2);
    let gumbel = Gumbel::new(vec![1.0, 0.0], 2, 1.0).unwrap();
    for _ in 0..2 {
        let selected = tree.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap();
        tree.create_child(&selected.path, winning_child(true))
            .unwrap();
    }
    assert!(matches!(
        tree.root().state,
        NodeState::Proven(Proof {
            outcome: ProofOutcome::Loss,
            repetition: true,
            ..
        })
    ));
    let action = gumbel.action(tree.root(), 1.0).unwrap();
    assert_eq!(tree.root().edges[action].q(), Some(0.0));
    assert!(!should_resign(tree.root(), action).unwrap());
}

#[test]
fn unvisited_final_action_uses_mixed_value_and_never_resigns() {
    // 訪問済みの候補がすべて負けでも、未訪問の手を混合価値つきで選べる。
    for repetition in [false, true] {
        let mut tree = small_tree(3);
        let gumbel = Gumbel::new(vec![100.0, 10.0, 0.0], 2, 1.0).unwrap();
        for _ in 0..2 {
            let selected = tree.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap();
            tree.create_child(&selected.path, winning_child(repetition))
                .unwrap();
        }
        let root = tree.root();
        assert_eq!(root.state, NodeState::Expanded);
        assert_eq!(root.edges.iter().filter(|edge| edge.visits > 0).count(), 2);
        let action = gumbel.action(root, 1.0).unwrap();
        assert_eq!(root.edges[action].visits, 0);
        assert_eq!(root.edges[action].q(), None);
        let value = selected_value(root, action).unwrap();
        assert_eq!(
            value.to_bits(),
            gumbel::mixed_value(root).unwrap().to_bits()
        );
        assert!(value > 0.0 && value < RESIGN_THRESHOLD);
        assert!(!should_resign(root, action).unwrap());
    }
}

#[test]
fn unresolved_root_with_unvisited_edges_resigns_only_without_repetition_cutoffs() {
    for (child, expected) in [
        (
            Child::Draw {
                value: 0.5,
                approximate: true,
            },
            false,
        ),
        (winning_child(true), false),
        (winning_child(false), true),
        (
            Child::Draw {
                value: 0.5,
                approximate: false,
            },
            true,
        ),
        (
            Child::Proven(Proof {
                outcome: ProofOutcome::Win,
                distance: 0,
                repetition: true,
                approximate: false,
            }),
            true,
        ),
    ] {
        let mut tree = small_tree(3);
        let gumbel = Gumbel::new(vec![100.0, 10.0, 0.0], 2, 1.0).unwrap();
        let first = tree.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap();
        let moves = Game::new(Rules::ENGINE_DEFAULT).legal_moves();
        tree.create_child(&first.path, Child::Pending(&moves[..2]))
            .unwrap();
        tree.evaluate(&first.path, &[0.0, 0.0], 0.999).unwrap();
        let second = tree.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap();
        tree.create_child(&second.path, child).unwrap();
        assert_eq!(tree.root().state, NodeState::Expanded);
        assert!(tree.root().edges.iter().any(|edge| edge.visits == 0));
        let selected = first.path[0];
        assert!(tree.root().edges[selected].q().unwrap() < 0.01);
        assert_eq!(should_resign(tree.root(), selected).unwrap(), expected);
    }
}

#[test]
fn false_resignation_counts_wins_draws_and_discards_but_not_losses() {
    for side in Color::ALL {
        for (result, expected) in [
            (
                Some(GameResult::Win {
                    winner: side.opposite(),
                    reason: WinReason::Mate,
                }),
                false,
            ),
            (
                Some(GameResult::Win {
                    winner: side,
                    reason: WinReason::RoyalCapture,
                }),
                true,
            ),
            (
                Some(GameResult::Draw {
                    reason: DrawReason::Repetition,
                }),
                true,
            ),
            (None, true),
        ] {
            assert_eq!(false_resignation(true, Some(side), result), expected);
            assert!(!false_resignation(false, Some(side), result));
            assert!(!false_resignation(true, None, result));
        }
    }
}

struct FavorMove(Move);

impl Evaluator for FavorMove {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        for query in queries {
            logits.extend(
                query
                    .moves
                    .iter()
                    .map(|&mv| if mv == self.0 { 1000.0 } else { 0.0 }),
            );
            values.push(0.5);
        }
        Ok(())
    }
}

fn step(from: minase_core::Square, to: minase_core::Square) -> Move {
    Move {
        from,
        mid: None,
        to,
        promote: false,
    }
}

#[test]
fn selfplay_driver_keeps_repetition_draw_at_half_despite_nonzero_draw_option() {
    let start = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(3, 3), Color::Black, PieceKind::Rook),
            (sq(8, 8), Color::White, PieceKind::Rook),
            (sq(11, 11), Color::White, PieceKind::King),
        ],
    );
    let line = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];
    let rules = Rules {
        exhaustion: ExhaustionRule::E2,
        ..Rules::ENGINE_DEFAULT
    };
    let mut game = Game::from_position(rules, start);
    for &mv in &line[..3] {
        assert_eq!(game.play(mv).unwrap(), GameStatus::Ongoing);
    }
    for draw_value in [-1200, 1200] {
        let snapshot = SearchSnapshot::from_game(&game)
            .unwrap()
            .with_draw_value(draw_value)
            .unwrap();
        // 本番と同じSearch::newを通し、DrawValue::SelfPlayの配線を検査する。
        let mut search = Search::new(snapshot, &mut XorShift64::new(derive_seed(7, 1)), 1).unwrap();
        let Step::Evaluate(request) = search
            .driver
            .select_leaf(SelectionRule::Gumbel(&search.gumbel))
            .unwrap()
        else {
            panic!("the root needs evaluation");
        };
        let mut logits = Vec::new();
        let mut values = Vec::new();
        FavorMove(line[3])
            .evaluate(&[request.query()], &mut logits, &mut values)
            .unwrap();
        search
            .driver
            .evaluate(&request, &logits, values[0])
            .unwrap();
        assert!(matches!(
            search
                .driver
                .select_leaf(SelectionRule::Gumbel(&search.gumbel))
                .unwrap(),
            Step::BackedUp
        ));
        let root = search.driver.tree().root();
        let edge = root.edges.iter().find(|edge| edge.mv == line[3]).unwrap();
        assert_eq!(
            edge.child.as_ref().unwrap().state,
            NodeState::Draw {
                value: 0.5,
                approximate: true
            }
        );
        assert_eq!(edge.visits, 1);
        assert!(search.finished());
    }
}

#[test]
fn single_simulation_plays_unvisited_action_after_repetition_loss() {
    // R1の直近1周では、白飛車だけが黒の歩兵への攻撃を続ける。
    // 4手目の反復は探索では白の負けになり、唯一訪問した候補から除かれる。
    let rules = Rules {
        exhaustion: ExhaustionRule::E2,
        ..Rules::ENGINE_DEFAULT
    };
    let mut game = PlayingGame::new(
        1,
        Settings {
            rules,
            seed: 7,
            simulations: 1,
            playthrough_percent: 1,
            max_ply: 6,
        },
    )
    .unwrap();
    game.game = Game::from_position(
        rules,
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(3, 3), Color::Black, PieceKind::Rook),
                (sq(8, 1), Color::Black, PieceKind::Pawn),
                (sq(8, 8), Color::White, PieceKind::Rook),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    );
    let line = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];
    for &mv in &line[..3] {
        assert_eq!(game.game.play(mv).unwrap(), GameStatus::Ongoing);
    }
    game.entries.clear();
    assert!(!game.playthrough);
    let mut evaluator = FavorMove(line[3]);
    while let Some(request) = game.advance().unwrap() {
        let mut logits = Vec::new();
        let mut values = Vec::new();
        evaluator
            .evaluate(&[request.query()], &mut logits, &mut values)
            .unwrap();
        game.evaluate(&request, &logits, values[0]).unwrap();
    }
    assert_eq!(game.game.ply_count(), 6);
    assert_eq!(game.entries.len(), 3);
    let entry = &game.entries[0];
    let played = entry.played_move().expect("the unvisited action is played");
    assert_ne!(played, line[3]);
    assert_eq!(entry.visits(), Some(1));
    // 根の評価0.5と、唯一の訪問の負け0を混合した0.25を量子化する。
    assert_eq!(entry.value(), Value::Mcts(16384));
    assert!(matches!(entry.policy(), Policy::Candidates(_)));
    assert!(!game.resigned);
    assert_eq!(game.first_trigger, None);
}

#[test]
fn proven_root_stops_before_budget_and_terminal_move_has_an_unsearched_last_entry() {
    let mut game = PlayingGame::new(
        1,
        Settings {
            rules: Rules::ENGINE_DEFAULT,
            seed: 7,
            simulations: 8,
            playthrough_percent: 100,
            max_ply: 1000,
        },
    )
    .unwrap();
    // 既知の王駒捕獲を持つ局面で、駆動部から着手・終局の記録までを通す。
    let capture = step(sq(4, 7), sq(4, 8));
    game.game = Game::from_position(
        Rules {
            exhaustion: ExhaustionRule::E2,
            ..Rules::ENGINE_DEFAULT
        },
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(4, 7), Color::Black, PieceKind::Rook),
                (sq(4, 8), Color::White, PieceKind::King),
            ],
        ),
    );
    game.entries.clear();
    let mut evaluator = FavorMove(capture);
    while let Some(request) = game.advance().unwrap() {
        let mut logits = Vec::new();
        let mut values = Vec::new();
        evaluator
            .evaluate(&[request.query()], &mut logits, &mut values)
            .unwrap();
        game.evaluate(&request, &logits, values[0]).unwrap();
    }
    let completed = game.finish().unwrap();
    assert!(!completed.resigned);
    assert_eq!(
        completed.result,
        Some(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture
        })
    );
    let record = completed.record.unwrap();
    assert_eq!(record.entries().len(), 2);
    assert_eq!(record.entries()[0].played_move(), Some(capture));
    assert_eq!(record.entries()[0].value(), Value::Mcts(65535));
    assert_eq!(record.entries()[0].visits(), Some(1));
    let last = record.entries().last().unwrap();
    assert_eq!(last.played_move(), None);
    assert_eq!(last.value(), Value::None);
    assert_eq!(last.policy(), &Policy::None);
    assert_eq!(last.visits(), Some(0));
}

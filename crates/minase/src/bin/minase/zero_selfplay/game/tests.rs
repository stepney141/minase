//! 反復による投了の抑止、手数の境界、および自己対局の引き分け値の契約。

use super::*;
use minase::nn::test_util::UniformEvaluator;
use minase::nn::{Evaluator, NnError, Query};
use minase::search::mcts::tree::{Child, Outcome as ProofOutcome, Proof, Tree};
use minase_core::test_util::{position, sq};
use minase_core::{ExhaustionRule, GameResult, GameStatus, PieceKind, WinReason};

fn play_to_end(game: &mut PlayingGame, evaluator: &mut impl Evaluator) -> usize {
    let mut logits = Vec::new();
    let mut values = Vec::new();
    let mut evaluations = 0;
    while let Some(request) = game.advance().unwrap() {
        evaluator
            .evaluate(&[request.query()], &mut logits, &mut values)
            .unwrap();
        game.evaluate(&request, &logits, values[0]).unwrap();
        evaluations += 1;
    }
    evaluations
}

#[test]
fn a_searched_move_can_end_in_a_mate_in_three_win_for_the_next_side() {
    // 白が遠方の歩兵を指すと、黒は飛車を捨てて3手で勝てる。
    // 白の探索した着手の直後に黒の勝ちを記録し、手数上限による引き分けより先に扱う。
    for resign_threshold in [0.0, 1.0] {
        let mut game = PlayingGame::new(
            2,
            Settings {
                start_position: StartPosition::Initial,
                rules: Rules::JCSA,
                seed: 7,
                simulations: 4,
                resign_threshold,
                max_ply: 1,
            },
        )
        .unwrap();
        game.game = Game::from_position(
            Rules::JCSA,
            position(
                Color::White,
                &[
                    (sq(2, 2), Color::Black, PieceKind::King),
                    (sq(3, 1), Color::Black, PieceKind::Rook),
                    (sq(2, 5), Color::Black, PieceKind::Rook),
                    (sq(0, 0), Color::White, PieceKind::King),
                    (sq(0, 1), Color::White, PieceKind::Pawn),
                    (sq(1, 0), Color::White, PieceKind::Pawn),
                    (sq(11, 5), Color::White, PieceKind::Pawn),
                ],
            ),
        );
        game.entries.clear();
        let mv = step(sq(11, 5), sq(11, 4));
        play_to_end(&mut game, &mut FavorMove(mv));
        assert!(game.game.result().is_none());
        let completed = game.finish().unwrap();
        assert_eq!(completed.plies, 1);
        assert_eq!(completed.searches, 1);
        let record = completed.record;
        assert_eq!(
            record.outcome(),
            Outcome::MateInThree {
                winner: Color::Black
            }
        );
        assert_eq!(record.entries().len(), 2);
        assert_eq!(record.entries()[0].played_move(), Some(mv));
        let last = &record.entries()[1];
        assert_eq!(last.played_move(), None);
        assert_eq!(last.value(), Value::None);
        assert_eq!(last.policy(), &Policy::None);
        assert_eq!(last.visits(), Some(0));
    }
}

#[test]
fn two_royals_do_not_cut_off_a_game_after_a_searched_move() {
    let mut game = PlayingGame::new(
        2,
        Settings {
            start_position: StartPosition::Initial,
            rules: Rules::JCSA,
            seed: 7,
            simulations: 4,
            resign_threshold: 0.0,
            max_ply: 1,
        },
    )
    .unwrap();
    let mut builder = minase_core::PositionBuilder::new(Color::White);
    for (square, color, kind) in [
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
        (sq(11, 5), Color::White, PieceKind::Pawn),
    ] {
        builder
            .put(square, minase_core::PieceCode::new(color, kind).unwrap())
            .unwrap();
    }
    builder
        .put(
            sq(10, 10),
            minase_core::PieceCode::new_promoted(Color::White, PieceKind::CrownPrince).unwrap(),
        )
        .unwrap();
    game.game = Game::from_position(
        Rules {
            exhaustion: ExhaustionRule::E2,
            ..Rules::JCSA
        },
        builder.finish().unwrap(),
    );
    game.entries.clear();
    play_to_end(&mut game, &mut FavorMove(step(sq(11, 5), sq(11, 4))));
    assert_eq!(game.game.ply_count(), 1);
    assert_eq!(game.mate_winner, None);
    assert_eq!(mate_in_three(&game.game).unwrap(), Verdict::TwoRoyals);
    assert_eq!(game.finish().unwrap().record.outcome(), Outcome::MaxPlyDraw);
}

#[test]
fn search_uses_value_scale_one() {
    let snapshot = SearchSnapshot::from_game(&Game::new(Rules::JCSA)).unwrap();
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
    let moves = Game::new(Rules::JCSA).legal_moves();
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
    assert!(!should_resign(
        tree.root(),
        selected_value(tree.root(), action).unwrap(),
        10,
        0.01
    ));
}

#[test]
fn unvisited_final_action_uses_mixed_value_and_checks_repetition() {
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
        assert!(value > 0.0 && value < 0.01);
        assert_eq!(should_resign(root, value, 10, 0.01), !repetition);
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
        let moves = Game::new(Rules::JCSA).legal_moves();
        tree.create_child(&first.path, Child::Pending(&moves[..2]))
            .unwrap();
        tree.evaluate(&first.path, &[0.0, 0.0], 0.999).unwrap();
        let second = tree.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap();
        tree.create_child(&second.path, child).unwrap();
        assert_eq!(tree.root().state, NodeState::Expanded);
        assert!(tree.root().edges.iter().any(|edge| edge.visits == 0));
        let selected = first.path[0];
        assert!(tree.root().edges[selected].q().unwrap() < 0.01);
        assert_eq!(
            should_resign(
                tree.root(),
                selected_value(tree.root(), selected).unwrap(),
                10,
                0.01
            ),
            expected
        );
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
        ..Rules::JCSA
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
fn proven_root_stops_before_budget_and_terminal_move_has_an_unsearched_last_entry() {
    let mut game = PlayingGame::new(
        2,
        Settings {
            start_position: StartPosition::Initial,
            rules: Rules::JCSA,
            seed: 7,
            simulations: 8,
            resign_threshold: 0.0,
            max_ply: 1,
        },
    )
    .unwrap();
    // 既知の王駒捕獲を持つ局面で、駆動部から着手・終局の記録までを通す。
    let capture = step(sq(4, 7), sq(4, 8));
    game.game = Game::from_position(
        Rules {
            exhaustion: ExhaustionRule::E2,
            ..Rules::JCSA
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
    assert_eq!(play_to_end(&mut game, &mut FavorMove(capture)), 1);
    let completed = game.finish().unwrap();
    assert_eq!(
        completed.record.outcome(),
        Outcome::Game(GameResult::Win {
            winner: Color::Black,
            reason: WinReason::RoyalCapture
        })
    );
    let record = completed.record;
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

// 第8条・第21条: 王手放置後の1手の勝ちは、探索の確定と着手後の打ち切りの双方に効く。

fn settings(max_ply: u32) -> Settings {
    Settings {
        rules: Rules::JCSA,
        seed: 7,
        start_position: StartPosition::Initial,
        simulations: 1,
        resign_threshold: 1.0,
        max_ply,
    }
}

fn root_with_visits(visits: &[u32]) -> Node {
    use minase::search::mcts::tree::Edge;
    let moves = Game::new(Rules::JCSA).legal_moves();
    let count = visits.len();
    Node {
        value: Some(0.5),
        state: NodeState::Expanded,
        visited_policy: 0.0,
        edges: visits
            .iter()
            .enumerate()
            .map(|(i, &visits)| Edge {
                mv: moves[i],
                logit: 0.0,
                policy: 1.0 / count as f32,
                visits,
                virtual_loss: 0,
                value_sum: f64::from(visits) * 0.5,
                child: None,
            })
            .collect(),
    }
}

#[test]
fn explorative_actions_use_all_visits_at_29_and_final_selection_at_30() {
    let root = root_with_visits(&[0, 1, 3, 0]);
    let seed = derive_seed(7, 2);
    let mut rng = XorShift64::new(seed);
    let mut expected_rng = XorShift64::new(seed);
    let mut counts = [0; 4];
    for _ in 0..16384 {
        // 仕様の累積区間は[0, 1)と[1, 4)。未訪問の辺は空区間になる。
        let expected = if expected_rng.index(NonZeroUsize::new(4).unwrap()) == 0 {
            1
        } else {
            2
        };
        let actual = explorative_action(&root, 2, 29, &mut rng);
        assert_eq!(actual, expected);
        counts[actual] += 1;
    }
    assert_eq!(counts[0], 0);
    assert_eq!(counts[3], 0);
    assert!((counts[1] as f64 / 16384.0 - 0.25).abs() < 0.02);
    for _ in 0..100 {
        assert_eq!(explorative_action(&root, 1, 30, &mut rng), 1);
    }
    assert_eq!(
        rng.next(),
        expected_rng.next(),
        "p=30 does not consume a draw"
    );
    let unvisited = root_with_visits(&[0, 0]);
    assert_eq!(explorative_action(&unvisited, 1, 29, &mut rng), 1);
    assert_eq!(
        rng.next(),
        expected_rng.next(),
        "zero visits do not consume a draw"
    );
}

#[test]
fn resignation_obeys_ply_threshold_and_approximation_boundaries() {
    let mut root = root_with_visits(&[0]);
    assert!(!should_resign(&root, 0.0, 9, 0.5));
    assert!(should_resign(&root, 0.0, 10, 0.5));
    assert!(!should_resign(&root, 0.5, 10, 0.5));
    assert!(should_resign(&root, 0.5, 10, 0.5000000001));
    assert!(!should_resign(&root, 0.0, 10, 0.0));
    root.edges[0].child = Some(Box::new(Node {
        state: NodeState::Draw {
            value: 0.5,
            approximate: true,
        },
        ..root_with_visits(&[])
    }));
    assert!(!should_resign(&root, 0.0, 10, 0.5));
}

#[test]
fn every_tenth_game_disables_resignation_and_other_games_resign_at_ply_ten() {
    for number in [1, 2, 10, 11, 12, 20, 21] {
        let mut playing = PlayingGame::new(number, settings(12)).unwrap();
        play_to_end(&mut playing, &mut UniformEvaluator);
        let completed = playing.finish().unwrap();
        let no_resign = [1, 11, 21].contains(&number);
        assert_eq!(completed.plies, if no_resign { 12 } else { 10 });
        assert_eq!(completed.searches, if no_resign { 12 } else { 11 });
        assert_eq!(completed.calibration.is_some(), no_resign);
        assert_eq!(
            completed.record.outcome(),
            if no_resign {
                Outcome::MaxPlyDraw
            } else {
                Outcome::Game(GameResult::Win {
                    winner: Color::White,
                    reason: WinReason::Resignation,
                })
            }
        );
        assert!(completed.record.entries()[9].played_move().is_some());
        assert_eq!(
            completed.record.entries()[10].played_move().is_some(),
            no_resign
        );
    }
}

#[test]
fn proven_values_override_stale_q_in_both_directions_including_approximation() {
    for approximate in [false, true] {
        for (outcome, expected) in [(ProofOutcome::Loss, 1.0), (ProofOutcome::Win, 0.0)] {
            let mut root = root_with_visits(&[1]);
            root.edges[0].value_sum = 0.37;
            root.edges[0].child = Some(Box::new(Node {
                state: NodeState::Proven(Proof {
                    outcome,
                    distance: 0,
                    repetition: approximate,
                    approximate,
                }),
                ..root_with_visits(&[])
            }));
            assert_eq!(selected_value(&root, 0).unwrap(), expected);
        }
    }
}

#[test]
fn selected_values_reject_nonfinite_and_out_of_range_values() {
    for value in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        let mut root = root_with_visits(&[1]);
        root.edges[0].value_sum = value;
        assert!(matches!(
            selected_value(&root, 0),
            Err(Error::InvalidValue(_))
        ));
    }
}

#[test]
fn calibration_uses_each_sides_minimum_including_the_first_ten_positions() {
    struct SequenceEvaluator(usize);
    impl Evaluator for SequenceEvaluator {
        fn evaluate(
            &mut self,
            queries: &[Query<'_>],
            logits: &mut Vec<f32>,
            values: &mut Vec<f32>,
        ) -> Result<(), NnError> {
            UniformEvaluator.evaluate(queries, logits, values)?;
            // 1手1探索では根と子を順に評価する。子の値から根の値へ反転する。
            let child_values = [0.875, 0.75, 0.5, 0.25];
            values[0] = if self.0.is_multiple_of(2) {
                0.5
            } else {
                child_values[self.0 / 2]
            };
            self.0 += 1;
            Ok(())
        }
    }
    let mut playing = PlayingGame::new(1, settings(4)).unwrap();
    play_to_end(&mut playing, &mut SequenceEvaluator(0));
    let completed = playing.finish().unwrap();
    let calibration = completed.calibration.unwrap();
    assert_eq!(calibration.min_value_black, 0.125);
    assert_eq!(calibration.min_value_white, 0.25);
    assert_eq!(calibration.result, CalibrationResult::Draw);
    assert_eq!(completed.record.outcome(), Outcome::MaxPlyDraw);
    assert_eq!(completed.record.entries().len(), 5);
    assert!(
        completed.record.entries()[..4]
            .iter()
            .all(|entry| matches!(entry.policy(), Policy::Candidates(_)))
    );
}

#[test]
fn missing_side_in_calibration_is_a_typed_error() {
    let mut playing = PlayingGame::new(1, settings(1)).unwrap();
    play_to_end(&mut playing, &mut UniformEvaluator);
    assert!(matches!(
        playing.finish(),
        Err(Error::MissingCalibrationValue {
            game: 1,
            side: Color::White
        })
    ));
}

#[test]
fn decision_keeps_final_selection_value_and_policy_when_exploration_plays_another_move() {
    let game = Game::new(Rules::JCSA);
    let mut search = Search::new(
        SearchSnapshot::from_game(&game).unwrap(),
        &mut XorShift64::new(derive_seed(7, 1)),
        2,
    )
    .unwrap();
    for (simulation, value) in [0.5, 0.25, 0.75].into_iter().enumerate() {
        let Step::Evaluate(request) = search
            .driver
            .select_leaf(SelectionRule::Gumbel(&search.gumbel))
            .unwrap()
        else {
            panic!("evaluation expected")
        };
        let mut logits = vec![0.0; request.query().moves.len()];
        if simulation == 0 {
            logits.fill(-1000.0);
            logits[..2].fill(0.0);
        }
        search.driver.evaluate(&request, &logits, value).unwrap();
    }
    let root = search.driver.tree().root();
    let action = search.gumbel.action(root, 1.0).unwrap();
    // 子の値1/4を根から見た3/4の手が最終選択となり、量子化値は49,151となる。
    assert_eq!(root.edges[action].q(), Some(0.75));
    let expected_value = Value::Mcts(49151);
    let expected_policy = search.policy(&game).unwrap();
    let mut differs = false;
    for seed in 1..32 {
        let decision = search
            .decision(&game, &mut XorShift64::new(derive_seed(seed, 2)), 0.0)
            .unwrap();
        differs |= decision.mv != root.edges[action].mv;
        assert_eq!(decision.value, expected_value);
        assert_eq!(decision.policy, expected_policy);
    }
    assert!(differs);
}

#[test]
fn proven_loss_records_zero_and_mate_takes_precedence_over_the_ply_limit() {
    let rules = Rules {
        e1: true,
        exhaustion: ExhaustionRule::E2,
        ..Rules::JCSA
    };
    let mut playing = PlayingGame::new(
        2,
        Settings {
            rules,
            simulations: 32,
            ..settings(1)
        },
    )
    .unwrap();
    playing.game = Game::from_position(
        rules,
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(0, 2), Color::White, PieceKind::Rook),
                (sq(1, 2), Color::White, PieceKind::Rook),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    );
    playing.start_position = playing.game.position().clone();
    assert_eq!(play_to_end(&mut playing, &mut UniformEvaluator), 1);
    let completed = playing.finish().unwrap();
    assert_eq!(
        completed.record.outcome(),
        Outcome::MateInThree {
            winner: Color::White
        }
    );
    assert_eq!(completed.record.entries()[0].value(), Value::Mcts(0));
    assert!(matches!(
        completed.record.entries()[0].policy(),
        Policy::Candidates(_)
    ));
    assert_eq!(completed.record.entries()[0].visits(), Some(3));
}

#[test]
fn decision_uses_exploration_at_29_and_gumbel_scale_one_at_30() {
    let mut playing = PlayingGame::new(1, settings(30)).unwrap();
    for ply in [29, 30] {
        playing.settings.max_ply = ply;
        play_to_end(&mut playing, &mut UniformEvaluator);
        assert_eq!(playing.game.ply_count(), ply);
        let game = &playing.game;
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
                panic!("evaluation expected")
            };
            let mut logits = vec![0.0; request.query().moves.len()];
            if simulation == 0 {
                logits.fill(-1000.0);
                logits[0] = 0.0;
                logits[1] = 0.75;
            }
            search.driver.evaluate(&request, &logits, 0.5).unwrap();
        }
        let mut rng = XorShift64::new(derive_seed(7, 2));
        let mut expected_rng = XorShift64::new(derive_seed(7, 2));
        let mut played_second = false;
        for _ in 0..64 {
            let expected = if ply == 29 {
                expected_rng.index(NonZeroUsize::new(2).unwrap())
            } else {
                0
            };
            let decision = search.decision(game, &mut rng, 0.0).unwrap();
            assert_eq!(decision.mv, moves[expected]);
            played_second |= decision.mv == moves[1];
        }
        assert_eq!(played_second, ply == 29);
    }
}

#[test]
fn exploration_rng_uses_the_game_seed_with_derivation_two() {
    for number in [1, 2, 11] {
        let mut playing = PlayingGame::new(number, settings(12)).unwrap();
        let game_seed = derive_seed(7, u64::from(number));
        let mut expected = XorShift64::new(derive_seed(game_seed.get(), 2));
        for _ in 0..32 {
            assert_eq!(playing.action_rng.next(), expected.next());
        }
    }
}

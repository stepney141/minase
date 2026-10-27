use super::*;
use crate::nn::UniformEvaluator;
use crate::search::mcts::gumbel::{Gumbel, policy_target};
use crate::search::mcts::puct::PuctConfig;
use crate::search::mcts::test_support::*;
use crate::search::{MATE, MATE_THRESHOLD, MAX_PLY, REPETITION_WIN, in_repetition_band};
use minase_core::test_util::{position, sq};
use minase_core::{Game, GameStatus, PieceKind, RepetitionRule, Rules};

fn advance(driver: &mut Driver, evaluator: &mut dyn crate::nn::Evaluator) -> Option<Vec<usize>> {
    match driver
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap()
    {
        Step::Evaluate(request) => {
            let path = request.path.clone();
            evaluate(driver, evaluator, &request);
            Some(path)
        }
        Step::BackedUp => None,
        _ => panic!("sequential search has capacity and no pending requests"),
    }
}

// 指示書3: 根を必ず評価し、最後の王駒の捕獲は子の評価なしでMATE - 1となる。
#[test]
fn last_royal_capture_is_proven_without_evaluating_the_child() {
    let root = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 4), Color::Black, PieceKind::Rook),
            (sq(5, 9), Color::White, PieceKind::King),
        ],
    );
    let mv = step(sq(5, 4), sq(5, 9));
    let mut input = snapshot(&root);
    input.root_moves = vec![mv];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    let mut evaluator = UniformEvaluator;
    advance(&mut driver, &mut evaluator);
    assert_eq!(driver.simulations(), 1);
    assert_eq!(advance(&mut driver, &mut evaluator), None);
    assert_eq!(driver.simulations(), 2);
    assert_eq!(driver.score(SCALE).unwrap(), MATE - 1);
    assert_eq!(driver.tree().best_move(), Some(mv));
    assert_eq!(driver.tree().principal_variation(), [mv]);
    assert_eq!(driver.tree().node(&[0]).unwrap().value, None);
}

// R1は直近の周回の一方だけが攻撃を続けた場合に、その側の負けとする。
// 2者のどちらが攻撃側でも、根を4か所へ回転しても、終端を推論へ送らない。
#[test]
fn r1_path_cycles_classify_both_attackers_and_draws_without_inference() {
    for attacks in [[true, false], [false, true], [false, false], [true, true]] {
        let (mut root, cycle) = rook_cycle(attacks[0], attacks[1]);
        for offset in 0..4 {
            let line: Vec<_> = (0..4).map(|i| cycle[(offset + i) % 4]).collect();
            let mut evaluator = ScriptedEvaluator::line(&root, &line);
            let mut driver = Driver::new(snapshot(&root), CAPACITY, DrawValue::SelfPlay).unwrap();
            for _ in 0..4 {
                assert!(advance(&mut driver, &mut evaluator).is_some());
            }
            assert_eq!(advance(&mut driver, &mut evaluator), None);
            let terminal = driver.tree().node(&path(&driver, &line)).unwrap();
            let expected = if attacks[0] == attacks[1] {
                NodeState::Draw(0.5)
            } else {
                let side_attacks = attacks[root.side_to_move().index()];
                proven(
                    if side_attacks {
                        Outcome::Loss
                    } else {
                        Outcome::Win
                    },
                    true,
                )
            };
            assert_eq!(terminal.state, expected);
            assert_eq!(terminal.value, None);
            assert_eq!(evaluator.seen.len(), 4);
            assert_eq!(
                evaluator
                    .seen
                    .iter()
                    .filter(|position| **position == root)
                    .count(),
                1
            );
            assert_eq!(driver.simulations(), 5);
            root.make_move_unchecked(cycle[offset], Rules::ENGINE_DEFAULT.moves);
        }
    }
}

// 履歴部分の攻撃連続数は、周回に必要な回数とちょうど等しい場合にだけ十分となる。
#[test]
fn r1_history_counts_cover_the_history_segment_and_current_move() {
    for attacker in Color::ALL {
        let (root, line) = rook_cycle(attacker == Color::Black, attacker == Color::White);
        for prefix in [2, 3] {
            let mut game = Game::from_position(rules(RepetitionRule::R1), root.clone());
            for &mv in &line[..prefix] {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
            for sufficient in [false, true] {
                let mut input = SearchSnapshot::from_game(&game).unwrap();
                if !sufficient {
                    let SearchRepetition::R1 {
                        ref mut consecutive_attacking_moves,
                    } = input.repetition
                    else {
                        panic!()
                    };
                    consecutive_attacking_moves[attacker.index()] -= 1;
                }
                let mut evaluator = ScriptedEvaluator::line(game.position(), &line[prefix..]);
                let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
                for _ in prefix..4 {
                    advance(&mut driver, &mut evaluator);
                }
                assert_eq!(advance(&mut driver, &mut evaluator), None);
                let terminal = driver.tree().node(&path(&driver, &line[prefix..])).unwrap();
                assert_eq!(
                    terminal.state,
                    if sufficient {
                        proven(
                            if attacker == Color::Black {
                                Outcome::Loss
                            } else {
                                Outcome::Win
                            },
                            true,
                        )
                    } else {
                        NodeState::Draw(0.5)
                    }
                );
                assert_eq!(evaluator.seen.len(), 4 - prefix);
            }
        }
    }
}

#[test]
fn r1_uses_the_nearest_occurrence_in_history() {
    let (root, cycle) = rook_cycle(true, false);
    let detour = [
        step(sq(3, 3), sq(4, 3)),
        cycle[1],
        step(sq(4, 3), sq(3, 3)),
        cycle[3],
    ];
    let mut game = Game::from_position(rules(RepetitionRule::R1), root);
    for &mv in detour.iter().chain(&cycle[..3]) {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }
    let mut input = SearchSnapshot::from_game(&game).unwrap();
    input.root_moves = vec![cycle[3]];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    advance(&mut driver, &mut UniformEvaluator);
    assert_eq!(driver.score(SCALE).unwrap(), REPETITION_WIN - 1);
}

// 根は対局管理層の候補を保ち、内部節点ではR2の1回、R3の3回を禁止の閾値にする。
#[test]
fn r2_r3_root_moves_exclude_the_forbidden_return() {
    for (rule, rounds) in [(RepetitionRule::R2, 0), (RepetitionRule::R3, 2)] {
        let (root, cycle) = rook_cycle(false, false);
        let mut game = Game::from_position(rules(rule), root);
        for _ in 0..rounds {
            for &mv in &cycle {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
        }
        for &mv in &cycle[..3] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }
        let input = SearchSnapshot::from_game(&game).unwrap();
        let mut expected = legal_moves(game.position());
        expected.retain(|&mv| mv != cycle[3]);
        assert_eq!(input.root_moves(), expected);
        let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        let Step::Evaluate(request) = driver
            .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(request.query().moves, expected);
        evaluate(&mut driver, &mut UniformEvaluator, &request);
    }
}

#[test]
fn r2_r3_child_moves_filter_exact_history_thresholds() {
    for (rule, threshold) in [(RepetitionRule::R2, 1), (RepetitionRule::R3, 3)] {
        for count in 0..=threshold {
            let mut input = initial();
            input.repetition = if rule == RepetitionRule::R2 {
                SearchRepetition::R2
            } else {
                SearchRepetition::R3
            };
            let first = input.root_moves()[0];
            input.root_moves = vec![first];
            let mut child = input.position().clone();
            child.make_move_unchecked(first, input.rules());
            let mut expected = legal_moves(&child);
            let forbidden = expected[0];
            let mut returned = child.clone();
            returned.make_move_unchecked(forbidden, input.rules());
            input.history_keys = vec![repetition_key(rule, &returned); count];
            input
                .history_keys
                .push(repetition_key(rule, input.position()));
            if count == threshold {
                expected.retain(|&mv| mv != forbidden);
            }
            let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
            advance(&mut driver, &mut UniformEvaluator);
            let Step::Evaluate(request) = driver
                .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(request.query().moves, expected);
            evaluate(&mut driver, &mut UniformEvaluator, &request);
        }
    }
}

// 根を履歴末尾と経路先頭の両方で数えると、R3の2回目を誤って禁止する。
#[test]
fn prohibition_counts_the_root_once_and_includes_the_child_path() {
    let (root, line) = rook_cycle(false, false);
    for (rule, root_occurrences, forbidden) in [
        (RepetitionRule::R2, 1, true),
        (RepetitionRule::R3, 1, false),
        (RepetitionRule::R3, 2, false),
        (RepetitionRule::R3, 3, true),
    ] {
        let mut input =
            SearchSnapshot::from_game(&Game::from_position(rules(rule), root.clone())).unwrap();
        input.history_keys = vec![repetition_key(rule, &root); root_occurrences];
        let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        let mut evaluator = ScriptedEvaluator::line(&root, &line);
        for _ in 0..4 {
            advance(&mut driver, &mut evaluator);
        }
        let parent = driver.tree().node(&path(&driver, &line[..3])).unwrap();
        assert_eq!(
            parent.edges.iter().any(|edge| edge.mv == line[3]),
            !forbidden
        );
        if !forbidden {
            assert_eq!(advance(&mut driver, &mut evaluator), None);
            assert_eq!(
                driver.tree().node(&path(&driver, &line)).unwrap().state,
                NodeState::Draw(0.5)
            );
        }
    }
}

// 履歴に子の全応手の到達先を置く。子自体は初出なので禁止手を除く段まで進む。
#[test]
fn all_forbidden_replies_prove_a_loss_in_the_repetition_band() {
    for (repetition, count) in [(SearchRepetition::R2, 1), (SearchRepetition::R3, 3)] {
        let mut input = initial();
        input.repetition = repetition;
        let mv = input.root_moves()[0];
        input.root_moves = vec![mv];
        let mut child = input.position().clone();
        child.make_move_unchecked(mv, input.rules());
        let mut history = Vec::new();
        for reply in legal_moves(&child) {
            let mut next = child.clone();
            next.make_move_unchecked(reply, input.rules());
            history.extend(vec![repetition_key(repetition.rule(), &next); count]);
        }
        assert!(!history.contains(&repetition_key(repetition.rule(), &child)));
        history.push(repetition_key(repetition.rule(), input.position()));
        input.history_keys = history;
        let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        advance(&mut driver, &mut UniformEvaluator);
        assert_eq!(advance(&mut driver, &mut UniformEvaluator), None);
        assert_eq!(
            driver.tree().node(&[0]).unwrap().state,
            proven(Outcome::Loss, true)
        );
        assert_eq!(driver.score(SCALE).unwrap(), REPETITION_WIN - 1);
        assert!(in_repetition_band(driver.score(SCALE).unwrap()));
    }
}

#[test]
fn repetition_loss_at_the_root_uses_its_own_score_band() {
    let (root, line) = rook_cycle(true, false);
    let mut game = Game::from_position(rules(RepetitionRule::R1), root);
    // 3手先の黒の攻撃的な戻りを、根から1手の反復負けとして読む。
    for mv in [line[0], line[1], line[2], line[3], line[0], line[1]] {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }
    let mut input = SearchSnapshot::from_game(&game).unwrap();
    input.root_moves = vec![line[2]];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    advance(&mut driver, &mut UniformEvaluator);
    assert_eq!(driver.score(SCALE).unwrap(), -(REPETITION_WIN - 1));
    assert!(in_repetition_band(driver.score(SCALE).unwrap()));
    assert!(driver.score(SCALE).unwrap().abs() < MATE_THRESHOLD);
}

#[test]
fn draws_use_root_perspective_and_selfplay_stays_neutral() {
    let (root, line) = rook_cycle(false, false);
    for prefix in [0, 1] {
        let mut game = Game::from_position(rules(RepetitionRule::R1), root.clone());
        for &mv in &line[..prefix] {
            game.play(mv).unwrap();
        }
        for cp in [-1200, 1200] {
            for draw in [
                DrawValue::Game {
                    cp_per_logit: SCALE,
                },
                DrawValue::SelfPlay,
            ] {
                let input = SearchSnapshot::from_game(&game)
                    .unwrap()
                    .with_draw_value(cp)
                    .unwrap();
                let mut evaluator = ScriptedEvaluator::line(game.position(), &line[prefix..]);
                let mut driver = Driver::new(input, CAPACITY, draw).unwrap();
                for _ in prefix..=4 {
                    advance(&mut driver, &mut evaluator);
                }
                let state = driver
                    .tree()
                    .node(&path(&driver, &line[prefix..]))
                    .unwrap()
                    .state;
                let NodeState::Draw(actual) = state else {
                    panic!("{state:?}")
                };
                let r = match draw {
                    DrawValue::SelfPlay => 0.5,
                    DrawValue::Game { .. } => {
                        1.0 / (1.0 + (-f64::from(cp) / f64::from(SCALE)).exp())
                    }
                };
                let expected = if prefix == 0 { r } else { 1.0 - r };
                assert!((f64::from(actual) - expected).abs() < 1e-7);
            }
        }
    }
    // R3の合法な2回目の再現は、対局用の設定でも0.5とする。
    let input = SearchSnapshot::from_game(&Game::from_position(
        rules(RepetitionRule::R3),
        root.clone(),
    ))
    .unwrap()
    .with_draw_value(1200)
    .unwrap();
    let mut driver = Driver::new(
        input,
        CAPACITY,
        DrawValue::Game {
            cp_per_logit: SCALE,
        },
    )
    .unwrap();
    let mut evaluator = ScriptedEvaluator::line(&root, &line);
    for _ in 0..5 {
        advance(&mut driver, &mut evaluator);
    }
    assert_eq!(
        driver.tree().node(&path(&driver, &line)).unwrap().state,
        NodeState::Draw(0.5)
    );
}

#[test]
fn depth_limit_is_a_network_leaf_and_score_bands_include_max_ply() {
    let mut input = initial();
    input.root_moves.truncate(1);
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    assert_eq!(driver.tree.depth_limit(), MAX_PLY);
    driver.tree.max_ply = 2;
    advance(&mut driver, &mut UniformEvaluator);
    advance(&mut driver, &mut UniformEvaluator);
    let Step::Evaluate(request) = driver
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(request.path.len(), 2);
    assert!(request.query().moves.is_empty());
    evaluate(&mut driver, &mut UniformEvaluator, &request);
    let leaf = driver.tree.node(&request.path).unwrap();
    assert_eq!(leaf.state, NodeState::Unexpanded);
    assert!(leaf.edges.is_empty());
    for repetition in [false, true] {
        for outcome in [Outcome::Win, Outcome::Loss] {
            let score = Proof {
                outcome,
                distance: MAX_PLY,
                repetition,
            }
            .score();
            assert_eq!(in_repetition_band(score), repetition);
            assert_eq!(score.abs() >= MATE_THRESHOLD, !repetition);
        }
    }
}

#[test]
fn reroot_preserves_repetition_judgments_against_fresh_history() {
    for attacks in [[false, false], [true, false], [false, true]] {
        let (root, line) = rook_cycle(attacks[0], attacks[1]);
        let mut game = Game::from_position(rules(RepetitionRule::R1), root.clone());
        let mut driver = Driver::new(
            SearchSnapshot::from_game(&game).unwrap(),
            CAPACITY,
            DrawValue::SelfPlay,
        )
        .unwrap();
        let mut evaluator = ScriptedEvaluator::line(&root, &line);
        for _ in 0..5 {
            advance(&mut driver, &mut evaluator);
        }
        let before = driver.tree().node(&path(&driver, &line)).unwrap().state;
        game.play(line[0]).unwrap();
        let input = SearchSnapshot::from_game(&game).unwrap();
        assert!(driver.reuse(&input, DrawValue::SelfPlay).unwrap());
        assert_eq!(driver.simulations(), 0);
        assert_eq!(
            driver
                .tree()
                .node(&path(&driver, &line[1..]))
                .unwrap()
                .state,
            before
        );
        let mut fresh = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        let mut evaluator = ScriptedEvaluator::line(game.position(), &line[1..]);
        for _ in 0..4 {
            advance(&mut fresh, &mut evaluator);
        }
        assert_eq!(
            fresh.tree().node(&path(&fresh, &line[1..])).unwrap().state,
            before
        );
    }
}

#[test]
fn repeated_terminal_cannot_be_reused_as_a_root() {
    let (root, line) = rook_cycle(false, false);
    let mut game = Game::from_position(rules(RepetitionRule::R1), root.clone());
    let mut driver = Driver::new(
        SearchSnapshot::from_game(&game).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    let mut evaluator = ScriptedEvaluator::line(&root, &line);
    for _ in 0..5 {
        advance(&mut driver, &mut evaluator);
    }
    for mv in line {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }
    let input = SearchSnapshot::from_game(&game).unwrap();
    assert!(!driver.reuse(&input, DrawValue::SelfPlay).unwrap());
    let mut fresh = Driver::new(input.clone(), CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut fresh, &mut UniformEvaluator);
    assert_eq!(fresh.tree().root().state, NodeState::Expanded);
    assert_eq!(
        fresh
            .tree()
            .root()
            .edges
            .iter()
            .map(|edge| edge.mv)
            .collect::<Vec<_>>(),
        input.root_moves()
    );
}

#[test]
fn gumbel_driver_completes_32_simulations_and_targets_all_legal_moves() {
    let input = initial().with_draw_value(1200).unwrap();
    let moves = input.root_moves().to_vec();
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    let gumbel = Gumbel::new(vec![0.0; moves.len()], 32).unwrap();
    for _ in 0..32 {
        match driver.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap() {
            Step::Evaluate(request) => evaluate(&mut driver, &mut UniformEvaluator, &request),
            Step::BackedUp => {}
            _ => panic!("Gumbel is sequential and has sufficient capacity"),
        }
    }
    assert_eq!(driver.simulations(), 33);
    let root = driver.tree().root();
    assert!(moves.contains(&root.edges[gumbel.action(root).unwrap()].mv));
    let target = policy_target(root).unwrap();
    assert_eq!(target.len(), moves.len());
    assert!(target.iter().all(|p| *p > 0.0));
    assert!((target.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    assert!(matches!(
        driver.select_leaf(SelectionRule::Gumbel(&gumbel)),
        Err(Error::Tree(TreeError::BudgetExhausted))
    ));
}

#[test]
fn leaves_from_multiple_games_share_one_inference_batch() {
    let mut drivers: Vec<_> = (0..3)
        .map(|_| Driver::new(initial(), CAPACITY, DrawValue::SelfPlay).unwrap())
        .collect();
    let config = PuctConfig::default();
    let requests: Vec<_> = drivers
        .iter_mut()
        .map(|driver| {
            let Step::Evaluate(request) = driver.select_leaf(SelectionRule::Puct(&config)).unwrap()
            else {
                panic!()
            };
            request
        })
        .collect();
    let queries: Vec<_> = requests.iter().map(EvaluationRequest::query).collect();
    let mut logits = Vec::new();
    let mut values = Vec::new();
    crate::nn::Evaluator::evaluate(&mut UniformEvaluator, &queries, &mut logits, &mut values)
        .unwrap();
    let mut offset = 0;
    for ((driver, request), value) in drivers.iter_mut().zip(&requests).zip(values) {
        let end = offset + request.query().moves.len();
        driver
            .evaluate(request, &logits[offset..end], value)
            .unwrap();
        assert_eq!(driver.simulations(), 1);
        offset = end;
    }
    assert_eq!(offset, logits.len());
}

#[test]
fn score_uses_best_q_root_value_and_probability_clamps() {
    for (value, expected) in [
        (0.0, -6907),
        (0.2, -1386),
        (0.5, 0),
        (0.8, 1386),
        (1.0, 6907),
    ] {
        let mut input = initial();
        input.root_moves.truncate(1);
        let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        let config = PuctConfig::default();
        let Step::Evaluate(root) = driver.select_leaf(SelectionRule::Puct(&config)).unwrap() else {
            panic!()
        };
        driver.evaluate(&root, &[0.0], value).unwrap();
        assert_eq!(driver.score(1000.0).unwrap(), expected);
        let Step::Evaluate(child) = driver.select_leaf(SelectionRule::Puct(&config)).unwrap()
        else {
            panic!()
        };
        driver
            .evaluate(&child, &vec![0.0; child.query().moves.len()], 1.0 - value)
            .unwrap();
        assert_eq!(driver.score(1000.0).unwrap(), expected);
    }
}

// 第23条: 反復禁止を適用する前から合法手がなければ、詰みの印を付ける。
#[test]
fn no_generated_moves_are_a_mate_loss_before_repetition_filtering() {
    let root = position(
        Color::White,
        &[
            (sq(0, 11), Color::Black, PieceKind::King),
            (sq(1, 11), Color::Black, PieceKind::Pawn),
            (sq(0, 10), Color::Black, PieceKind::Lance),
            (sq(1, 10), Color::Black, PieceKind::Lance),
            (sq(10, 0), Color::White, PieceKind::King),
        ],
    );
    let mv = step(sq(10, 0), sq(10, 1));
    let mut input =
        SearchSnapshot::from_game(&Game::from_position(rules(RepetitionRule::R2), root)).unwrap();
    input.root_moves = vec![mv];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    assert_eq!(advance(&mut driver, &mut UniformEvaluator), None);
    assert_eq!(
        driver.tree().node(&[0]).unwrap().state,
        proven(Outcome::Loss, false)
    );
    assert_eq!(driver.score(SCALE).unwrap(), MATE - 1);
}

// 駆動部の依頼は取消し後に再選択でき、誤った推論結果では回数を増やさない。
#[test]
fn invalid_evaluation_and_cancel_leave_simulations_and_virtual_losses_consistent() {
    let mut input = initial();
    input.root_moves.truncate(1);
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    let config = PuctConfig::default();
    let rule = SelectionRule::Puct(&config);
    let Step::Evaluate(root) = driver.select_leaf(rule).unwrap() else {
        panic!()
    };
    assert!(matches!(driver.select_leaf(rule), Ok(Step::Collision)));
    assert!(matches!(
        driver.evaluate(&root, &[], 0.5),
        Err(Error::Tree(TreeError::InvalidEvaluation))
    ));
    assert_eq!(driver.simulations(), 0);
    driver.cancel(&root).unwrap();
    assert!(driver.evaluate(&root, &[0.0], 0.5).is_err());
    advance(&mut driver, &mut UniformEvaluator);
    let Step::Evaluate(child) = driver.select_leaf(rule).unwrap() else {
        panic!()
    };
    assert!(matches!(driver.select_leaf(rule), Ok(Step::Collision)));
    driver.cancel(&child).unwrap();
    assert_eq!(driver.simulations(), 1);
    assert_eq!(driver.tree().root().edges[0].virtual_loss, 0);
    assert!(driver.tree().root().edges[0].child.is_none());
    advance(&mut driver, &mut UniformEvaluator);
    assert_eq!(driver.simulations(), 2);
}

#[test]
fn reroot_preserves_r2_r3_prohibitions_when_path_becomes_history() {
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let (root, line) = rook_cycle(false, false);
        let mut game = Game::from_position(rules(rule), root.clone());
        let mut driver = Driver::new(
            SearchSnapshot::from_game(&game).unwrap(),
            CAPACITY,
            DrawValue::SelfPlay,
        )
        .unwrap();
        let mut evaluator = ScriptedEvaluator::line(&root, &line);
        for _ in 0..4 {
            advance(&mut driver, &mut evaluator);
        }
        let before: Vec<_> = driver
            .tree()
            .node(&path(&driver, &line[..3]))
            .unwrap()
            .edges
            .iter()
            .map(|edge| edge.mv)
            .collect();
        assert_eq!(before.contains(&line[3]), rule == RepetitionRule::R3);
        game.play(line[0]).unwrap();
        let input = SearchSnapshot::from_game(&game).unwrap();
        assert!(driver.reuse(&input, DrawValue::SelfPlay).unwrap());
        let after: Vec<_> = driver
            .tree()
            .node(&path(&driver, &line[1..3]))
            .unwrap()
            .edges
            .iter()
            .map(|edge| edge.mv)
            .collect();
        assert_eq!(before, after);
        let mut fresh = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        let mut evaluator = ScriptedEvaluator::line(game.position(), &line[1..]);
        for _ in 0..3 {
            advance(&mut fresh, &mut evaluator);
        }
        let expected: Vec<_> = fresh
            .tree()
            .node(&path(&fresh, &line[1..3]))
            .unwrap()
            .edges
            .iter()
            .map(|edge| edge.mv)
            .collect();
        assert_eq!(after, expected);
    }
}

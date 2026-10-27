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
                NodeState::Draw {
                    value: 0.5,
                    approximate: true,
                }
            } else {
                let side_attacks = attacks[root.side_to_move().index()];
                proven(
                    if side_attacks {
                        Outcome::Loss
                    } else {
                        Outcome::Win
                    },
                    true,
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

// 履歴部分もGame::playで作り、攻撃連続数を手作業で注入しない。
#[test]
fn r1_history_counts_cover_the_history_segment_and_current_move() {
    for attacks in [[true, false], [false, true], [false, false], [true, true]] {
        let (root, line) = rook_cycle(attacks[0], attacks[1]);
        for prefix in [2, 3] {
            let mut game = Game::from_position(rules(RepetitionRule::R1), root.clone());
            for &mv in &line[..prefix] {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
            let mut driver = Driver::new(
                SearchSnapshot::from_game(&game).unwrap(),
                CAPACITY,
                DrawValue::SelfPlay,
            )
            .unwrap();
            let mut evaluator = ScriptedEvaluator::line(game.position(), &line[prefix..]);
            for _ in prefix..4 {
                advance(&mut driver, &mut evaluator);
            }
            assert_eq!(advance(&mut driver, &mut evaluator), None);
            let terminal = driver.tree().node(&path(&driver, &line[prefix..])).unwrap();
            let expected = match attacks {
                [true, false] => proven(Outcome::Loss, true, true),
                [false, true] => proven(Outcome::Win, true, true),
                _ => NodeState::Draw {
                    value: 0.5,
                    approximate: true,
                },
            };
            assert_eq!(terminal.state, expected);
            assert_eq!(evaluator.seen.len(), 4 - prefix);
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

// 過去の周回とは異なる迂回を通り、初出の子から既出局面へ戻る応手を調べる。
#[test]
fn r2_r3_child_moves_filter_exact_history_thresholds() {
    for (rule, threshold) in [(RepetitionRule::R2, 1), (RepetitionRule::R3, 3)] {
        for count in 1..=threshold {
            let (root, cycle) = rook_cycle(false, false);
            let mut game = Game::from_position(rules(rule), root);
            let old_cycle = [
                cycle[0],
                step(sq(8, 8), sq(8, 6)),
                cycle[2],
                step(sq(8, 6), sq(8, 8)),
            ];
            for _ in 1..count {
                for mv in old_cycle {
                    assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
                }
            }
            for &mv in &cycle[..2] {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
            let mut driver = Driver::new(
                SearchSnapshot::from_game(&game).unwrap(),
                CAPACITY,
                DrawValue::SelfPlay,
            )
            .unwrap();
            let mut evaluator = ScriptedEvaluator::line(game.position(), &cycle[2..]);
            advance(&mut driver, &mut evaluator);
            let Step::Evaluate(request) = driver
                .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
                .unwrap()
            else {
                panic!()
            };
            assert_eq!(game.play(cycle[2]), Ok(GameStatus::Ongoing));
            assert_eq!(request.query().moves, game.legal_moves());
            assert_eq!(request.query().moves.contains(&cycle[3]), count < threshold);
            evaluate(&mut driver, &mut evaluator, &request);
        }
    }
}

// 根は履歴末尾と経路先頭の両方にあるが、R3の合法な2回目は辺に残す。
#[test]
fn prohibition_counts_the_root_once_and_includes_the_child_path() {
    let (root, line) = rook_cycle(false, false);
    for rule in [RepetitionRule::R2, RepetitionRule::R3] {
        let input =
            SearchSnapshot::from_game(&Game::from_position(rules(rule), root.clone())).unwrap();
        let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        let mut evaluator = ScriptedEvaluator::line(&root, &line);
        for _ in 0..4 {
            advance(&mut driver, &mut evaluator);
        }
        let parent = driver.tree().node(&path(&driver, &line[..3])).unwrap();
        assert_eq!(
            parent.edges.iter().any(|edge| edge.mv == line[3]),
            rule == RepetitionRule::R3
        );
        if rule == RepetitionRule::R3 {
            assert_eq!(advance(&mut driver, &mut evaluator), None);
            assert_eq!(
                driver.tree().node(&path(&driver, &line)).unwrap().state,
                NodeState::Draw {
                    value: 0.5,
                    approximate: true
                }
            );
        }
    }
}

// 第23条: 局面合法手がすべて反復で禁止される場合も、審判層の合法手なしで確定する。
#[test]
fn all_forbidden_replies_are_an_exact_stalemate_in_the_mate_band() {
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::GoBetween),
        (sq(11, 11), Color::Black, PieceKind::King),
        (sq(10, 10), Color::Black, PieceKind::Pawn),
        (sq(10, 11), Color::Black, PieceKind::Pawn),
        (sq(11, 10), Color::Black, PieceKind::Pawn),
        (sq(5, 5), Color::White, PieceKind::King),
    ];
    pieces.extend((2..=11).map(|rank| (sq(0, rank), Color::Black, PieceKind::Pawn)));
    let cycle = [
        step(sq(5, 5), sq(5, 4)),
        step(sq(0, 0), sq(0, 1)),
        step(sq(5, 4), sq(5, 5)),
        step(sq(0, 1), sq(0, 0)),
    ];
    for (rule, rounds) in [(RepetitionRule::R2, 0), (RepetitionRule::R3, 2)] {
        for e1 in [false, true] {
            let mut configured = rules(rule);
            configured.e1 = e1;
            let mut game = Game::from_position(configured, position(Color::White, &pieces));
            for mv in cycle.into_iter().cycle().take(rounds * 4 + 2) {
                assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
            }
            check_transition(
                &game,
                cycle[2],
                GameStatus::Finished(GameResult::Win {
                    winner: Color::White,
                    reason: WinReason::Stalemate,
                }),
            );
        }
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
                let NodeState::Draw { value: actual, .. } = state else {
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
    // R3の合法な2回目の再現にも、対局用の引き分け値を適用する。
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
        NodeState::Draw {
            value: driver.draw_probability,
            approximate: true
        }
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
                approximate: repetition,
            }
            .score();
            assert_eq!(in_repetition_band(score), repetition);
            assert_eq!(score.abs() >= MATE_THRESHOLD, !repetition);
        }
    }
}

#[test]
fn reuse_rejects_ambiguous_keys_even_when_only_one_edge_has_a_child() {
    for children in 1..=2 {
        let mut game = ambiguous_lion_game();
        let moves = ["8h9i8h", "4h5i4h"]
            .map(|text| minase_core::notation::usi::parse(game.position(), text).unwrap());
        let mut input = SearchSnapshot::from_game(&game).unwrap();
        assert!(moves.iter().all(|mv| input.root_moves().contains(mv)));
        input.root_moves = moves.to_vec();
        let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
        advance(&mut driver, &mut UniformEvaluator);
        let config = PuctConfig::default();
        let requests: Vec<_> = (0..children)
            .map(|_| {
                let Step::Evaluate(request) =
                    driver.select_leaf(SelectionRule::Puct(&config)).unwrap()
                else {
                    panic!("each distinct move must have its own child");
                };
                request
            })
            .collect();
        for request in requests {
            evaluate(&mut driver, &mut UniformEvaluator, &request);
        }
        assert_eq!(
            driver
                .tree()
                .root()
                .edges
                .iter()
                .filter(|edge| edge.child.is_some())
                .count(),
            children
        );
        game.play(moves[1]).unwrap();
        let next = SearchSnapshot::from_game(&game).unwrap();
        for mv in moves {
            let mut position = driver.snapshot.position().clone();
            position.make_move_unchecked(mv, next.rules());
            assert_eq!(
                repetition_key(RepetitionRule::R1, &position),
                *next.history_keys().last().unwrap()
            );
        }
        assert!(!driver.reuse(&next, DrawValue::SelfPlay).unwrap());
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
        proven(Outcome::Loss, false, false)
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

// 設計書の終局契約をGameの結果と照合する。根の候補の選別は別の試験で検査する。
fn check_transition(game: &Game, mv: Move, expected: GameStatus) -> Driver {
    let before = game.clone();
    let mut oracle = game.clone();
    assert_eq!(oracle.play(mv), Ok(expected), "fixture move: {mv:?}");
    let mut input = SearchSnapshot::from_game(game).unwrap();
    input.root_moves = vec![mv];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    if matches!(expected, GameStatus::Finished(_)) {
        driver.tree.max_ply = 1;
    }
    advance(&mut driver, &mut UniformEvaluator);
    let evaluated = advance(&mut driver, &mut UniformEvaluator).is_some();
    let child = driver.tree().node(&[0]).unwrap();
    match expected {
        GameStatus::Finished(result) => {
            assert!(
                !evaluated,
                "a terminal child must not request inference: {result:?}"
            );
            assert_eq!(child.value, None);
            assert!(child.edges.is_empty());
            assert!(!child.has_approximate_result());
            match result {
                GameResult::Win { winner, reason } => {
                    let outcome = if winner == oracle.position().side_to_move() {
                        Outcome::Win
                    } else {
                        Outcome::Loss
                    };
                    assert_eq!(
                        child.state,
                        proven(outcome, reason == WinReason::Repetition, false)
                    );
                    assert_eq!(
                        driver.tree().root().edges[0].q(),
                        Some(if winner == game.position().side_to_move() {
                            1.0
                        } else {
                            0.0
                        })
                    );
                    let NodeState::Proven(parent) = driver.tree().root().state else {
                        panic!()
                    };
                    assert_eq!(parent.distance, 1);
                    assert_eq!(
                        parent.outcome,
                        if outcome == Outcome::Win {
                            Outcome::Loss
                        } else {
                            Outcome::Win
                        }
                    );
                }
                GameResult::Draw { .. } => {
                    assert_eq!(
                        child.state,
                        NodeState::Draw {
                            value: 0.5,
                            approximate: false
                        }
                    );
                    assert_eq!(driver.tree().root().edges[0].q(), Some(0.5));
                }
            }
        }
        GameStatus::Ongoing => {
            assert!(
                evaluated || child.has_approximate_result(),
                "ongoing game classified as {:?}",
                child.state
            );
        }
    }
    assert!(before.same_state(game));
    assert!(before.same_state(driver.snapshot.game().unwrap()));
    driver
}

fn win(winner: Color, reason: WinReason) -> GameStatus {
    GameStatus::Finished(GameResult::Win { winner, reason })
}

fn draw(reason: minase_core::DrawReason) -> GameStatus {
    GameStatus::Finished(GameResult::Draw { reason })
}

// 監査の3例。E1は詰みだけを止め、E2は駒枯れだけを止める。
#[test]
fn audit_terminal_examples_follow_the_full_endgame_rules() {
    use minase_core::{DrawReason, ExhaustionRule};
    let cases = [
        (
            position(
                Color::Black,
                &[
                    (sq(0, 0), Color::Black, PieceKind::King),
                    (sq(11, 11), Color::White, PieceKind::King),
                    (sq(4, 4), Color::Black, PieceKind::Rook),
                    (sq(4, 5), Color::White, PieceKind::Pawn),
                ],
            ),
            step(sq(4, 4), sq(4, 5)),
            win(Color::Black, WinReason::PieceExhaustion),
        ),
        (
            position(
                Color::Black,
                &[
                    (sq(0, 0), Color::Black, PieceKind::King),
                    (sq(11, 11), Color::White, PieceKind::King),
                    (sq(0, 1), Color::White, PieceKind::Pawn),
                ],
            ),
            step(sq(0, 0), sq(0, 1)),
            draw(DrawReason::PieceExhaustion),
        ),
        (
            position(
                Color::Black,
                &[
                    (sq(2, 2), Color::Black, PieceKind::King),
                    (sq(0, 0), Color::White, PieceKind::King),
                    (sq(1, 2), Color::Black, PieceKind::FreeKing),
                    (sq(11, 5), Color::White, PieceKind::Pawn),
                ],
            ),
            step(sq(1, 2), sq(1, 1)),
            win(Color::Black, WinReason::Mate),
        ),
    ];
    for (position, mv, result) in cases {
        for e1 in [false, true] {
            for exhaustion in [ExhaustionRule::E0, ExhaustionRule::E2] {
                let configured = Rules {
                    e1,
                    exhaustion,
                    ..Rules::ENGINE_DEFAULT
                };
                let expected = if (result == win(Color::Black, WinReason::Mate) && e1)
                    || (result != win(Color::Black, WinReason::Mate)
                        && exhaustion == ExhaustionRule::E2)
                {
                    GameStatus::Ongoing
                } else {
                    result
                };
                check_transition(
                    &Game::from_position(configured, position.clone()),
                    mv,
                    expected,
                );
            }
        }
    }
}

#[test]
fn royal_capture_handles_the_prince_and_both_steps_of_a_lion_move() {
    use minase_core::PieceCode;
    use minase_core::test_util::position_from_codes;
    let position = position_from_codes(
        Color::Black,
        &[
            (
                sq(0, 0),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            ),
            (
                sq(5, 5),
                PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
            ),
            (
                sq(5, 6),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            ),
            (
                sq(6, 6),
                PieceCode::new_promoted(Color::White, PieceKind::CrownPrince).unwrap(),
            ),
        ],
    );
    let game = Game::from_position(Rules::ENGINE_DEFAULT, position);
    for to in [sq(5, 6), sq(6, 6)] {
        check_transition(&game, step(sq(5, 5), to), GameStatus::Ongoing);
    }
    // 王駒捕獲と駒枯れがともに成立しても、捕獲を理由とする。
    check_transition(
        &game,
        Move {
            from: sq(5, 5),
            mid: Some(sq(5, 6)),
            to: sq(6, 6),
            promote: false,
        },
        win(Color::Black, WinReason::RoyalCapture),
    );
}

fn grace_game() -> (Game, Move) {
    (
        Game::from_position(
            Rules::ENGINE_DEFAULT,
            position(
                Color::Black,
                &[
                    (sq(0, 0), Color::Black, PieceKind::King),
                    (sq(5, 5), Color::Black, PieceKind::GoldGeneral),
                    (sq(4, 7), Color::White, PieceKind::King),
                    (sq(5, 6), Color::White, PieceKind::Pawn),
                ],
            ),
        ),
        step(sq(5, 5), sq(5, 6)),
    )
}

#[test]
fn grace_is_carried_from_the_root_and_isolated_between_branches() {
    use minase_core::DrawReason;
    let (mut game, establish) = grace_game();
    let driver = check_transition(&game, establish, GameStatus::Ongoing);
    let before = game.clone();
    let node = driver.tree().node(&[0]).unwrap();
    for _ in 0..2 {
        for (mv, result) in [
            (
                step(sq(4, 7), sq(3, 7)),
                win(Color::Black, WinReason::PieceExhaustion),
            ),
            (step(sq(4, 7), sq(5, 6)), draw(DrawReason::PieceExhaustion)),
        ] {
            let index = node.edges.iter().position(|edge| edge.mv == mv).unwrap();
            let (_, moves, state) = driver.classify_child(&[0, index]).unwrap();
            assert!(moves.is_empty());
            assert_eq!(
                state,
                if matches!(result, GameStatus::Finished(GameResult::Win { .. })) {
                    proven(Outcome::Win, false, false)
                } else {
                    NodeState::Draw {
                        value: 0.5,
                        approximate: false,
                    }
                }
            );
            assert!(before.same_state(driver.snapshot.game().unwrap()));
        }
    }
    assert_eq!(game.play(establish), Ok(GameStatus::Ongoing));
    // 裸側の根で同じ猶予を引き継ぎ、捕獲しない応手は敗北となる。
    check_transition(
        &game,
        step(sq(4, 7), sq(3, 7)),
        win(Color::Black, WinReason::PieceExhaustion),
    );
    check_transition(
        &game,
        step(sq(4, 7), sq(5, 6)),
        draw(DrawReason::PieceExhaustion),
    );
    // 余分な駒を取っても王駒同士が隣接していれば、対局は継続する。
    let mut adjacent = Game::from_position(
        Rules::ENGINE_DEFAULT,
        position(
            Color::Black,
            &[
                (sq(4, 5), Color::Black, PieceKind::King),
                (sq(5, 5), Color::Black, PieceKind::GoldGeneral),
                (sq(4, 7), Color::White, PieceKind::King),
                (sq(5, 6), Color::White, PieceKind::Pawn),
            ],
        ),
    );
    check_transition(&adjacent, establish, GameStatus::Ongoing);
    adjacent.play(establish).unwrap();
    check_transition(&adjacent, step(sq(4, 7), sq(5, 6)), GameStatus::Ongoing);
}

#[test]
fn material_counts_dead_pieces_unpromoted_pieces_and_capturable_bare_kings() {
    for kind in [PieceKind::Pawn, PieceKind::Lance] {
        for (color, rank) in [(Color::Black, 11), (Color::White, 0)] {
            let game = Game::from_position(
                Rules::ENGINE_DEFAULT,
                position(
                    Color::Black,
                    &[
                        (sq(2, 2), Color::Black, PieceKind::King),
                        (sq(9, 9), Color::White, PieceKind::King),
                        (sq(5, 5), Color::Black, PieceKind::GoldGeneral),
                        (sq(0, rank), color, kind),
                    ],
                ),
            );
            check_transition(
                &game,
                step(sq(2, 2), sq(2, 3)),
                win(Color::Black, WinReason::PieceExhaustion),
            );
        }
    }
    for kind in [PieceKind::Pawn, PieceKind::GoBetween] {
        let game = Game::from_position(
            Rules::ENGINE_DEFAULT,
            position(
                Color::Black,
                &[
                    (sq(0, 0), Color::Black, PieceKind::King),
                    (sq(11, 11), Color::White, PieceKind::King),
                    (sq(4, 4), Color::Black, kind),
                ],
            ),
        );
        check_transition(&game, step(sq(4, 4), sq(4, 5)), GameStatus::Ongoing);
    }
    for exhaustion in [
        minase_core::ExhaustionRule::E0,
        minase_core::ExhaustionRule::E3,
    ] {
        let game = Game::from_position(
            Rules {
                exhaustion,
                ..Rules::ENGINE_DEFAULT
            },
            position(
                Color::Black,
                &[
                    (sq(4, 4), Color::Black, PieceKind::King),
                    (sq(4, 6), Color::White, PieceKind::King),
                ],
            ),
        );
        check_transition(&game, step(sq(4, 4), sq(4, 5)), GameStatus::Ongoing);
    }
}

#[test]
fn bare_king_wins_and_draws_are_exact() {
    use minase_core::{DrawReason, ExhaustionRule};
    let configured = Rules {
        exhaustion: ExhaustionRule::E3,
        ..Rules::ENGINE_DEFAULT
    };
    let game = Game::from_position(
        configured,
        position(
            Color::Black,
            &[
                (sq(5, 4), Color::Black, PieceKind::King),
                (sq(9, 9), Color::White, PieceKind::Rook),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    );
    check_transition(
        &game,
        step(sq(5, 4), sq(5, 5)),
        win(Color::White, WinReason::BareKing),
    );
    let game = Game::from_position(
        configured,
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    );
    check_transition(&game, step(sq(0, 0), sq(0, 1)), draw(DrawReason::BareKing));
}

#[test]
fn exact_r1_wins_losses_and_draws_precede_the_cycle_approximation() {
    use minase_core::DrawReason;
    for attacks in [[true, false], [false, true], [false, false], [true, true]] {
        let (start, cycle) = rook_cycle(attacks[0], attacks[1]);
        let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start);
        for ply in 0..12 {
            let mv = cycle[ply % 4];
            let expected = if ply < 11 {
                GameStatus::Ongoing
            } else {
                match attacks {
                    [true, false] => win(Color::White, WinReason::Repetition),
                    [false, true] => win(Color::Black, WinReason::Repetition),
                    _ => draw(DrawReason::Repetition),
                }
            };
            if ply >= 10 {
                check_transition(&game, mv, expected);
            }
            if ply == 11 && attacks == [false, true] {
                // 白の自滅は根から除くが、Gameでは受理して黒の勝ちにする。
                let input = SearchSnapshot::from_game(&game).unwrap();
                assert!(!input.root_moves().contains(&mv));
                assert!(game.repetition_root_moves().losing().contains(&mv));
            }
            assert_eq!(game.play(mv), Ok(expected));
        }
    }
}

#[test]
fn exact_terminal_and_approximate_repetition_precede_the_depth_limit() {
    let (game, establish) = grace_game();
    let mut driver = check_transition(&game, establish, GameStatus::Ongoing);
    driver.tree.max_ply = 2;
    let index = driver
        .tree()
        .node(&[0])
        .unwrap()
        .edges
        .iter()
        .position(|edge| edge.mv == step(sq(4, 7), sq(3, 7)))
        .unwrap();
    assert_eq!(
        driver.classify_child(&[0, index]).unwrap().2,
        proven(Outcome::Win, false, false)
    );
    let (start, cycle) = rook_cycle(false, false);
    let mut driver = Driver::new(snapshot(&start), CAPACITY, DrawValue::SelfPlay).unwrap();
    driver.tree.max_ply = 4;
    let mut evaluator = ScriptedEvaluator::line(&start, &cycle);
    for _ in 0..4 {
        advance(&mut driver, &mut evaluator);
    }
    assert_eq!(advance(&mut driver, &mut evaluator), None);
    assert!(
        driver
            .tree()
            .node(&path(&driver, &cycle))
            .unwrap()
            .has_approximate_result()
    );
}

#[test]
fn exact_draws_use_the_configured_probability_from_either_side() {
    use minase_core::ExhaustionRule;
    for exhaustion in [ExhaustionRule::E0, ExhaustionRule::E3] {
        let (mut game, establish) = grace_game();
        if exhaustion == ExhaustionRule::E3 {
            game = Game::from_position(
                Rules {
                    exhaustion,
                    ..Rules::ENGINE_DEFAULT
                },
                game.position().clone(),
            );
        }
        for prefix in [false, true] {
            let mut root = game.clone();
            if prefix {
                root.play(establish).unwrap();
            }
            let line = if prefix {
                vec![step(sq(4, 7), sq(5, 6))]
            } else {
                vec![establish, step(sq(4, 7), sq(5, 6))]
            };
            for cp in [-1200, 1200] {
                for draw_value in [
                    DrawValue::SelfPlay,
                    DrawValue::Game {
                        cp_per_logit: SCALE,
                    },
                ] {
                    let input = SearchSnapshot::from_game(&root)
                        .unwrap()
                        .with_draw_value(cp)
                        .unwrap();
                    let mut driver = Driver::new(input, CAPACITY, draw_value).unwrap();
                    let mut evaluator = ScriptedEvaluator::line(root.position(), &line);
                    for _ in 0..line.len() {
                        advance(&mut driver, &mut evaluator);
                    }
                    assert_eq!(advance(&mut driver, &mut evaluator), None);
                    let expected = if prefix {
                        1.0 - driver.draw_probability
                    } else {
                        driver.draw_probability
                    };
                    assert_eq!(
                        driver.tree().node(&path(&driver, &line)).unwrap().state,
                        NodeState::Draw {
                            value: expected,
                            approximate: false
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn seeded_games_match_the_referee_without_terminal_inference() {
    use minase_core::rng::XorShift64;
    use std::num::{NonZeroU64, NonZeroUsize};
    let mut checked = 0;
    let mut terminal = 0;
    for seed in [1, 17, 123, 20261028] {
        let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
        let mut game = Game::with_default_rules();
        for _ in 0..600 {
            let moves = game.legal_moves();
            let mv = moves[rng.index(NonZeroUsize::new(moves.len()).unwrap())];
            let mut next = game.clone();
            let expected = next.play(mv).unwrap();
            if SearchSnapshot::from_game(&game)
                .unwrap()
                .root_moves()
                .contains(&mv)
            {
                check_transition(&game, mv, expected);
                checked += 1;
            }
            game = next;
            if matches!(expected, GameStatus::Finished(_)) {
                terminal += 1;
                break;
            }
        }
    }
    assert!(checked >= 600, "only {checked} plies checked");
    assert!(terminal > 0, "random coverage must include a terminal move");
}

#[test]
fn reuse_compares_full_rules_and_the_draw_perspective() {
    let game = Game::with_default_rules();
    let mut driver = Driver::new(
        SearchSnapshot::from_game(&game).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    assert!(
        driver
            .reuse(
                &SearchSnapshot::from_game(&game).unwrap(),
                DrawValue::SelfPlay
            )
            .unwrap()
    );
    advance(&mut driver, &mut UniformEvaluator);
    for configured in [
        Rules {
            e1: true,
            ..game.rules()
        },
        Rules {
            exhaustion: minase_core::ExhaustionRule::E2,
            ..game.rules()
        },
    ] {
        let different = Game::from_position(configured, game.position().clone());
        assert!(
            !driver
                .reuse(
                    &SearchSnapshot::from_game(&different).unwrap(),
                    DrawValue::SelfPlay
                )
                .unwrap()
        );
    }
    let draw_value = DrawValue::Game {
        cp_per_logit: SCALE,
    };
    let input = SearchSnapshot::from_game(&game)
        .unwrap()
        .with_draw_value(1200)
        .unwrap();
    let mut driver = Driver::new(input, CAPACITY, draw_value).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    let path = advance(&mut driver, &mut UniformEvaluator).unwrap();
    let mv = driver.tree().root().edges[path[0]].mv;
    let mut next = game.clone();
    next.play(mv).unwrap();
    let input = SearchSnapshot::from_game(&next)
        .unwrap()
        .with_draw_value(1200)
        .unwrap();
    assert!(!driver.reuse(&input, draw_value).unwrap());
}

#[test]
fn snapshots_without_a_game_are_rejected_by_mcts() {
    let source = initial();
    let synthetic = SearchSnapshot::from_parts(
        source.position.clone(),
        source.rules,
        source.repetition,
        source.history_keys.clone(),
        source.root_moves.clone(),
    )
    .unwrap();
    assert!(matches!(
        Driver::new(synthetic, CAPACITY, DrawValue::SelfPlay),
        Err(Error::MissingGame)
    ));
}

#[test]
fn illegal_replay_returns_the_original_error_and_cancels_virtual_loss() {
    use minase_core::{GameError, IllegalMoveCause};
    use std::error::Error as _;
    let invalid = step(sq(5, 5), sq(5, 6));
    let mut input = initial();
    input.root_moves = vec![invalid];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    let expected = GameError::IllegalMove {
        mv: invalid,
        cause: IllegalMoveCause::Movement,
    };
    for _ in 0..2 {
        let error = match driver.select_leaf(SelectionRule::Puct(&PuctConfig::default())) {
            Err(error) => error,
            _ => panic!(),
        };
        assert_eq!(
            error.source().unwrap().downcast_ref::<GameError>(),
            Some(&expected)
        );
        assert_eq!(
            error,
            Error::Replay {
                path: vec![invalid],
                ply: 1,
                source: expected
            }
        );
        assert_eq!(driver.simulations(), 1);
        assert_eq!(driver.tree().root().edges[0].virtual_loss, 0);
        assert!(driver.tree().root().edges[0].child.is_none());
    }
}

#[test]
fn an_existing_path_cannot_continue_after_an_exact_terminal() {
    let game = Game::from_position(
        Rules::ENGINE_DEFAULT,
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(0, 1), Color::White, PieceKind::Pawn),
            ],
        ),
    );
    let first = step(sq(0, 0), sq(0, 1));
    let second = step(sq(11, 11), sq(10, 11));
    let mut input = SearchSnapshot::from_game(&game).unwrap();
    input.root_moves = vec![first];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    let config = PuctConfig::default();
    // 既存の子を故意に継続扱いにして、不整合な木を作る。
    let selection = driver
        .tree
        .select_leaf(SelectionRule::Puct(&config))
        .unwrap();
    driver
        .tree
        .create_child(&selection.path, Child::Pending(&[second]))
        .unwrap();
    driver.tree.evaluate(&selection.path, &[0.0], 0.5).unwrap();
    let error = match driver.select_leaf(SelectionRule::Puct(&config)) {
        Err(error) => error,
        _ => panic!(),
    };
    assert_eq!(
        error,
        Error::PrematureTerminal {
            path: vec![first, second],
            ply: 1,
            result: GameResult::Draw {
                reason: minase_core::DrawReason::PieceExhaustion
            }
        }
    );
    assert_eq!(driver.tree().root().edges[0].virtual_loss, 0);
    assert_eq!(driver.tree().node(&[0]).unwrap().edges[0].virtual_loss, 0);
}

// 第21条第3項c・d: 正確なR1の勝利と引き分けだけが詰みの回避手となる。
#[test]
fn exact_repetition_can_rescue_mate_but_a_fresh_position_cannot() {
    for drawn in [false, true] {
        let mut pieces = vec![
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(1, 8), Color::Black, PieceKind::Rook),
            (sq(3, 9), Color::Black, PieceKind::Rook),
            (sq(2, 9), Color::Black, PieceKind::Pawn),
            (sq(0, 5), Color::White, PieceKind::Rook),
            (sq(2, 1), Color::White, PieceKind::GoldGeneral),
            (sq(1, 11), Color::White, PieceKind::Pawn),
            (sq(7, 10), Color::White, PieceKind::Pawn),
            (sq(11, 0), Color::White, PieceKind::King),
        ];
        if drawn {
            pieces.push((sq(7, 9), Color::White, PieceKind::Pawn));
        }
        let mut game = Game::from_position(Rules::ENGINE_DEFAULT, position(Color::White, &pieces));
        let w_out = step(sq(0, 5), sq(2, 5));
        let w_back = step(sq(2, 5), sq(0, 5));
        let z_out = step(sq(1, 8), sq(1, 2));
        let z_back = step(sq(1, 2), sq(1, 8));
        let y_out = step(sq(3, 9), sq(3, 10));
        for mv in [
            w_out, z_out, w_back, z_back, w_out, z_out, w_back, z_back, w_out, y_out,
        ] {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }
        let fresh = Game::from_position(game.rules(), game.position().clone());
        check_transition(&fresh, w_back, win(Color::White, WinReason::Mate));
        check_transition(&game, w_back, GameStatus::Ongoing);
        game.play(w_back).unwrap();
        check_transition(
            &game,
            step(sq(3, 10), sq(3, 9)),
            if drawn {
                draw(minase_core::DrawReason::Repetition)
            } else {
                win(Color::Black, WinReason::Repetition)
            },
        );
    }
}

// 猶予の開始時は、守られた駒の捕獲でも認めるため、詰みを先に裁定しない。
#[test]
fn exhaustion_grace_precedes_mate_and_its_consumption_precedes_capture_search() {
    use minase_core::PieceCode;
    use minase_core::test_util::position_from_codes;
    let game = Game::from_position(
        Rules::ENGINE_DEFAULT,
        position_from_codes(
            Color::Black,
            &[
                (
                    sq(0, 0),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
                (
                    sq(1, 2),
                    PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                ),
                (
                    sq(1, 1),
                    PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap(),
                ),
                (
                    sq(0, 1),
                    PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
                ),
            ],
        ),
    );
    let mv = step(sq(1, 2), sq(0, 1));
    check_transition(&game, mv, GameStatus::Ongoing);
    let mut next = game.clone();
    next.play(mv).unwrap();
    check_transition(
        &next,
        step(sq(0, 0), sq(1, 0)),
        win(Color::Black, WinReason::PieceExhaustion),
    );
    check_transition(&next, step(sq(0, 0), sq(0, 1)), GameStatus::Ongoing);
}

// 2手周期のじっとでは4回目の出現が6手目に来るが、R1の12可逆手条件を満たすまで裁定しない。
#[test]
fn fourth_occurrence_at_eleven_reversible_plies_is_still_only_approximate() {
    let mut game = Game::from_position(
        Rules::ENGINE_DEFAULT,
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(3, 3), Color::Black, PieceKind::Lion),
                (sq(8, 8), Color::White, PieceKind::Lion),
                (sq(11, 11), Color::White, PieceKind::King),
            ],
        ),
    );
    let cycle = [step(sq(3, 3), sq(3, 3)), step(sq(8, 8), sq(8, 8))];
    for ply in 0..10 {
        assert_eq!(game.play(cycle[ply % 2]), Ok(GameStatus::Ongoing));
    }
    let driver = check_transition(&game, cycle[0], GameStatus::Ongoing);
    assert!(driver.tree().node(&[0]).unwrap().has_approximate_result());
    assert_eq!(game.play(cycle[0]), Ok(GameStatus::Ongoing));
    check_transition(&game, cycle[1], draw(minase_core::DrawReason::Repetition));
}

#[test]
fn sibling_replays_do_not_share_repetition_history() {
    let (start, cycle) = rook_cycle(false, false);
    let mut game = Game::from_position(Rules::ENGINE_DEFAULT, start);
    for &mv in &cycle[..3] {
        game.play(mv).unwrap();
    }
    let before = game.clone();
    let driver = Driver::new(
        SearchSnapshot::from_game(&game).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    let back = driver
        .tree()
        .root()
        .edges
        .iter()
        .position(|edge| edge.mv == cycle[3])
        .unwrap();
    let other = driver
        .tree()
        .root()
        .edges
        .iter()
        .position(|edge| edge.mv == step(sq(8, 7), sq(7, 7)))
        .unwrap();
    for _ in 0..2 {
        assert_eq!(
            driver.classify_child(&[back]).unwrap().2,
            NodeState::Draw {
                value: 0.5,
                approximate: true
            }
        );
        assert_eq!(
            driver.classify_child(&[other]).unwrap().2,
            NodeState::Pending { unexpanded: false }
        );
        assert!(before.same_state(driver.snapshot.game().unwrap()));
    }
}

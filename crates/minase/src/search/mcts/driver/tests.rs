use super::*;
use crate::nn::test_util::UniformEvaluator;
use crate::search::mcts::gumbel::Gumbel;
use crate::search::mcts::puct::PuctConfig;
use crate::search::mcts::test_support::*;
use crate::search::{MATE, MATE_THRESHOLD, MAX_PLY, REPETITION_WIN, in_repetition_band};
use minase_core::test_util::{position, sq};
use minase_core::{Game, GameStatus, PieceKind, PositionBuilder, RepetitionRule, Rules, Square};

const WIN_IN_ONE: NodeState = NodeState::Proven(Proof {
    outcome: Outcome::Win,
    distance: 1,
    repetition: false,
    approximate: false,
});

// 第21条第1項・第5項の神託。補題を呼ばず、審判が受理した捕獲手の結果を調べる。
fn has_royal_capture(game: &Game) -> bool {
    game.legal_moves().into_iter().any(|mv| {
        game.clone().play(mv).unwrap()
            == win(game.position().side_to_move(), WinReason::RoyalCapture)
    })
}

// 黒王の合法手は3手で、(1, 1)へ進む手だけが白角に取られる（第8条）。
fn exposed_king_game() -> (Game, Move) {
    (
        Game::from_position(
            Rules {
                e1: true,
                ..rules(RepetitionRule::R1)
            },
            position(
                Color::Black,
                &[
                    (sq(0, 0), Color::Black, PieceKind::King),
                    (sq(3, 3), Color::White, PieceKind::Bishop),
                    (sq(11, 11), Color::White, PieceKind::King),
                ],
            ),
        ),
        step(sq(0, 0), sq(1, 1)),
    )
}

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

// 第20条・第21条第5項: 2枚の王駒を別々に取れるだけでは1手の勝ちにならない。
#[test]
fn royal_capture_in_one_requires_both_royals_in_a_single_lion_move() {
    use minase_core::PieceCode;
    use minase_core::test_util::position_from_codes;
    for (king, prince, expected) in [(sq(5, 6), sq(6, 6), true), (sq(4, 7), sq(6, 7), false)] {
        let game = Game::from_position(
            Rules {
                e1: true,
                ..rules(RepetitionRule::R1)
            },
            position_from_codes(
                Color::White,
                &[
                    (
                        sq(0, 0),
                        PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                    ),
                    (
                        sq(5, 5),
                        PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
                    ),
                    (king, PieceCode::new(Color::White, PieceKind::King).unwrap()),
                    (
                        prince,
                        PieceCode::new_promoted(Color::White, PieceKind::CrownPrince).unwrap(),
                    ),
                    (
                        sq(11, 6),
                        PieceCode::new(Color::White, PieceKind::Pawn).unwrap(),
                    ),
                ],
            ),
        );
        let mv = step(sq(11, 6), sq(11, 5));
        let mut next = game.clone();
        assert_eq!(next.play(mv), Ok(GameStatus::Ongoing));
        assert_eq!(has_royal_capture(&next), expected);
        let driver = check_transition(&game, mv, GameStatus::Ongoing);
        let child = driver.tree().node(&[0]).unwrap();
        assert_eq!(child.state == WIN_IN_ONE, expected);
        if expected {
            assert!(child.edges.is_empty());
            assert_eq!(child.value, None);
            assert_eq!(driver.score(SCALE).unwrap(), -(MATE - 2));
        } else {
            assert_eq!(child.state, NodeState::Expanded);
        }
    }
}

#[test]
fn royal_capture_in_one_precedes_a_repetition_loss() {
    let mut game = Game::from_position(
        Rules {
            e1: true,
            ..rules(RepetitionRule::R1)
        },
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(3, 3), Color::Black, PieceKind::Rook),
                (sq(8, 8), Color::White, PieceKind::Rook),
                (sq(3, 9), Color::White, PieceKind::King),
            ],
        ),
    );
    let cycle = [
        step(sq(3, 3), sq(3, 4)),
        step(sq(8, 8), sq(8, 7)),
        step(sq(3, 4), sq(3, 3)),
        step(sq(8, 7), sq(8, 8)),
    ];
    for &mv in &cycle[..3] {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }
    let mut next = game.clone();
    assert_eq!(next.play(cycle[3]), Ok(GameStatus::Ongoing));
    assert!(has_royal_capture(&next));
    let driver = check_transition(&game, cycle[3], GameStatus::Ongoing);
    // この局面には実際に反復の近似が適用され、黒の連続王手による負けになる。
    assert_eq!(
        driver.repetition(
            next.position(),
            &[repetition_key(RepetitionRule::R1, game.position())],
            &cycle[3..],
        ),
        Some(proven(Outcome::Loss, true, true))
    );
    assert_eq!(driver.tree().node(&[0]).unwrap().state, WIN_IN_ONE);
    assert!(!driver.tree().root().has_approximate_result());
    assert_eq!(driver.score(SCALE).unwrap(), -(MATE - 2));
}

#[test]
fn royal_capture_in_one_at_the_depth_limit_still_requests_evaluation() {
    let (game, mv) = exposed_king_game();
    let mut next = game.clone();
    assert_eq!(next.play(mv), Ok(GameStatus::Ongoing));
    assert!(has_royal_capture(&next));
    let mut input = SearchSnapshot::from_game(&game).unwrap();
    input.root_moves = vec![mv];
    let mut driver = Driver::new(input, CAPACITY, DrawValue::SelfPlay).unwrap();
    driver.tree.depth_limit = 1;
    advance(&mut driver, &mut UniformEvaluator);
    let Step::Evaluate(request) = driver
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap()
    else {
        panic!("the depth-limit leaf must request evaluation even with a royal capture");
    };
    assert_eq!(request.path, [0]);
    assert_eq!(request.query().position, next.position());
    assert!(request.query().moves.is_empty());
    assert_eq!(
        driver.tree().node(&[0]).unwrap().state,
        NodeState::Pending { unexpanded: true }
    );
    evaluate(&mut driver, &mut UniformEvaluator, &request);
    let child = driver.tree().node(&[0]).unwrap();
    assert_eq!(child.state, NodeState::Unexpanded);
    assert_eq!(child.value, Some(0.5));
    assert!(child.edges.is_empty());
    assert_eq!(driver.tree().root().state, NodeState::Expanded);
}

#[test]
fn puct_never_revisits_the_only_move_allowing_royal_capture() {
    let (game, losing) = exposed_king_game();
    let legal = game.legal_moves();
    let capturable: Vec<_> = legal
        .iter()
        .copied()
        .filter(|&mv| {
            let mut next = game.clone();
            assert_eq!(next.play(mv), Ok(GameStatus::Ongoing));
            has_royal_capture(&next)
        })
        .collect();
    assert_eq!(legal.len(), 3);
    assert_eq!(capturable, [losing]);
    let mut driver = Driver::new(
        SearchSnapshot::from_game(&game).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    let index = driver
        .tree()
        .root()
        .edges
        .iter()
        .position(|edge| edge.mv == losing)
        .unwrap();
    // 一様な方策・価値で全3辺を1回ずつ訪問する。
    for _ in 0..=legal.len() {
        advance(&mut driver, &mut UniformEvaluator);
    }
    assert_eq!(driver.tree().root().edges[index].visits, 1);
    for _ in 0..64 {
        advance(&mut driver, &mut UniformEvaluator);
        assert_eq!(
            driver.tree().root().edges[index].visits,
            1,
            "PUCT revisited the move allowing immediate royal capture"
        );
    }
    let edge = &driver.tree().root().edges[index];
    assert_eq!(edge.child.as_ref().unwrap().state, WIN_IN_ONE);
    assert_eq!(edge.child.as_ref().unwrap().value, None);
    assert_eq!(edge.q(), Some(0.0));
    assert_ne!(driver.tree().best_move(), Some(losing));
}

#[test]
fn puct_returns_a_move_when_every_move_allows_royal_capture() {
    let game = Game::from_position(
        Rules {
            e1: true,
            ..rules(RepetitionRule::R1)
        },
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
    let legal = game.legal_moves();
    assert_eq!(legal.len(), 3);
    for &mv in &legal {
        let mut next = game.clone();
        assert_eq!(next.play(mv), Ok(GameStatus::Ongoing));
        assert!(has_royal_capture(&next));
    }
    let mut driver = Driver::new(
        SearchSnapshot::from_game(&game).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    advance(&mut driver, &mut UniformEvaluator);
    for _ in &legal {
        assert_eq!(advance(&mut driver, &mut UniformEvaluator), None);
    }
    assert_eq!(
        driver.tree().root().state,
        NodeState::Proven(Proof {
            outcome: Outcome::Loss,
            distance: 2,
            repetition: false,
            approximate: false,
        })
    );
    assert_eq!(driver.score(SCALE).unwrap(), -(MATE - 2));
    assert!(legal.contains(&driver.tree().best_move().unwrap()));
    for edge in &driver.tree().root().edges {
        assert_eq!(edge.visits, 1);
        assert_eq!(edge.child.as_ref().unwrap().state, WIN_IN_ONE);
        assert_eq!(edge.q(), Some(0.0));
    }
    // 根が確定した後も、選べる辺がないというエラーにはしない。
    assert_eq!(advance(&mut driver, &mut UniformEvaluator), None);
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
            root.make_move_unchecked(cycle[offset], Rules::JCSA.moves);
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

fn promotion_game(codes: &str, kind: PieceKind, extra: &[(Square, Color, PieceKind)]) -> Game {
    let rules = Rules::from_codes(&minase_core::rules::parse_rule_set(codes).unwrap()).unwrap();
    let mut pieces = vec![
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
        (sq(10, 6), Color::White, PieceKind::GoBetween),
        (sq(5, 7), Color::Black, kind),
    ];
    pieces.extend_from_slice(extra);
    Game::from_position(rules, position(Color::Black, &pieces))
}

fn promotion_line() -> [Move; 6] {
    [
        step(sq(5, 7), sq(5, 8)),
        step(sq(10, 6), sq(10, 5)),
        step(sq(5, 8), sq(5, 9)),
        step(sq(10, 5), sq(10, 6)),
        step(sq(5, 9), sq(5, 10)),
        step(sq(10, 6), sq(10, 5)),
    ]
}

fn expand_line(game: &Game, line: &[Move]) -> (Driver, Position) {
    let mut driver = Driver::new(
        SearchSnapshot::from_game(game).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    let mut evaluator =
        ScriptedEvaluator::line_with_rules(game.position(), line, game.rules().moves);
    for ply in 0..=line.len() {
        let selected = advance(&mut driver, &mut evaluator).unwrap();
        assert_eq!(selected, path(&driver, &line[..ply]));
    }
    (driver, evaluator.seen.pop().unwrap())
}

fn assert_promotion_moves(moves: &[Move], mv: Move, unpromoted: bool, promoted: bool) {
    assert_eq!(moves.contains(&mv), unpromoted, "{mv:?}");
    let promotion = Move {
        promote: true,
        ..mv
    };
    assert_eq!(moves.contains(&promotion), promoted, "{promotion:?}");
}

fn assert_tree_and_root_promotion(
    game: &Game,
    line: &[Move],
    mv: Move,
    unpromoted: bool,
    promoted: bool,
) {
    let (driver, leaf_position) = expand_line(game, line);
    let mut replayed = game.clone();
    for &mv in line {
        assert_eq!(replayed.play(mv), Ok(GameStatus::Ongoing));
    }
    assert_eq!(&leaf_position, replayed.position());
    let expected = replayed.legal_moves();
    let node = driver.tree().node(&path(&driver, line)).unwrap();
    let moves: Vec<_> = node.edges.iter().map(|edge| edge.mv).collect();
    assert_eq!(moves, expected);
    assert_promotion_moves(&moves, mv, unpromoted, promoted);

    let mut root = Driver::new(
        SearchSnapshot::from_game(&replayed).unwrap(),
        CAPACITY,
        DrawValue::SelfPlay,
    )
    .unwrap();
    let Step::Evaluate(request) = root
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap()
    else {
        panic!("the root requires evaluation")
    };
    assert_eq!(request.query().moves, expected);
    assert_promotion_moves(request.query().moves, mv, unpromoted, promoted);
}

// 第30条P1: 回復には、不成を選んだ駒自身の追加の1回の移動が要る。
#[test]
fn article_30_p1_tree_and_root_restore_promotion_after_the_piece_moves_once_more() {
    let game = promotion_game("L0,P1,R1,E2", PieceKind::FerociousLeopard, &[]);
    let line = promotion_line();
    assert_tree_and_root_promotion(&game, &line[..2], step(sq(5, 8), sq(5, 9)), true, false);
    assert_tree_and_root_promotion(&game, &line[..4], step(sq(5, 9), sq(5, 10)), true, true);
    let other_piece = [line[0], line[1], step(sq(0, 0), sq(0, 1)), line[3]];
    assert_tree_and_root_promotion(&game, &other_piece, step(sq(5, 8), sq(5, 9)), true, false);
}

// 第30条P2: 直後の自分の1手番が過ぎれば、別の駒を動かした場合も待機は満了する。
#[test]
fn article_30_p2_tree_and_root_block_quiet_promotion_for_one_own_turn() {
    let game = promotion_game("L0,P2,R1,E2", PieceKind::FerociousLeopard, &[]);
    let line = promotion_line();
    assert_tree_and_root_promotion(&game, &line[..2], step(sq(5, 8), sq(5, 9)), true, false);
    assert_tree_and_root_promotion(&game, &line[..4], step(sq(5, 9), sq(5, 10)), true, true);
    let other_piece = [line[0], line[1], step(sq(0, 0), sq(0, 1)), line[3]];
    assert_tree_and_root_promotion(&game, &other_piece, step(sq(5, 8), sq(5, 9)), true, true);
}

// 第30条P5: 保留した歩兵は、併用する成り規則で成れる最奥段への着手だけが強制成りとなる。
#[test]
fn article_30_p5_tree_and_root_limit_a_deferred_pawn_to_forced_last_rank_promotion() {
    let line = promotion_line();
    // 第18条第2項aの捕獲でも、最奥段以外への成りはP5が禁じる。
    let capture = promotion_game(
        "L0,P0,P5,R1,E2",
        PieceKind::Pawn,
        &[(sq(5, 9), Color::White, PieceKind::Pawn)],
    );
    assert_tree_and_root_promotion(&capture, &line[..2], step(sq(5, 8), sq(5, 9)), true, false);
    // P5では、第19条第1項の非捕獲による成りの機会がない。
    let quiet = promotion_game("L0,P0,P5,R1,E2", PieceKind::Pawn, &[]);
    assert_tree_and_root_promotion(&quiet, &line, step(sq(5, 10), sq(5, 11)), true, false);
    let last_rank_capture = promotion_game(
        "L0,P0,P5,R1,E2",
        PieceKind::Pawn,
        &[(sq(5, 11), Color::White, PieceKind::Lance)],
    );
    assert_tree_and_root_promotion(
        &last_rank_capture,
        &line,
        step(sq(5, 10), sq(5, 11)),
        false,
        true,
    );
    let p2 = promotion_game("L0,P2,P5,R1,E2", PieceKind::Pawn, &[]);
    assert_tree_and_root_promotion(&p2, &line, step(sq(5, 10), sq(5, 11)), false, true);
}

// 駆動部の契約: 同じ159面でも、辺は各対局の審判層に従う。
// 第18条第2項a・第5項と第30条P5により、最奥段での捕獲は保留の有無で選択が異なる。
#[test]
fn positions_differing_only_in_deferred_promotion_expand_to_their_own_legal_moves() {
    let mut game_x = promotion_game(
        "L0,P0,P5,R1,E2",
        PieceKind::Pawn,
        &[(sq(5, 11), Color::White, PieceKind::Lance)],
    );
    let mut game_y = Game::from_position(
        game_x.rules(),
        position(
            Color::Black,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(10, 5), Color::White, PieceKind::GoBetween),
                (sq(5, 8), Color::Black, PieceKind::Pawn),
                (sq(5, 11), Color::White, PieceKind::Lance),
            ],
        ),
    );
    let line = promotion_line();
    let (driver_x, leaf_x) = expand_line(&game_x, &line);
    let (driver_y, leaf_y) = expand_line(&game_y, &line[2..]);
    assert_ne!(leaf_x, leaf_y);
    assert!(leaf_x.promotion_deferred().contains(sq(5, 10)));
    assert!(leaf_y.promotion_deferred().is_empty());
    let mut input_x = vec![0; crate::nn::INPUT_SIZE];
    let mut input_y = vec![0; crate::nn::INPUT_SIZE];
    crate::nn::encode(&leaf_x, &mut input_x);
    crate::nn::encode(&leaf_y, &mut input_y);
    assert_eq!(input_x, input_y);

    for (game, driver, leaf, moves, unpromoted) in [
        (&mut game_x, &driver_x, &leaf_x, line.as_slice(), false),
        (&mut game_y, &driver_y, &leaf_y, &line[2..], true),
    ] {
        for &mv in moves {
            assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        }
        assert_eq!(leaf, game.position());
        let node = driver.tree().node(&path(driver, moves)).unwrap();
        let edges: Vec<_> = node.edges.iter().map(|edge| edge.mv).collect();
        assert_eq!(edges, game.legal_moves());
        assert_promotion_moves(&edges, step(sq(5, 10), sq(5, 11)), unpromoted, true);
    }
}

// [`Driver::reuse`]の契約: 第31条R2のキー列が同じでも、保留を含む局面の一致を要する。
#[test]
fn reuse_requires_the_same_deferred_promotion_state() {
    let rules =
        Rules::from_codes(&minase_core::rules::parse_rule_set("L1,L3,P0,P5,P6,R2,E1,E2").unwrap())
            .unwrap();
    let without_deferred = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(10, 5), Color::White, PieceKind::GoBetween),
            (sq(5, 8), Color::Black, PieceKind::Pawn),
        ],
    );
    let mut builder = PositionBuilder::new(Color::Black);
    for square in without_deferred.occupied() {
        builder
            .put(square, without_deferred.piece_at(square).unwrap())
            .unwrap();
    }
    builder.mark_promotion_deferred(sq(5, 8)).unwrap();
    let mut game_a = Game::from_position(rules, builder.finish().unwrap());
    let mut game_b = Game::from_position(rules, without_deferred);
    let mv = step(sq(5, 8), sq(5, 9));
    let (mut driver, _) = expand_line(&game_a, &[mv]);
    let root_b = SearchSnapshot::from_game(&game_b).unwrap();
    assert_eq!(driver.snapshot.history_keys(), root_b.history_keys());
    assert!(!driver.reuse(&root_b, DrawValue::SelfPlay).unwrap());
    assert_eq!(game_b.play(mv), Ok(GameStatus::Ongoing));
    let child_b = SearchSnapshot::from_game(&game_b).unwrap();
    assert!(!driver.reuse(&child_b, DrawValue::SelfPlay).unwrap());
    assert_eq!(game_a.play(mv), Ok(GameStatus::Ongoing));
    let child_a = SearchSnapshot::from_game(&game_a).unwrap();
    assert_eq!(child_a.history_keys(), child_b.history_keys());
    assert!(driver.reuse(&child_a, DrawValue::SelfPlay).unwrap());
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
    assert_eq!(driver.tree.depth_limit, MAX_PLY);
    driver.tree.depth_limit = 2;
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
    let gumbel = Gumbel::new(vec![0.0; moves.len()], 32, 1.0).unwrap();
    for _ in 0..32 {
        match driver.select_leaf(SelectionRule::Gumbel(&gumbel)).unwrap() {
            Step::Evaluate(request) => evaluate(&mut driver, &mut UniformEvaluator, &request),
            Step::BackedUp => {}
            _ => panic!("Gumbel is sequential and has sufficient capacity"),
        }
    }
    assert_eq!(driver.simulations(), 33);
    let root = driver.tree().root();
    assert!(moves.contains(&root.edges[gumbel.action(root, 1.0).unwrap()].mv));
    let target = gumbel.policy_target(root).unwrap();
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
        driver.tree.depth_limit = 1;
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
            let captures_royal = has_royal_capture(&oracle);
            assert_eq!(child.state == WIN_IN_ONE, captures_royal);
            if captures_royal {
                assert!(!evaluated);
                assert_eq!(child.value, None);
                assert!(child.edges.is_empty());
            }
            assert!(
                evaluated || captures_royal || child.has_approximate_result(),
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
                    ..Rules::JCSA
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
    let game = Game::from_position(Rules::JCSA, position);
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
            Rules::JCSA,
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
        Rules::JCSA,
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
                Rules::JCSA,
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
            Rules::JCSA,
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
                ..Rules::JCSA
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
        ..Rules::JCSA
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
        let mut game = Game::from_position(Rules::JCSA, start);
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
    driver.tree.depth_limit = 2;
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
    driver.tree.depth_limit = 4;
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
                    ..Rules::JCSA
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
    let mut captures_in_one = 0;
    let mut no_capture = 0;
    for repetition in [RepetitionRule::R1, RepetitionRule::R2, RepetitionRule::R3] {
        for seed in [1, 17, 123, 20261028] {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut game = Game::new(Rules {
                repetition,
                ..Rules::JCSA
            });
            for _ in 0..600 {
                let moves = game.legal_moves();
                let mv = moves[rng.index(NonZeroUsize::new(moves.len()).unwrap())];
                let mut next = game.clone();
                let expected = next.play(mv).unwrap();
                let driver = check_transition(&game, mv, expected);
                if expected == GameStatus::Ongoing {
                    if driver.tree().node(&[0]).unwrap().state == WIN_IN_ONE {
                        captures_in_one += 1;
                    } else {
                        no_capture += 1;
                    }
                }
                checked += 1;
                game = next;
                if matches!(expected, GameStatus::Finished(_)) {
                    terminal += 1;
                    break;
                }
            }
        }
    }
    eprintln!(
        "referee comparison: checked={checked}, terminal={terminal}, captures_in_one={captures_in_one}, no_capture={no_capture}"
    );
    assert!(checked >= 600, "only {checked} plies checked");
    assert!(terminal > 0, "random coverage must include a terminal move");
    assert!(
        captures_in_one > 0,
        "random coverage must include a win in one"
    );
    assert!(
        no_capture > 0,
        "random coverage must include a position without a win in one"
    );
}

#[test]
fn reuse_compares_full_rules_and_the_draw_perspective() {
    let game = Game::new(Rules::JCSA);
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
        Rules::JCSA,
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
        let mut game = Game::from_position(Rules::JCSA, position(Color::White, &pieces));
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
        Rules::JCSA,
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
        Rules::JCSA,
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
    let mut game = Game::from_position(Rules::JCSA, start);
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

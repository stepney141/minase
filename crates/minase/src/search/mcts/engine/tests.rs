use super::*;
use crate::nn::{Query, UniformEvaluator};
use crate::search::mcts::test_support::*;
use crate::search::mcts::tree::{Edge, Node};
use crate::search::{MATE, SearchRepetition};
use minase_core::test_util::{position, sq};
use minase_core::{Color, Game, GameStatus, PieceKind, PromotionRule, RepetitionRule};
use std::error::Error as _;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

fn engine(evaluator: impl Evaluator + Send + 'static) -> Engine {
    Engine::with_evaluator(
        Some(Box::new(evaluator)),
        Config {
            capacity_bytes: CAPACITY,
            batch_size: NonZeroUsize::new(4).unwrap(),
            ..Config::new(SCALE)
        },
    )
}

fn finish(handle: SearchHandle) -> (Engine, SearchEvent) {
    let event = handle
        .events()
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(matches!(
        event,
        SearchEvent::Finished {
            partial_score: None,
            ..
        }
    ));
    assert_eq!(
        handle.events().recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    let Searcher::Mcts(engine) = handle.join().unwrap() else {
        panic!()
    };
    (engine, event)
}

#[test]
fn uniform_engine_captures_the_last_royal_and_finishes_once() {
    let root = position(
        Color::Black,
        &[
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(5, 4), Color::Black, PieceKind::Rook),
            (sq(5, 9), Color::White, PieceKind::King),
        ],
    );
    let mv = step(sq(5, 4), sq(5, 9));
    let (_, event) = finish(
        engine(UniformEvaluator)
            .start(snapshot(&root), nodes(1000), 17, false)
            .unwrap(),
    );
    let SearchEvent::Finished {
        search_id,
        best_move,
        score,
        depth,
        nodes,
        pv,
        stop_reason,
        ..
    } = event
    else {
        panic!()
    };
    assert_eq!(search_id, 17);
    assert_eq!(best_move, mv);
    assert_eq!(score, MATE - 1);
    assert_eq!(depth, 1);
    assert!(nodes > 1);
    assert_eq!(pv, [mv]);
    assert_eq!(stop_reason, StopReason::DepthCompleted);
}

#[test]
fn nodes_one_chooses_the_maximum_policy_logit_even_after_a_previous_search() {
    let input = initial();
    let mv = *input.root_moves().last().unwrap();
    let evaluator = ScriptedEvaluator::line(input.position(), &[mv]);
    let (engine, _) = finish(
        engine(evaluator)
            .start(input.clone(), nodes(8), 1, false)
            .unwrap(),
    );
    let (engine, event) = finish(engine.start(input, nodes(1), 2, false).unwrap());
    assert!(!engine.reused);
    assert!(
        matches!(event, SearchEvent::Finished { best_move, score: 0, depth: 1, nodes: 1, stop_reason: StopReason::NodeLimit, .. } if best_move == mv)
    );
}

struct RecordingEvaluator {
    batches: Arc<Mutex<Vec<usize>>>,
}
impl Evaluator for RecordingEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        self.batches.lock().unwrap().push(queries.len());
        UniformEvaluator.evaluate(queries, logits, values)
    }
}

#[test]
fn batch_collection_honors_both_batch_size_and_remaining_simulations() {
    let batches = Arc::new(Mutex::new(Vec::new()));
    let (_, event) = finish(
        engine(RecordingEvaluator {
            batches: batches.clone(),
        })
        .start(initial(), nodes(10), 1, false)
        .unwrap(),
    );
    assert_eq!(*batches.lock().unwrap(), [1, 4, 4, 1]);
    assert!(matches!(
        event,
        SearchEvent::Finished {
            nodes: 10,
            stop_reason: StopReason::NodeLimit,
            ..
        }
    ));
}

#[test]
fn capacity_stops_after_evaluating_the_root_without_counting_a_failed_descent() {
    let input = initial();
    let mut engine = engine(UniformEvaluator);
    engine.config.capacity_bytes =
        std::mem::size_of::<Node>() + input.root_moves().len() * std::mem::size_of::<Edge>();
    let (engine, event) = finish(engine.start(input, nodes(100), 1, false).unwrap());
    assert!(matches!(
        event,
        SearchEvent::Finished {
            nodes: 1,
            stop_reason: StopReason::NodeLimit,
            ..
        }
    ));
    assert!(
        engine
            .driver
            .unwrap()
            .tree()
            .root()
            .edges
            .iter()
            .all(|edge| edge.virtual_loss == 0)
    );
}

#[test]
fn played_subtree_is_reused_and_clear_or_draw_changes_discard_it() {
    for change in [0, 1, 2] {
        let mut game = Game::new(rules(RepetitionRule::R1));
        let input = SearchSnapshot::from_game(&game).unwrap();
        let (mut engine, _) = finish(
            engine(UniformEvaluator)
                .start(input, nodes(40), 1, false)
                .unwrap(),
        );
        let driver = engine.driver.as_ref().unwrap();
        let edge = driver
            .tree()
            .root()
            .edges
            .iter()
            .find(|edge| {
                edge.child
                    .as_ref()
                    .is_some_and(|child| child.state == NodeState::Expanded)
            })
            .unwrap();
        let mv = edge.mv;
        let bytes = edge.child.as_ref().unwrap().edges.len() * std::mem::size_of::<Edge>()
            + std::mem::size_of::<Node>();
        assert!(driver.tree().used_bytes() > bytes);
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
        let mut next = SearchSnapshot::from_game(&game).unwrap();
        if change == 1 {
            engine.clear();
            assert!(engine.driver.is_none());
        }
        if change == 2 {
            next = next.with_draw_value(100).unwrap();
        }
        let (engine, _) = finish(engine.start(next, nodes(2), 2, false).unwrap());
        assert_eq!(engine.reused, change == 0);
        assert_eq!(engine.driver.unwrap().simulations(), 2);
    }
}

#[test]
fn new_root_at_a_repetition_terminal_is_rebuilt_from_snapshot_candidates() {
    let (root, line) = rook_cycle(false, false);
    let mut game = Game::from_position(rules(RepetitionRule::R1), root.clone());
    let mut engine = engine(ScriptedEvaluator::line(&root, &line));
    engine.config.batch_size = NonZeroUsize::new(1).unwrap();
    let (engine, _) = finish(
        engine
            .start(
                SearchSnapshot::from_game(&game).unwrap(),
                nodes(5),
                1,
                false,
            )
            .unwrap(),
    );
    for mv in line {
        assert_eq!(game.play(mv), Ok(GameStatus::Ongoing));
    }
    let input = SearchSnapshot::from_game(&game).unwrap();
    let expected = input.root_moves().to_vec();
    let (engine, _) = finish(engine.start(input, nodes(2), 2, false).unwrap());
    assert!(!engine.reused);
    let driver = engine.driver.unwrap();
    assert_eq!(driver.tree().root().state, NodeState::Expanded);
    assert_eq!(
        driver
            .tree()
            .root()
            .edges
            .iter()
            .map(|edge| edge.mv)
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn reuse_rejects_changed_history_rules_position_candidates_and_draw_perspective() {
    for change in 0..6 {
        let input = initial()
            .with_draw_value(if change == 5 { 100 } else { 0 })
            .unwrap();
        let (mut engine, _) = finish(
            engine(UniformEvaluator)
                .start(input.clone(), nodes(8), 1, false)
                .unwrap(),
        );
        let mut next = input;
        match change {
            0 => next.history_keys[0] ^= 1,
            1 => next.rules.l2 = !next.rules.l2,
            2 => next.repetition = SearchRepetition::R3,
            3 => {
                next.position.make_null_move();
            }
            4 => {
                next.root_moves.pop();
            }
            5 => {
                let mv = engine
                    .driver
                    .as_ref()
                    .unwrap()
                    .tree()
                    .root()
                    .edges
                    .iter()
                    .find(|edge| edge.child.is_some())
                    .unwrap()
                    .mv;
                let mut game = Game::new(rules(RepetitionRule::R1));
                game.play(mv).unwrap();
                next = SearchSnapshot::from_game(&game)
                    .unwrap()
                    .with_draw_value(100)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        // 不整合な合成入力は探索せず、再利用の検査だけに使う。
        engine.prepare_driver(next, nodes(2)).unwrap();
        assert!(!engine.reused, "case {change}");
    }
}

#[test]
fn reordered_root_candidates_keep_the_reused_edge_logit_mapping() {
    let input = initial();
    let selected = input.root_moves()[0];
    let (mut engine, _) = finish(
        engine(ScriptedEvaluator::line(input.position(), &[selected]))
            .start(input.clone(), nodes(4), 1, false)
            .unwrap(),
    );
    let mut next = input;
    next.root_moves.reverse();
    engine.prepare_driver(next, nodes(2)).unwrap();
    assert!(engine.reused);
    let driver = engine.driver.as_mut().unwrap();
    let Step::Evaluate(request) = driver
        .select_leaf(SelectionRule::Puct(&PuctConfig::default()))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(request.query().moves[0], selected);
    evaluate(
        driver,
        engine.evaluator.as_mut().unwrap().as_mut(),
        &request,
    );
    assert_eq!(driver.tree().root().edges[0].logit, 0.0);
}

#[derive(Default)]
struct ManualClock(AtomicU64);
impl ManualClock {
    fn set(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}
impl SearchClock for ManualClock {
    fn elapsed(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

struct GatedEvaluator {
    entered: Sender<usize>,
    release: Receiver<()>,
    dropped: Sender<()>,
    calls: usize,
}
impl Evaluator for GatedEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        self.calls += 1;
        self.entered.send(self.calls).unwrap();
        self.release.recv_timeout(Duration::from_secs(5)).unwrap();
        UniformEvaluator.evaluate(queries, logits, values)
    }
}
impl Drop for GatedEvaluator {
    fn drop(&mut self) {
        let _ = self.dropped.send(());
    }
}

fn gated() -> (Engine, Receiver<usize>, Sender<()>, Receiver<()>) {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (dropped_tx, dropped) = mpsc::channel();
    let mut engine = engine(GatedEvaluator {
        entered: entered_tx,
        release: release_rx,
        dropped: dropped_tx,
        calls: 0,
    });
    engine.config.batch_size = NonZeroUsize::new(1).unwrap();
    (engine, entered, release, dropped)
}
fn entered(receiver: &Receiver<usize>, expected: usize) {
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        expected
    );
}

#[test]
fn external_stop_still_evaluates_the_root_and_sends_only_finished() {
    let (engine, calls, release, _) = gated();
    let handle = engine
        .start(initial(), SearchLimits::infinite(), 1, false)
        .unwrap();
    entered(&calls, 1);
    handle.request_stop();
    release.send(()).unwrap();
    let (_, event) = finish(handle);
    assert!(matches!(
        event,
        SearchEvent::Finished {
            nodes: 1,
            stop_reason: StopReason::ExternalStop,
            ..
        }
    ));
}

struct DropProbe(Sender<()>);
impl Evaluator for DropProbe {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        UniformEvaluator.evaluate(queries, logits, values)
    }
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.send(()).unwrap();
    }
}

#[test]
fn dropping_the_handle_stops_and_joins_the_mcts_thread() {
    let (dropped_tx, dropped) = mpsc::channel();
    let handle = engine(DropProbe(dropped_tx))
        .start(initial(), SearchLimits::infinite(), 1, false)
        .unwrap();
    drop(handle);
    assert_eq!(dropped.try_recv(), Ok(()));
}

#[test]
fn ponder_ignores_time_until_hit_then_finishes_once_at_the_budget() {
    let (engine, calls, release, _) = gated();
    let clock = Arc::new(ManualClock::default());
    let limits = SearchLimits::new(None, None, Some(100), None).unwrap();
    let handle = engine
        .start_with_clock(initial(), limits, 1, true, clock.clone())
        .unwrap();
    entered(&calls, 1);
    clock.set(10_000);
    release.send(()).unwrap();
    entered(&calls, 2);
    assert_eq!(handle.events().try_recv(), Err(mpsc::TryRecvError::Empty));
    handle.ponderhit();
    release.send(()).unwrap();
    entered(&calls, 3);
    clock.set(10_050);
    handle.ponderhit();
    clock.set(10_100);
    release.send(()).unwrap();
    let (_, event) = finish(handle);
    assert!(
        matches!(event, SearchEvent::Finished { nodes: 3, elapsed, stop_reason: StopReason::SoftLimit, .. } if elapsed == Duration::from_millis(10_100))
    );
}

#[test]
fn hard_limit_predicts_one_more_collection_and_inference_batch() {
    let (engine, calls, release, _) = gated();
    let clock = Arc::new(ManualClock::default());
    let limits = SearchLimits::new(None, None, Some(100), None).unwrap();
    let handle = engine
        .start_with_clock(initial(), limits, 1, false, clock.clone())
        .unwrap();
    entered(&calls, 1);
    release.send(()).unwrap();
    entered(&calls, 2);
    clock.set(60);
    release.send(()).unwrap();
    let (_, event) = finish(handle);
    assert!(matches!(
        event,
        SearchEvent::Finished {
            nodes: 2,
            stop_reason: StopReason::HardLimit,
            ..
        }
    ));
}

#[test]
fn finite_time_limits_use_the_shared_clock_budget() {
    let (engine, calls, release, _) = gated();
    let clock = Arc::new(ManualClock::default());
    // 秒読みだけ100 ms、安全余裕30 msなので、softとhardは70 msである。
    let limits = SearchLimits::new(
        None,
        None,
        None,
        Some(crate::search::ClockLimits::new(0, 0, 100, None, 0).unwrap()),
    )
    .unwrap();
    let handle = engine
        .start_with_clock(initial(), limits, 1, false, clock.clone())
        .unwrap();
    entered(&calls, 1);
    clock.set(70);
    release.send(()).unwrap();
    let (_, event) = finish(handle);
    assert!(matches!(
        event,
        SearchEvent::Finished {
            nodes: 1,
            stop_reason: StopReason::SoftLimit,
            ..
        }
    ));
}

#[test]
fn mcts_start_rejects_missing_evaluator_depth_and_deferred_promotion_rules() {
    let no_evaluator = Engine::with_evaluator(None, Config::new(SCALE));
    let failure = match Searcher::Mcts(no_evaluator).start(initial(), nodes(1), 1, false) {
        Err(failure) => failure,
        Ok(_) => panic!(),
    };
    assert!(matches!(
        failure.error,
        StartError::Mcts(Error::MissingEvaluator)
    ));
    assert!(matches!(failure.searcher, Searcher::Mcts(_)));
    let limits = SearchLimits::new(Some(2), None, None, None).unwrap();
    let failure = match Searcher::Mcts(engine(UniformEvaluator)).start(initial(), limits, 1, false)
    {
        Err(failure) => failure,
        Ok(_) => panic!(),
    };
    assert!(matches!(
        failure.error,
        StartError::Mcts(Error::DepthLimitUnsupported)
    ));
    assert!(matches!(failure.searcher, Searcher::Mcts(_)));
    for (promotion, p5) in [
        (PromotionRule::P1, false),
        (PromotionRule::P2, false),
        (PromotionRule::P0, true),
    ] {
        let mut input = initial();
        assert!(input.position().promotion_deferred().is_empty());
        input.rules.promotion = promotion;
        input.rules.p5 = p5;
        let failure =
            match Searcher::Mcts(engine(UniformEvaluator)).start(input, nodes(1), 1, false) {
                Err(failure) => failure,
                Ok(_) => panic!(),
            };
        assert!(matches!(
            failure.error,
            StartError::Mcts(Error::PromotionDeferred)
        ));
        assert!(matches!(failure.searcher, Searcher::Mcts(_)));
    }
}

#[test]
fn mcts_options_are_bounded_and_prepare_does_not_allocate_a_transposition_table() {
    let mut searcher = Searcher::Mcts(engine(UniformEvaluator));
    let options: Vec<_> = searcher
        .options()
        .into_iter()
        .filter_map(|option| {
            let SearchOptionKind::Spin { default, min, max } = option.kind else {
                return None;
            };
            Some((option.name, default, min, max))
        })
        .collect();
    assert_eq!(
        options,
        [
            ("USI_Hash", 256, 1, 65_536),
            ("Threads", 1, 1, 1),
            ("NnBatchSize", 256, 1, 1024)
        ]
    );
    for (name, value) in [
        ("USI_Hash", 0),
        ("USI_Hash", 65_537),
        ("USI_Hash", usize::MAX),
        ("Threads", 0),
        ("Threads", 2),
    ] {
        assert!(matches!(
            searcher.set_option(name, SearchOptionValue::Spin(value)),
            Err(SetOptionError::OutOfRange { .. })
        ));
    }
    assert!(matches!(
        searcher.set_option("Unknown", SearchOptionValue::Spin(4)),
        Err(SetOptionError::UnknownOption { .. })
    ));
    searcher
        .set_option("USI_Hash", SearchOptionValue::Spin(1))
        .unwrap();
    searcher
        .set_option("Threads", SearchOptionValue::Spin(1))
        .unwrap();
    searcher.prepare().unwrap();
    assert!(searcher.transposition_table().is_none());
    assert_eq!(searcher.threads().get(), 1);
}

struct FailingEvaluator {
    calls: usize,
}
impl Evaluator for FailingEvaluator {
    fn evaluate(
        &mut self,
        queries: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        self.calls += 1;
        if self.calls == 2 {
            return Err(NnError::ReadNetwork {
                path: "test-network.onnx".into(),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            });
        }
        UniformEvaluator.evaluate(queries, logits, values)
    }
}

#[test]
fn inference_failure_emits_no_finished_and_preserves_the_original_error() {
    let handle = engine(FailingEvaluator { calls: 0 })
        .start(initial(), nodes(10), 1, false)
        .unwrap();
    assert_eq!(
        handle.events().recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    let error = match handle.join() {
        Err(error) => error,
        Ok(_) => panic!(),
    };
    assert!(error.to_string().contains("test-network.onnx"));
    assert!(error.source().unwrap().downcast_ref::<NnError>().is_some());
    let SearchJoinError::Inference(NnError::ReadNetwork { source, .. }) = error else {
        panic!()
    };
    assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
}

struct PanicEvaluator;
impl Evaluator for PanicEvaluator {
    fn evaluate(
        &mut self,
        _: &[Query<'_>],
        _: &mut Vec<f32>,
        _: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        std::panic::panic_any(42_u32)
    }
}

#[test]
fn thread_panic_is_distinct_from_inference_failure_and_keeps_its_payload() {
    let handle = engine(PanicEvaluator)
        .start(initial(), nodes(1), 1, false)
        .unwrap();
    assert_eq!(
        handle.events().recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    let error = match handle.join() {
        Err(error) => error,
        Ok(_) => panic!(),
    };
    let SearchJoinError::Panic(payload) = error else {
        panic!()
    };
    assert_eq!(*payload.downcast::<u32>().unwrap(), 42);
}

#[test]
fn an_already_requested_stop_cannot_skip_the_root_evaluation() {
    let config = Config::new(SCALE);
    let mut driver = Driver::new(initial(), CAPACITY, DrawValue::SelfPlay).unwrap();
    let clock = ManualClock::default();
    clock.set(1_000);
    let limits = SearchLimits::new(None, None, Some(100), None).unwrap();
    let reason = run(
        &mut driver,
        &mut UniformEvaluator,
        config,
        limits,
        &clock,
        &AtomicU64::new(0),
        &AtomicBool::new(true),
    )
    .unwrap();
    assert_eq!(reason, StopReason::ExternalStop);
    assert_eq!(driver.simulations(), 1);
    assert_eq!(driver.tree().root().value, Some(0.5));
}

#[test]
fn invalid_scale_puct_and_root_capacity_return_the_engine_before_spawning() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut engine = engine(UniformEvaluator);
        engine.config.cp_per_logit = scale;
        let failure = match engine.start(initial(), nodes(2), 1, false) {
            Err(failure) => failure,
            Ok(_) => panic!(),
        };
        assert!(matches!(
            failure.error,
            StartError::Mcts(Error::InvalidScale)
        ));
        assert!(matches!(failure.searcher, Searcher::Mcts(_)));
    }
    for capacity in [false, true] {
        let mut engine = engine(UniformEvaluator);
        if capacity {
            engine.config.capacity_bytes = 0;
        } else {
            engine.config.puct.c_base = 0.0;
        }
        let failure = match engine.start(initial(), nodes(2), 1, false) {
            Err(failure) => failure,
            Ok(_) => panic!(),
        };
        let expected = if capacity {
            super::super::tree::TreeError::CapacityExceeded
        } else {
            super::super::tree::TreeError::InvalidPuctConfig
        };
        assert!(matches!(failure.error, StartError::Mcts(Error::Tree(error)) if error == expected));
        assert!(matches!(failure.searcher, Searcher::Mcts(_)));
    }
}

struct WrongShapeEvaluator;
impl Evaluator for WrongShapeEvaluator {
    fn evaluate(
        &mut self,
        _: &[Query<'_>],
        logits: &mut Vec<f32>,
        values: &mut Vec<f32>,
    ) -> Result<(), NnError> {
        logits.clear();
        values.clear();
        Ok(())
    }
}

#[test]
fn malformed_inference_output_is_an_error_without_panicking_or_finishing() {
    let handle = engine(WrongShapeEvaluator)
        .start(initial(), nodes(1), 1, false)
        .unwrap();
    assert_eq!(
        handle.events().recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    assert!(matches!(
        handle.join(),
        Err(SearchJoinError::Inference(NnError::OutputShape {
            name: "values",
            ..
        }))
    ));
}

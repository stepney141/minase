//! MCTSの試験（docs/plans/alphazero.md「検証」の節のうち探索本体の項目）。

use core::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::Deserialize;

use minase_core::rules::{RuleCode, Rules};
use minase_core::test_util::sq;
use minase_core::{Color, Game, GameStatus, Move, PieceCode, PieceKind, Position, PositionBuilder};

use super::engine::{
    Config, DEFAULT_GUMBEL_SIMULATIONS, Engine, MAX_CONSIDERED_ACTIONS, MctsError,
};
use super::select::{GumbelParams, GumbelRoot, Selection, improved_policy};
use super::simulate::{Domain, Rule, Simulator};
use super::tree::{Proven, Tree};
use crate::nn::evaluator::{Evaluator, EvaluatorError, INPUT_SIZE, UniformEvaluator};
use crate::nn::{POLICY_SIZE, policy};
use crate::search::events::{SearchEvent, StopReason};
use crate::search::handle::SearchHandle;
use crate::search::limits::{ClockLimits, SearchLimits};
use crate::search::searcher::Searcher;
use crate::search::snapshot::SearchSnapshot;
use crate::search::{MATE, MATE_THRESHOLD};

/// 探索の設定を作る。
fn config(selection: Selection, batch_size: usize, gumbel_simulations: u32) -> Config {
    Config {
        selection,
        batch_size: NonZeroUsize::new(batch_size).unwrap(),
        gumbel_simulations: NonZeroU32::new(gumbel_simulations).unwrap(),
        max_considered: MAX_CONSIDERED_ACTIONS,
        seed: NonZeroU64::new(7).unwrap(),
        win_rate_scale: crate::eval::weights().unwrap().k(),
        max_nodes: NonZeroU32::MAX,
    }
}

/// 評価器と設定からMCTSを作る。
fn engine(evaluator: impl Evaluator + Send + 'static, config: Config) -> Engine {
    Engine::new(Box::new(evaluator), config)
}

/// ノード数だけを制限する。
fn nodes(limit: u64) -> SearchLimits {
    SearchLimits::new(None, Some(limit), None, None).unwrap()
}

/// 駒の一覧から局面を作る。
fn position(side_to_move: Color, pieces: &[((u8, u8), Color, PieceKind)]) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    for &((file, rank), color, kind) in pieces {
        builder
            .put(sq(file, rank), PieceCode::new(color, kind).unwrap())
            .unwrap();
    }
    builder.finish().unwrap()
}

/// 標準の規則の対局のスナップショットを作る。
fn snapshot_of(position: Position) -> SearchSnapshot {
    SearchSnapshot::from_game(&Game::from_position(Rules::ENGINE_DEFAULT, position)).unwrap()
}

// ---------------------------------------------------------------------------
// mctxとの照合
// ---------------------------------------------------------------------------

/// `gen_gumbel_fixture.py`が書いた参照値。
#[derive(Deserialize)]
struct Fixture {
    scenarios: Vec<Scenario>,
}

/// 1つの木についての参照値。
#[derive(Deserialize)]
struct Scenario {
    name: String,
    num_actions: usize,
    num_simulations: u32,
    max_considered: u32,
    c_visit: f64,
    c_scale: f64,
    gumbel: Vec<f64>,
    table: Vec<TableRow>,
    visit_order: Vec<Vec<usize>>,
    root_visits: Vec<u32>,
    action: usize,
    action_weights: Vec<f64>,
}

/// 根からの手順で表した節点の方策logitと手番側の勝率。
#[derive(Deserialize)]
struct TableRow {
    path: Vec<usize>,
    logits: Vec<f32>,
    value: f32,
}

/// 根からの手順を局面とし、すべての節点が同じ数の行動を持つ木。
struct TableDomain {
    num_actions: usize,
    path: Vec<usize>,
}

impl Domain for TableDomain {
    type Action = usize;

    fn key(&self) -> u64 {
        self.path
            .iter()
            .fold(0x9E37_79B9_7F4A_7C15, |key, &action| {
                minase_core::rng::splitmix64(key ^ action as u64)
            })
    }

    fn legal_actions(&mut self, out: &mut Vec<usize>) {
        out.clear();
        out.extend(0..self.num_actions);
    }

    fn wins_at_once(&self, _: usize) -> bool {
        false
    }

    fn label(&self, action: usize) -> Result<usize, MctsError> {
        Ok(action)
    }

    /// 入力の先頭に手順の長さ、続けて手順を書く。
    fn encode(&self, out: &mut [f32]) {
        out.fill(0.0);
        out[0] = self.path.len() as f32;
        for (slot, &action) in out[1..].iter_mut().zip(&self.path) {
            *slot = action as f32;
        }
    }

    fn play(&mut self, action: usize) {
        self.path.push(action);
    }

    fn undo(&mut self) {
        self.path.pop();
    }
}

/// 手順ごとの表を返し、評価した手順を順に記録する評価器。
struct TableEvaluator {
    table: HashMap<Vec<usize>, (Vec<f32>, f32)>,
    log: Vec<Vec<usize>>,
}

impl Evaluator for TableEvaluator {
    fn evaluate(
        &mut self,
        inputs: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        out_policy.fill(0.0);
        for row in 0..batch {
            let input = &inputs[row * INPUT_SIZE..(row + 1) * INPUT_SIZE];
            let path: Vec<usize> = input[1..=input[0] as usize]
                .iter()
                .map(|&action| action as usize)
                .collect();
            let (logits, value) = self
                .table
                .get(&path)
                .unwrap_or_else(|| panic!("path {path:?} is not in the fixture table"));
            out_policy[row * POLICY_SIZE..row * POLICY_SIZE + logits.len()].copy_from_slice(logits);
            out_value[row] = *value;
            self.log.push(path);
        }
        Ok(())
    }
}

/// 相対誤差を返す。
fn relative_error(actual: f64, expected: f64) -> f64 {
    (actual - expected).abs() / expected.abs().max(f64::MIN_POSITIVE)
}

// alphazero.md「検証」: 固定の小さな探索木について、Gumbel探索の訪問の順序と方策の目標が
// mctxの出力と一致する。参照値は gen_gumbel_fixture.py が mctx 0.0.71 で生成した。
#[test]
fn gumbel_search_matches_mctx() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("../../../tests/fixtures/gumbel_mctx.json")).unwrap();
    assert!(!fixture.scenarios.is_empty());
    for scenario in fixture.scenarios {
        let name = &scenario.name;
        let mut evaluator = TableEvaluator {
            table: scenario
                .table
                .iter()
                .map(|row| (row.path.clone(), (row.logits.clone(), row.value)))
                .collect(),
            log: Vec::new(),
        };
        let domain = TableDomain {
            num_actions: scenario.num_actions,
            path: Vec::new(),
        };
        let mut tree = Tree::new(domain.key());
        let mut simulator = Simulator::new(domain);
        let actions: Vec<usize> = (0..scenario.num_actions).collect();
        assert!(
            simulator
                .expand_root(&mut tree, &mut evaluator, &actions)
                .unwrap()
        );
        let params = GumbelParams {
            c_visit: scenario.c_visit,
            c_scale: scenario.c_scale,
        };
        let root = GumbelRoot::new(
            scenario.gumbel.clone(),
            scenario.num_simulations,
            scenario.max_considered,
            params,
        );
        for _ in 0..scenario.num_simulations {
            let simulations = simulator
                .run_batch(
                    &mut tree,
                    &mut evaluator,
                    &Rule::Gumbel(&root, params),
                    &[],
                    1,
                )
                .unwrap();
            assert_eq!(simulations, 1, "{name}");
        }

        assert_eq!(evaluator.log[0], Vec::<usize>::new(), "{name}");
        assert_eq!(evaluator.log[1..], scenario.visit_order, "{name}");
        let visits: Vec<u32> = tree.root().edges.iter().map(|edge| edge.visits).collect();
        assert_eq!(visits, scenario.root_visits, "{name}");
        assert_eq!(root.final_action(tree.root()), scenario.action, "{name}");
        let weights = improved_policy(tree.root(), params);
        assert_eq!(weights.len(), scenario.action_weights.len(), "{name}");
        for (actual, expected) in weights.into_iter().zip(scenario.action_weights) {
            assert!(
                relative_error(actual, expected) < 1e-5,
                "{name}: {actual} != {expected}"
            );
        }
    }
}

// alphazero.md「検証」: 合法手が16手未満の局面でも、候補数が合法手数に収まり、予算nを配り切る。
#[test]
fn gumbel_budget_is_spent_on_fewer_legal_moves_than_candidates() {
    // 先手の王将は隅にあり、合法手は3手だけである。
    let position = position(
        Color::Black,
        &[
            ((0, 0), Color::Black, PieceKind::King),
            ((11, 11), Color::White, PieceKind::King),
            ((11, 9), Color::White, PieceKind::Pawn),
        ],
    );
    let snapshot = snapshot_of(position);
    assert_eq!(snapshot.root_moves().len(), 3);
    let n = DEFAULT_GUMBEL_SIMULATIONS.get();
    let mut engine = engine(UniformEvaluator, config(Selection::Gumbel, 1, n));
    let result = engine.search(&snapshot, &nodes(1_000)).unwrap();
    let root = engine.previous_tree().unwrap().root();
    assert_eq!(root.edges.len(), 3);
    assert_eq!(root.total_visits(), n);
    assert!(root.edges.iter().all(|edge| edge.visits > 0));
    // 根の展開を1回、根からのシミュレーションをn回と数える。
    assert_eq!(result.nodes, u64::from(n) + 1);
}

// ---------------------------------------------------------------------------
// 終局、確定値、および反復
// ---------------------------------------------------------------------------

/// 先手の金将が後手の唯一の王駒を取れる局面。
fn royal_capture_position() -> Position {
    position(
        Color::Black,
        &[
            ((0, 0), Color::Black, PieceKind::King),
            ((5, 5), Color::Black, PieceKind::GoldGeneral),
            ((0, 3), Color::Black, PieceKind::Pawn),
            ((5, 6), Color::White, PieceKind::King),
            ((11, 8), Color::White, PieceKind::Pawn),
            ((11, 11), Color::White, PieceKind::GoldGeneral),
        ],
    )
}

// alphazero.md「検証」: 一様評価器のMCTSが、相手の最後の王駒を取る手がある局面でその手を選び、
// 詰みの帯の評価値を返す。
#[test]
fn takes_the_last_royal_and_reports_a_mate_score() {
    let snapshot = snapshot_of(royal_capture_position());
    let capture = Move {
        from: sq(5, 5),
        mid: None,
        to: sq(5, 6),
        promote: false,
    };
    assert!(snapshot.root_moves().contains(&capture));
    for selection in [Selection::Puct, Selection::Gumbel] {
        let mut engine = engine(UniformEvaluator, config(selection, 4, 32));
        let result = engine.search(&snapshot, &nodes(64)).unwrap();
        assert_eq!(result.best_move, capture, "{selection:?}");
        assert_eq!(result.score, MATE - 1, "{selection:?}");
        assert!(result.score >= MATE_THRESHOLD);
    }
}

/// 状態を番号で表した小さな有向グラフ。各状態は（行き先, その手で相手の最後の王駒を取るか）の列を持つ。
struct GraphDomain {
    keys: Vec<u64>,
    moves: Vec<Vec<(usize, bool)>>,
    path: Vec<usize>,
}

impl GraphDomain {
    fn current(&self) -> usize {
        *self.path.last().unwrap()
    }
}

impl Domain for GraphDomain {
    type Action = usize;

    fn key(&self) -> u64 {
        self.keys[self.current()]
    }

    fn legal_actions(&mut self, out: &mut Vec<usize>) {
        out.clear();
        out.extend(0..self.moves[self.current()].len());
    }

    fn wins_at_once(&self, action: usize) -> bool {
        self.moves[self.current()][action].1
    }

    fn label(&self, action: usize) -> Result<usize, MctsError> {
        Ok(action)
    }

    fn encode(&self, out: &mut [f32]) {
        out.fill(0.0);
    }

    fn play(&mut self, action: usize) {
        let next = self.moves[self.current()][action].0;
        self.path.push(next);
    }

    fn undo(&mut self) {
        self.path.pop();
    }
}

/// グラフの根から一様評価器のPUCTでシミュレーションを`count`回行った木を返す。
fn search_graph(moves: Vec<Vec<(usize, bool)>>, history: &[u64], count: usize) -> Tree<usize> {
    let keys = (0..moves.len() as u64).map(|state| 100 + state).collect();
    let root_actions: Vec<usize> = (0..moves[0].len()).collect();
    let domain = GraphDomain {
        keys,
        moves,
        path: vec![0],
    };
    let mut tree = Tree::new(domain.key());
    let mut simulator = Simulator::new(domain);
    let mut evaluator = UniformEvaluator;
    simulator
        .expand_root(&mut tree, &mut evaluator, &root_actions)
        .unwrap();
    for _ in 0..count {
        if tree.root().proven.is_some() {
            break;
        }
        simulator
            .run_batch(&mut tree, &mut evaluator, &Rule::Puct, history, 1)
            .unwrap();
    }
    tree
}

// alphazero.md「検証」: 勝ちが確定した子を持つ節点は勝ちとして扱われ、確定値が根まで伝わる。
#[test]
fn proven_results_propagate_to_the_root() {
    // 状態0の手0は状態1へ、手1は状態2へ進む。状態1の唯一の手は状態3へ進み、
    // 状態3には相手の最後の王駒を取る手がある。したがって状態1は2手で負け、
    // 状態0は手0で3手の勝ちになる。状態2は終局しない循環である。
    let moves = vec![
        vec![(1, false), (2, false)],
        vec![(3, false)],
        vec![(4, false)],
        vec![(5, true)],
        vec![(2, false)],
        vec![],
    ];
    let tree = search_graph(moves, &[], 64);
    let root = tree.root();
    assert_eq!(root.edges[0].proven, Some(Proven::Win(3)));
    let state1 = tree.node(root.edges[0].child.unwrap());
    assert_eq!(state1.proven, Some(Proven::Loss(2)));
    assert_eq!(root.proven, Some(Proven::Win(3)));
    assert_eq!(root.best_edge(), Some(0));
}

// alphazero.md「検証」: 合法手のない節点は負けであり、全子が負けの節点は負けとして扱われる。
#[test]
fn a_node_whose_children_all_win_is_lost() {
    // 状態0の2手は、どちらも合法手のない状態（手番側の負け）へ進むので、状態0は1手で勝つ。
    // 状態0から見ると、相手の状態1と状態2の子はすべて負けである。
    let moves = vec![
        vec![(1, false), (2, false)],
        vec![(3, false)],
        vec![],
        vec![],
    ];
    let tree = search_graph(moves, &[], 16);
    let root = tree.root();
    assert_eq!(root.edges[1].proven, Some(Proven::Win(1)));
    assert_eq!(root.proven, Some(Proven::Win(1)));
    assert_eq!(root.best_edge(), Some(1));

    // 状態1の唯一の子（状態3）は合法手がないので、状態1を根にすると1手の勝ちである。
    // 状態1の側から見れば、全子が負けの相手局面へ進む（状態0の子がすべて負け）ことと同じ形になる。
    let moves = vec![vec![(1, false)], vec![(2, true)], vec![]];
    let tree = search_graph(moves, &[], 16);
    assert_eq!(tree.root().proven, Some(Proven::Loss(2)));
}

// alphazero.md「検証」: 探索木の内側の反復は引き分けとして扱われる。
#[test]
fn repetitions_inside_the_tree_are_draws() {
    // 状態0の唯一の手は状態1へ進み、状態1の唯一の手は状態0へ戻る。
    let moves = vec![vec![(1, false)], vec![(0, false)]];
    let tree = search_graph(moves, &[], 8);
    let state1 = tree.node(tree.root().edges[0].child.unwrap());
    let back = &state1.edges[0];
    assert!(back.visits > 0);
    assert_eq!(back.value_sum, 0.5 * f64::from(back.visits));
    assert_eq!(back.child, None);
    assert_eq!(tree.root().proven, None);

    // 対局の履歴にある局面への到達も引き分けとする。
    let moves = vec![vec![(1, false)], vec![(2, false)], vec![(1, false)]];
    let tree = search_graph(moves, &[100, 102], 8);
    let edge = &tree.root().edges[0];
    let state1 = tree.node(edge.child.unwrap());
    assert_eq!(
        state1.edges[0].value_sum,
        0.5 * f64::from(state1.edges[0].visits)
    );
    assert_eq!(state1.edges[0].child, None);
}

// ---------------------------------------------------------------------------
// 探索回数の上限が1の動作
// ---------------------------------------------------------------------------

/// 1つのラベルのlogitだけを大きくし、それ以外を0、勝率を0.5とする評価器。
struct OneHotEvaluator {
    label: usize,
}

impl Evaluator for OneHotEvaluator {
    fn evaluate(
        &mut self,
        _: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        out_policy.fill(0.0);
        for row in 0..batch {
            out_policy[row * POLICY_SIZE + self.label] = 5.0;
        }
        out_value.fill(0.5);
        Ok(())
    }
}

// alphazero.md「教師あり学習」「検証」: 探索回数の上限が1のとき、方策logitが最大の合法手を返す。
#[test]
fn node_limit_one_plays_the_highest_logit_move() {
    let snapshot = snapshot_of(Position::initial());
    let side = snapshot.position().side_to_move();
    for &target in [
        snapshot.root_moves()[0],
        *snapshot.root_moves().last().unwrap(),
    ]
    .iter()
    {
        let label = usize::from(policy::label(target, side).unwrap());
        for selection in [Selection::Puct, Selection::Gumbel] {
            let mut engine = engine(OneHotEvaluator { label }, config(selection, 8, 32));
            let result = engine.search(&snapshot, &nodes(1)).unwrap();
            assert_eq!(result.best_move, target, "{selection:?}");
            assert_eq!(result.nodes, 1);
            assert_eq!(result.score, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// 根の候補手、木の再利用、深さの制限
// ---------------------------------------------------------------------------

// alphazero.md「検証」: 根の候補手はスナップショットの合法手に従う（R2とR3で禁じられた手は
// 対局管理層が既に除いている）。
#[test]
fn root_candidates_follow_the_snapshot() {
    let game = Game::with_default_rules();
    let allowed = game.legal_moves()[3];
    let snapshot = SearchSnapshot::from_parts(
        game.position().clone(),
        game.rules().moves,
        game.search_key_history().to_vec(),
        vec![allowed],
    )
    .unwrap();
    for selection in [Selection::Puct, Selection::Gumbel] {
        let mut engine = engine(UniformEvaluator, config(selection, 4, 8));
        let result = engine.search(&snapshot, &nodes(32)).unwrap();
        assert_eq!(result.best_move, allowed);
        assert_eq!(engine.previous_tree().unwrap().root().edges.len(), 1);
    }
}

/// 王将を隣の升へ動かす非捕獲の着手。
fn step(from: (u8, u8), to: (u8, u8)) -> Move {
    Move {
        from: sq(from.0, from.1),
        mid: None,
        to: sq(to.0, to.1),
        promote: false,
    }
}

// alphazero.md「検証」: R2とR3の規則セットで、根の候補手から規則が禁じる手が除かれる。
// 局面と手順は`protocol/usi.rs`の先読み手の検証の試験と同じく、双方の王将を往復させる。
// E0では王駒だけが残ると駒枯れの引き分け（RULES.md第22条第8項）で終局するので、
// 双方に動かさない金将を1枚ずつ置く。
#[test]
fn repetition_rules_remove_forbidden_root_moves() {
    let board = position(
        Color::Black,
        &[
            ((3, 3), Color::Black, PieceKind::King),
            ((0, 8), Color::Black, PieceKind::GoldGeneral),
            ((8, 8), Color::White, PieceKind::King),
            ((11, 3), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let cycle = [
        step((3, 3), (3, 4)),
        step((8, 8), (8, 7)),
        step((3, 4), (3, 3)),
        step((8, 7), (8, 8)),
    ];
    // 先手が往復を終えた後の後手の戻る手は、開始局面（先手番）を再現する。
    let back = cycle[3];
    // （反復規則, 先に指す往復の回数, 戻る手が許されるか）。R3は4回目の出現だけを禁じる。
    for (rule, cycles, allowed) in [
        (RuleCode::R2, 0, false),
        (RuleCode::R3, 0, true),
        (RuleCode::R3, 2, false),
    ] {
        let rules = Rules::from_codes(&[RuleCode::L0, RuleCode::P0, rule, RuleCode::E0]).unwrap();
        let mut game = Game::from_position(rules, board.clone());
        for _ in 0..cycles {
            for mv in cycle {
                game.play(mv).unwrap();
            }
        }
        for mv in &cycle[..3] {
            game.play(*mv).unwrap();
        }
        assert!(matches!(game.status(), GameStatus::Ongoing));
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        let case = format!("{rule:?} after {cycles} cycles");
        assert_eq!(snapshot.root_moves().contains(&back), allowed, "{case}");
        for selection in [Selection::Puct, Selection::Gumbel] {
            let mut engine = engine(UniformEvaluator, config(selection, 4, 32));
            let result = engine.search(&snapshot, &nodes(64)).unwrap();
            assert!(snapshot.root_moves().contains(&result.best_move));
            let root = engine.previous_tree().unwrap().root();
            assert_eq!(
                root.edges.iter().any(|edge| edge.action == back),
                allowed,
                "{case} {selection:?}"
            );
            if !allowed {
                assert_ne!(result.best_move, back, "{case} {selection:?}");
            }
        }
    }
}

// alphazero.md「検証」: 指した手の部分木が次の探索で再利用され、再利用後の根の訪問回数は
// 前の探索の子の訪問回数から始まる。
#[test]
fn the_played_subtree_is_reused() {
    let mut game = Game::with_default_rules();
    let mut engine = engine(UniformEvaluator, config(Selection::Puct, 4, 32));
    engine
        .search(&SearchSnapshot::from_game(&game).unwrap(), &nodes(300))
        .unwrap();

    // 最も訪問した手と、その先の最も訪問した応手を指す（USIの対局と同じ2手分の前進）。
    let tree = engine.previous_tree().unwrap();
    let first = tree.root().best_edge().unwrap();
    let first_node = tree.node(tree.root().edges[first].child.unwrap());
    let reply = first_node.best_edge().unwrap();
    let reply_node = tree.node(first_node.edges[reply].child.unwrap());
    let inherited = reply_node.total_visits();
    assert!(inherited > 0);
    game.play(tree.root().edges[first].action).unwrap();
    game.play(first_node.edges[reply].action).unwrap();

    let simulations = 20;
    let result = engine
        .search(
            &SearchSnapshot::from_game(&game).unwrap(),
            &nodes(simulations),
        )
        .unwrap();
    assert_eq!(result.nodes, simulations);
    let root = engine.previous_tree().unwrap().root();
    assert_eq!(
        u64::from(root.total_visits()),
        u64::from(inherited) + simulations
    );

    // 新しい対局では木を捨てる。
    engine.new_game();
    assert!(engine.previous_tree().is_none());
}

// MCTSには深さに対応する概念がないので、深さの制限は専用のエラーで拒否する。
// 同期探索の無期限探索はαβ探索と同じく拒否する。
#[test]
fn depth_limits_are_rejected() {
    let snapshot = snapshot_of(Position::initial());
    let depth = SearchLimits::new(Some(3), None, None, None).unwrap();
    let mut mcts = engine(UniformEvaluator, config(Selection::Puct, 1, 32));
    assert_eq!(mcts.search(&snapshot, &depth), Err(MctsError::DepthLimit));
    assert!(matches!(
        mcts.start(snapshot.clone(), depth, 1, false),
        Err((_, MctsError::DepthLimit))
    ));
    let mut mcts = engine(UniformEvaluator, config(Selection::Puct, 1, 32));
    assert_eq!(
        mcts.search(&snapshot, &SearchLimits::infinite()),
        Err(MctsError::InfiniteSynchronousSearch)
    );
}

/// 各局面の方策logitを`logits`（[`POLICY_SIZE`]個）、勝率を`value`で埋める評価器。
struct FilledEvaluator {
    logits: Vec<f32>,
    value: f32,
}

impl Evaluator for FilledEvaluator {
    fn evaluate(
        &mut self,
        _: &[f32],
        _: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        for logits in out_policy.as_chunks_mut::<POLICY_SIZE>().0 {
            logits.copy_from_slice(&self.logits);
        }
        out_value.fill(self.value);
        Ok(())
    }
}

// alphazero.md「推論層」: 推論の出力に非有限値があれば、その局面を使わずにエラーとする。
#[test]
fn non_finite_outputs_are_errors() {
    let snapshot = snapshot_of(Position::initial());
    for (logit, value) in [(f32::NAN, 0.5), (0.0, f32::NAN), (0.0, f32::INFINITY)] {
        let evaluator = FilledEvaluator {
            logits: vec![logit; POLICY_SIZE],
            value,
        };
        let mut engine = engine(evaluator, config(Selection::Puct, 1, 32));
        assert_eq!(
            engine.search(&snapshot, &nodes(8)),
            Err(MctsError::Evaluator(EvaluatorError::NonFinite)),
            "logit {logit}, value {value}"
        );
    }
}

// 同節: 探索が使うのは合法手のlogitと勝率だけなので、合法手でないlogitの非有限値はエラーにしない。
#[test]
fn non_finite_logits_of_illegal_moves_are_ignored() {
    let game = Game::new(Rules::ENGINE_DEFAULT);
    let mut logits = vec![f32::NAN; POLICY_SIZE];
    for mv in game.legal_moves() {
        logits[usize::from(policy::label(mv, Color::Black).unwrap())] = 0.0;
    }
    let evaluator = FilledEvaluator { logits, value: 0.5 };
    let mut engine = engine(evaluator, config(Selection::Puct, 1, 32));
    assert!(
        engine
            .search(&snapshot_of(Position::initial()), &nodes(1))
            .is_ok()
    );
}

// ---------------------------------------------------------------------------
// 探索スレッドとイベント
// ---------------------------------------------------------------------------

/// 終了通知までのイベントを受け取る。
fn drain(handle: &SearchHandle) -> Vec<SearchEvent> {
    let mut events = Vec::new();
    loop {
        let event = handle
            .events()
            .recv_timeout(Duration::from_secs(30))
            .unwrap();
        let finished = matches!(event, SearchEvent::Finished { .. });
        events.push(event);
        if finished {
            return events;
        }
    }
}

// alphazero.md「検証」: stopはαβ探索と同じく、進捗の後に外部停止の終了通知を1回だけ返す。
#[test]
fn stop_finishes_an_infinite_search() {
    let snapshot = snapshot_of(Position::initial());
    let mcts = engine(UniformEvaluator, config(Selection::Puct, 4, 32));
    let handle = mcts
        .start(snapshot, SearchLimits::infinite(), 11, false)
        .unwrap_or_else(|(_, error)| panic!("{error}"));
    let first = handle
        .events()
        .recv_timeout(Duration::from_secs(30))
        .unwrap();
    assert!(matches!(
        first,
        SearchEvent::Progress {
            search_id: 11,
            nodes: 1,
            ..
        }
    ));
    handle.request_stop();
    let events = drain(&handle);
    let Some(SearchEvent::Finished {
        search_id,
        stop_reason,
        depth,
        pv,
        ..
    }) = events.last()
    else {
        panic!("the search must finish");
    };
    assert_eq!(*search_id, 11);
    assert_eq!(*stop_reason, StopReason::ExternalStop);
    assert_eq!(*depth as usize, pv.len());
    assert!(
        events[..events.len() - 1]
            .iter()
            .all(|event| matches!(event, SearchEvent::Progress { .. }))
    );
    let (_, result) = handle.join().unwrap();
    assert_eq!(result, Ok(()));
}

// alphazero.md「共存の構造」: 探索木の節点数が上限（UCT_NodeLimit）に達したら、新しい展開をやめて
// ノード数の制限として終える。UCT_NodeLimitの下限は10万なので、上限は設定へ直接与える。
#[test]
fn tree_capacity_stops_the_search() {
    let snapshot = snapshot_of(Position::initial());
    let mut config = config(Selection::Puct, 4, 32);
    config.max_nodes = NonZeroU32::new(10).unwrap();
    let handle = engine(UniformEvaluator, config)
        .start(snapshot, nodes(1_000), 3, false)
        .unwrap_or_else(|(_, error)| panic!("{error}"));
    let events = drain(&handle);
    let Some(SearchEvent::Finished { stop_reason, .. }) = events.last() else {
        panic!("the search must finish");
    };
    assert_eq!(*stop_reason, StopReason::NodeLimit);
    let (searcher, result) = handle.join().unwrap();
    assert_eq!(result, Ok(()));
    let Searcher::Mcts(engine) = searcher else {
        panic!("the MCTS search returns its own state");
    };
    assert_eq!(engine.previous_tree().unwrap().len(), 10);
}

// alphazero.md「検証」: 先読み中は時間制限を無効とし、ponderhitの後に時間で終了する。
#[test]
fn ponderhit_starts_the_clock() {
    let snapshot = snapshot_of(Position::initial());
    let limits = SearchLimits::new(None, None, Some(30), None).unwrap();
    let mcts = engine(UniformEvaluator, config(Selection::Puct, 4, 32));
    let handle = mcts
        .start(snapshot, limits, 5, true)
        .unwrap_or_else(|(_, error)| panic!("{error}"));
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        handle
            .events()
            .try_iter()
            .all(|event| matches!(event, SearchEvent::Progress { .. }))
    );
    handle.ponderhit();
    let events = drain(&handle);
    let Some(SearchEvent::Finished { stop_reason, .. }) = events.last() else {
        panic!("the search must finish");
    };
    assert_eq!(*stop_reason, StopReason::SoftLimit);
    let (_, result) = handle.join().unwrap();
    assert_eq!(result, Ok(()));
}

// 時間制御はαβ探索と同じ予算式でsoftの時刻を求め、そこで止まる。
#[test]
fn clock_limits_stop_the_search() {
    let snapshot = snapshot_of(Position::initial());
    let clock = ClockLimits::new(2_000, 0, 0, 0).unwrap();
    let limits = SearchLimits::new(None, None, None, Some(clock)).unwrap();
    let mut mcts = engine(UniformEvaluator, config(Selection::Puct, 4, 32));
    let started = std::time::Instant::now();
    let result = mcts.search(&snapshot, &limits).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(result.nodes > 1);
}

// 指示書P第2項: Const_Playoutが正なら、時間制御と無期限探索の制限に代えて、根の展開を含めたシミュレーション
// 回数がその値に達した時点でノード数の制限として止まる。go nodesの明示はConst_Playoutより優先する。
#[test]
fn const_playout_replaces_time_and_infinite_limits() {
    let clock = ClockLimits::new(1_000_000, 0, 0, 0).unwrap();
    for (limits, expected) in [
        (
            SearchLimits::new(None, None, None, Some(clock)).unwrap(),
            50,
        ),
        (
            SearchLimits::new(None, None, Some(1_000_000), None).unwrap(),
            50,
        ),
        (SearchLimits::infinite(), 50),
        (
            SearchLimits::new(None, Some(20), None, Some(clock)).unwrap(),
            20,
        ),
    ] {
        let mut mcts = engine(UniformEvaluator, config(Selection::Puct, 4, 32));
        assert_eq!(mcts.set_option("Const_Playout", Some("50")), Some(Ok(())));
        let handle = mcts
            .start(snapshot_of(Position::initial()), limits, 1, false)
            .unwrap_or_else(|(_, error)| panic!("{error}"));
        let events = drain(&handle);
        let Some(SearchEvent::Finished {
            nodes, stop_reason, ..
        }) = events.last()
        else {
            panic!("the search must finish");
        };
        assert_eq!((*nodes, *stop_reason), (expected, StopReason::NodeLimit));
        let (_, result) = handle.join().unwrap();
        assert_eq!(result, Ok(()));
    }
}

// ---------------------------------------------------------------------------
// 対局の完走
// ---------------------------------------------------------------------------

/// 一様評価器と同じ値を返し、1回の推論にまとめた局面数の最大値を記録する評価器。
struct BatchRecorder {
    max_batch: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Evaluator for BatchRecorder {
    fn evaluate(
        &mut self,
        inputs: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        self.max_batch
            .fetch_max(batch, std::sync::atomic::Ordering::Relaxed);
        UniformEvaluator.evaluate(inputs, batch, out_policy, out_value)
    }
}

// alphazero.md「フェーズ1　基盤」: 一様評価器のMCTSが、初期局面から1手ずつ指して対局を完走する。
// 着手は審判層で合法性を照合し、手数上限で打ち切る。上限600手は、この設定の2局が
// 終局する手数（200手未満）より十分長い。PUCTは仮想損失で複数の葉を1回の推論にまとめ、
// Gumbelは葉を1つずつ評価する（alphazero.md「対局ではPUCTを使う」）。
#[test]
fn uniform_mcts_plays_a_game() {
    const MAX_PLIES: u32 = 600;
    for selection in [Selection::Puct, Selection::Gumbel] {
        let max_batch = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorder = BatchRecorder {
            max_batch: std::sync::Arc::clone(&max_batch),
        };
        let mut game = Game::with_default_rules();
        let mut engine = engine(recorder, config(selection, 8, 16));
        while matches!(game.status(), GameStatus::Ongoing) && game.ply_count() < MAX_PLIES {
            let snapshot = SearchSnapshot::from_game(&game).unwrap();
            let result = engine.search(&snapshot, &nodes(64)).unwrap();
            assert!(snapshot.root_moves().contains(&result.best_move));
            game.play(result.best_move).unwrap();
        }
        assert!(game.ply_count() > 0);
        let max_batch = max_batch.load(std::sync::atomic::Ordering::Relaxed);
        match selection {
            Selection::Puct => assert!(max_batch > 1 && max_batch <= 8, "{max_batch}"),
            Selection::Gumbel => assert_eq!(max_batch, 1),
        }
    }
}

// alphazero.md「検証」: 手数の上限に近い着手列を1手ずつ与えても、1手あたりの固定費が時間切れの
// 安全余裕の内側に収まる（docs/lessons/per-move-overhead-grows-with-game-length.md）。
// 履歴の鍵を1つずつ延ばしながら、対局管理層と同じくスナップショットを作って探索回数1で探索し、
// 履歴の鍵が10個のときと3,500個のときの所要時間の最小値の差が、秒読みの安全余裕（既定30 ms）の
// 10分の1以下であることを確かめる。鍵は反復にならないよう、互いに異なる人工の値を並べる。
// 時間の揺らぎを避けるため、各長さで最初の1回を捨て、残りの最小値を比べる。
#[test]
fn per_move_overhead_does_not_grow_with_history_length() {
    const ROUNDS: usize = 30;
    const LONG: usize = 3_500;
    let game = Game::with_default_rules();
    let keys: Vec<u64> = (1..=(LONG + ROUNDS) as u64)
        .map(minase_core::rng::splitmix64)
        .collect();
    let fastest = |length: usize| {
        let mut engine = engine(UniformEvaluator, config(Selection::Puct, 64, 32));
        (0..=ROUNDS)
            .map(|round| {
                let started = Instant::now();
                let snapshot = SearchSnapshot::from_parts(
                    game.position().clone(),
                    game.rules().moves,
                    keys[..length + round].to_vec(),
                    game.legal_moves(),
                )
                .unwrap();
                let result = engine.search(&snapshot, &nodes(1)).unwrap();
                assert_eq!(result.nodes, 1);
                started.elapsed()
            })
            .skip(1)
            .min()
            .unwrap()
    };
    let short = fastest(10);
    let long = fastest(LONG);
    let margin = Duration::from_millis(ClockLimits::DEFAULT_BYOYOMI_MARGIN_MS);
    assert!(
        long.saturating_sub(short) <= margin / 10,
        "10 keys: {short:?}, {LONG} keys: {long:?}"
    );
}

// ---------------------------------------------------------------------------
// 推論を呼び出し側に任せるGumbel探索
// ---------------------------------------------------------------------------

/// 入力平面から決定的に作った、局面ごとに異なる方策logitと勝率を返す評価器。
struct InputHashEvaluator;

impl Evaluator for InputHashEvaluator {
    fn evaluate(
        &mut self,
        inputs: &[f32],
        batch: usize,
        out_policy: &mut [f32],
        out_value: &mut [f32],
    ) -> Result<(), EvaluatorError> {
        for row in 0..batch {
            let planes = &inputs[row * INPUT_SIZE..(row + 1) * INPUT_SIZE];
            let hash = planes
                .iter()
                .enumerate()
                .filter(|&(_, &value)| value != 0.0)
                .fold(0_u64, |hash, (index, _)| {
                    minase_core::rng::splitmix64(hash ^ index as u64)
                });
            for (label, logit) in out_policy[row * POLICY_SIZE..(row + 1) * POLICY_SIZE]
                .iter_mut()
                .enumerate()
            {
                *logit = (minase_core::rng::splitmix64(hash ^ label as u64) % 64) as f32 / 16.0;
            }
            out_value[row] = (hash % 1000) as f32 / 1000.0;
        }
        Ok(())
    }
}

// alphazero.md「強化学習」: 推論を呼び出し側に任せる探索は、同じ乱数のシードのGumbel探索
// （Engine）と同じ手、同じ方策の目標を返し、葉を1つずつ評価に出す。
#[test]
fn stepped_gumbel_search_matches_the_engine() {
    use minase_core::rng::XorShift64;

    use super::stepped::{Step, SteppedGumbelSearch};

    let mut game = Game::with_default_rules();
    for _ in 0..30 {
        let snapshot = SearchSnapshot::from_game(&game).unwrap();
        let mut engine = engine(InputHashEvaluator, config(Selection::Gumbel, 8, 16));
        let expected = engine.search(&snapshot, &nodes(1_000)).unwrap();
        let expected_target = engine.policy_target().unwrap();

        let mut search = SteppedGumbelSearch::new(&snapshot, NonZeroU32::new(16).unwrap()).unwrap();
        let mut rng = XorShift64::new(NonZeroU64::new(7).unwrap());
        let mut evaluations = 0;
        let searched = loop {
            match search.advance(&mut rng).unwrap() {
                Step::Evaluate => {
                    let input = search.pending_input();
                    assert_eq!(input.len(), INPUT_SIZE);
                    let mut policy = vec![0.0; POLICY_SIZE];
                    let mut value = [0.0];
                    InputHashEvaluator
                        .evaluate(input, 1, &mut policy, &mut value)
                        .unwrap();
                    search.supply(&policy, value[0]).unwrap();
                    evaluations += 1;
                }
                Step::Finished(searched) => break searched,
            }
        };
        assert_eq!(searched.best_move, expected.best_move);
        assert_eq!(searched.policy_target, expected_target);
        assert!((0.0..=1.0).contains(&searched.root_value));
        assert!(evaluations <= 17, "{evaluations}");
        if matches!(
            game.play(searched.best_move).unwrap(),
            GameStatus::Finished(_)
        ) {
            break;
        }
    }
}

//! strength-stage12.md「項目5」「検証」の持ち越し契約。

use super::*;
use crate::search::HistoryTables;
use crate::search::alphabeta::params;

// 停止済みでも根の開始は1回。全要素、両手番、各ワーカーで0方向に丸める。
#[test]
fn history_carry_ages_every_worker_once_before_the_first_iteration() {
    let snapshot = snapshot_for(&Position::initial());
    let mut histories = HistoryTables::new(worker_count(4));
    let values = [-7, -4, -3, -1, 0, 1, 3, 4, 7, 100];
    let expected = values.map(|value| value * params::history_decay() / 100);
    for (worker, history) in histories.workers.iter_mut().enumerate() {
        for (i, value) in history.iter_mut().flatten().flatten().enumerate() {
            *value = values[(i + worker) % values.len()];
        }
    }
    let outcome = run_search_team(
        &weights().unwrap(),
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(3),
        &AtomicBool::new(true),
        worker_count(4),
        &small_tt(),
        &mut histories,
        None,
        Instant::now(),
        &AtomicU64::new(0),
        false,
    );
    assert_eq!(outcome.result.nodes, 0);
    for (worker, history) in histories.workers.iter().enumerate() {
        for (i, value) in history.iter().flatten().flatten().enumerate() {
            assert_eq!(*value, expected[(i + worker) % expected.len()]);
        }
    }
}

// 学習に使われない初期局面の空升→同じ空升を印とし、反復中の再減衰を検出する。
#[test]
fn history_carry_survives_handle_round_trips_and_ages_once_per_go() {
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = HistoryTables::new(threads);
        for (worker, history) in histories.workers.iter_mut().enumerate() {
            history[0][60][60] = 160 + worker as i32 * 16;
        }
        let mut table = small_tt();
        let mut expected: Vec<_> = (0..threads.get())
            .map(|worker| 160 + worker as i32 * 16)
            .collect();
        for search_id in [801, 802] {
            for value in &mut expected {
                *value = *value * params::history_decay() / 100;
            }
            let handle = crate::search::start_search(
                weights().unwrap(),
                snapshot_for(&Position::initial()),
                depth_limits(3),
                search_id,
                threads,
                table,
                histories,
                false,
            );
            let (_, result) = event_reports(drain_raw(&handle));
            assert_eq!(result.depth, 3);
            (table, histories) = handle.join().unwrap();
            for (worker, history) in histories.workers.iter().enumerate() {
                assert_eq!(history[0][60][60], expected[worker]);
            }
        }
    }
}

// 持ち越しの影響を置換表の影響から分離し、同一局面の新品の表と比較する。
#[test]
fn history_carry_changes_the_second_search_with_a_fresh_transposition_table() {
    const CHILD: &str = "MINASE_HISTORY_CARRY_FRESH_PROCESS";
    let snapshot = snapshot_for(&quiet_midgame());
    if std::env::var_os(CHILD).is_some() {
        let fresh = crate::search::search(
            &weights().unwrap(),
            &snapshot,
            &depth_limits(4),
            DEFAULT_THREADS,
            &mut small_tt(),
            &mut HistoryTables::new(DEFAULT_THREADS),
        )
        .unwrap();
        println!("fresh_nodes={}", fresh.nodes);
        return;
    }
    let mut histories = HistoryTables::new(DEFAULT_THREADS);
    let mut table = small_tt();
    let mut nodes = Vec::new();
    for search_id in [810, 811] {
        let handle = crate::search::start_search(
            weights().unwrap(),
            snapshot.clone(),
            depth_limits(4),
            search_id,
            DEFAULT_THREADS,
            table,
            histories,
            false,
        );
        let (_, result) = event_reports(drain_raw(&handle));
        nodes.push(result.nodes);
        (table, histories) = handle.join().unwrap();
        assert!(
            histories.workers[0]
                .iter()
                .flatten()
                .flatten()
                .any(|&v| v != 0)
        );
        table.clear();
    }
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "search::alphabeta::tests::history::history_carry_changes_the_second_search_with_a_fresh_transposition_table", "--nocapture"])
        .env(CHILD, "1")
        .output().unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let stdout = String::from_utf8(child.stdout).unwrap();
    let fresh_nodes: u64 = stdout
        .lines()
        .find_map(|line| line.strip_prefix("fresh_nodes="))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(nodes[0], fresh_nodes);
    assert_ne!(
        nodes[1], fresh_nodes,
        "carried history must influence search"
    );
    eprintln!(
        "history carry: first={} carried={} fresh_process={}",
        nodes[0], nodes[1], fresh_nodes
    );
}

// ワーカーへ貸す表は番号で固定され、書き込みも同じ表へ返る。
#[test]
fn history_carry_workers_update_their_own_table() {
    let stop = AtomicBool::new(false);
    let shared = SharedSearch {
        external_stop: &stop,
        team_stop: AtomicBool::new(false),
        stop_reason: AtomicU8::new(0),
        total_nodes: AtomicU64::new(0),
        node_limit: None,
        started: Instant::now(),
        hard_limit: None,
    };
    let mut histories = HistoryTables::new(worker_count(4));
    for (index, history) in histories.workers.iter_mut().enumerate() {
        history[1][50][51] = index as i32 + 10;
    }
    let mv = legal_moves(&Position::initial())[0];
    run_worker_team(&mut histories.workers, &shared, |index, history| {
        assert_eq!(history[1][50][51], index as i32 + 10);
        history[1][50][51] += 100;
        WorkerOutcome {
            partial_score: None,
            worker_index: index,
            result: SearchResult {
                #[cfg(feature = "search-stats")]
                stats: crate::search::SearchStats::default(),
                best_move: mv,
                score: 0,
                depth: 1,
                nodes: 0,
            },
            pv: vec![mv],
            nodes: 0,
        }
    });
    for (index, history) in histories.workers.iter().enumerate() {
        assert_eq!(history[1][50][51], index as i32 + 110);
    }
}

/// search-revival-spsa.md「フェーズ1」: 連続する探索の間で全履歴が消える。
#[cfg(feature = "tuning")]
pub(super) fn zero_decay_clears_history_between_searches() {
    let snapshot = snapshot_for(&quiet_midgame());
    let pst = weights().unwrap();
    for threads in [DEFAULT_THREADS, worker_count(4)] {
        let mut histories = HistoryTables::new(threads);
        crate::search::search(
            &pst,
            &snapshot,
            &depth_limits(3),
            threads,
            &mut small_tt(),
            &mut histories,
        )
        .unwrap();
        assert!(
            histories
                .workers
                .iter()
                .any(|h| h.iter().flatten().flatten().any(|&v| v != 0))
        );
        // 2回目は新しい履歴を書き込む前に止め、前回の全要素が消えたことを確かめる。
        run_search_team(
            &pst,
            &snapshot.position,
            snapshot.rules,
            &snapshot.root_moves,
            &snapshot.history_keys,
            &depth_limits(3),
            &AtomicBool::new(true),
            threads,
            &small_tt(),
            &mut histories,
            None,
            Instant::now(),
            &AtomicU64::new(0),
            false,
        );
        assert!(
            histories
                .workers
                .iter()
                .all(|h| h.iter().flatten().flatten().all(|&v| v == 0))
        );
    }
    // 単一ワーカーの実探索でも、履歴を再利用した2回目と新品の表の結果が一致する。
    let mut histories = HistoryTables::new(DEFAULT_THREADS);
    let first = crate::search::search(
        &pst,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut histories,
    )
    .unwrap();
    let second = crate::search::search(
        &pst,
        &snapshot,
        &depth_limits(3),
        DEFAULT_THREADS,
        &mut small_tt(),
        &mut histories,
    )
    .unwrap();
    assert_eq!(first, second);
}

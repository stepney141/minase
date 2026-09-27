//! debugging-tools.md「探索統計の項目」の加算条件を検査する。

use super::*;
use crate::search::SearchStats;

fn quiet_position() -> Position {
    position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(6, 10), Color::Black, PieceKind::GoldGeneral),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    )
}

fn ranks(stats: SearchStats) -> [u64; 4] {
    [
        stats.best_move_rank_1,
        stats.best_move_rank_2,
        stats.best_move_rank_3,
        stats.best_move_rank_4_plus,
    ]
}

fn assert_relations(stats: SearchStats) {
    assert!(stats.first_move_beta_cutoffs <= stats.beta_cutoffs);
    assert!(stats.beta_cutoffs <= stats.normal_nodes);
    assert!(stats.tt_cutoffs <= stats.tt_hits);
    assert!(stats.tt_hits <= stats.tt_probes);
    assert!(stats.tt_probes <= stats.normal_nodes);
    assert!(stats.quiesce_tt_cutoffs <= stats.quiesce_tt_hits);
    assert!(stats.quiesce_tt_hits <= stats.quiesce_tt_probes);
    assert!(stats.quiesce_tt_probes <= stats.quiesce_nodes);
    assert!(ranks(stats).iter().sum::<u64>() <= stats.normal_nodes);
}

/// debugging-tools.md「探索統計の項目」: 深さ0の移行は静止探索だけに数える。
/// 捕獲なし、stand-pat、最大plyで返る呼び出しも数え、照会は実行時だけ数える。
#[test]
fn depth_zero_and_early_quiesce_returns_count_only_quiesce_calls() {
    let position = quiet_position();
    with_root_searcher(&position, &[], |searcher| {
        let mut current = position.clone();
        searcher
            .negamax(&mut current, 0, -INFINITY, INFINITY, 0)
            .unwrap();
        searcher
            .quiesce(&mut current, -INFINITY, -INFINITY + 1, 0)
            .unwrap();
        searcher
            .quiesce(&mut current, -INFINITY, INFINITY, MAX_PLY)
            .unwrap();
        assert_eq!(
            searcher.stats,
            SearchStats {
                quiesce_nodes: 3,
                ..SearchStats::default()
            }
        );
    });
}

/// debugging-tools.md「探索統計の項目」: 同じ表の2回目の通常探索は一致と打ち切りを各1回数える。
/// 置換表による打ち切りを着手のβカットや順位に混ぜない。
#[test]
fn normal_tt_repeat_counts_probe_hit_and_cutoff() {
    let position = quiet_position();
    with_root_searcher(&position, &[], |searcher| {
        let mut current = position.clone();
        let first = searcher
            .negamax(&mut current, 1, -INFINITY, INFINITY, 0)
            .unwrap();
        assert_eq!(searcher.stats.normal_nodes, 1);
        assert_eq!(searcher.stats.tt_probes, 1);
        assert_eq!(searcher.stats.tt_hits, 0);
        assert!(searcher.stats.quiesce_nodes > 0);
        assert_eq!(ranks(searcher.stats).iter().sum::<u64>(), 1);
        searcher.stats = SearchStats::default();
        assert_eq!(
            searcher.negamax(&mut current, 1, -INFINITY, INFINITY, 0),
            Some(first)
        );
        assert_eq!(
            searcher.stats,
            SearchStats {
                normal_nodes: 1,
                tt_probes: 1,
                tt_hits: 1,
                tt_cutoffs: 1,
                ..SearchStats::default()
            }
        );
    });
}

/// debugging-tools.md「探索統計の項目」: 深さ不足の一致は照会と一致だけに数える。
#[test]
fn insufficient_tt_depth_is_a_hit_without_a_cutoff() {
    let position = quiet_position();
    with_root_searcher(&position, &[], |searcher| {
        searcher
            .tt
            .store(search_key(&position), 0, 0, Bound::Exact, None, 0);
        searcher
            .negamax(&mut position.clone(), 1, -INFINITY, INFINITY, 0)
            .unwrap();
        assert_eq!(searcher.stats.tt_probes, 1);
        assert_eq!(searcher.stats.tt_hits, 1);
        assert_eq!(searcher.stats.tt_cutoffs, 0);
        assert_relations(searcher.stats);
    });
}

/// debugging-tools.md「探索統計の項目」: 捕獲を探索した後の再照会は静止探索の3項目に数える。
#[test]
fn quiesce_tt_repeat_counts_probe_hit_and_cutoff_separately() {
    let position = position(
        Color::Black,
        &[
            (fs(6, 12), Color::Black, PieceKind::King),
            (fs(3, 10), Color::Black, PieceKind::Rook),
            (fs(3, 4), Color::White, PieceKind::Pawn),
            (fs(6, 1), Color::White, PieceKind::King),
        ],
    );
    with_root_searcher(&position, &[], |searcher| {
        let mut current = position.clone();
        let first = searcher
            .quiesce(&mut current, -INFINITY, INFINITY, 0)
            .unwrap();
        assert!(searcher.stats.quiesce_nodes > 1);
        assert!(searcher.stats.quiesce_tt_probes > 0);
        assert_eq!(searcher.stats.normal_nodes, 0);
        assert_relations(searcher.stats);
        searcher.stats = SearchStats::default();
        assert_eq!(
            searcher.quiesce(&mut current, -INFINITY, INFINITY, 0),
            Some(first)
        );
        assert_eq!(
            searcher.stats,
            SearchStats {
                quiesce_nodes: 1,
                quiesce_tt_probes: 1,
                quiesce_tt_hits: 1,
                quiesce_tt_cutoffs: 1,
                ..SearchStats::default()
            }
        );
    });
}

/// debugging-tools.md「探索統計の項目」: 全評価値より低いβなら最初の手で切り、順位に含めない。
#[test]
fn first_searched_move_beta_cutoff_is_counted() {
    let position = quiet_position();
    with_root_searcher(&position, &[], |searcher| {
        searcher
            .negamax(&mut position.clone(), 1, -INFINITY, -INFINITY + 1, 0)
            .unwrap();
        assert_eq!(searcher.stats.beta_cutoffs, 1);
        assert_eq!(searcher.stats.first_move_beta_cutoffs, 1);
        assert_eq!(ranks(searcher.stats), [0; 4]);
        assert_relations(searcher.stats);
    });
}

/// debugging-tools.md「探索統計の項目」: ヌルムーブの呼び出しは数えるが、手のβカットに含めない。
#[test]
fn null_move_cutoff_counts_its_quiesce_call_without_a_move_cutoff() {
    let position = quiet_position();
    let mut after_null = position.clone();
    after_null.make_null_move();
    let expected = -evaluate(&weights().unwrap(), &after_null);
    with_root_searcher(&position, &[], |searcher| {
        assert_eq!(
            searcher.negamax(&mut position.clone(), 4, expected - 1, expected, 0),
            Some(expected)
        );
        assert_eq!(searcher.nodes, 0);
        assert_eq!(searcher.stats.normal_nodes, 1);
        assert_eq!(searcher.stats.quiesce_nodes, 1);
        assert_eq!(searcher.stats.beta_cutoffs, 0);
        assert_eq!(ranks(searcher.stats), [0; 4]);
    });
}

/// debugging-tools.md「探索統計の項目」: 全手走査で最後にαを更新した手を4区分に数える。
/// 捕獲のない深さ1では子の静的評価から最善手を決め、探索順だけをhistoryで指定する。
#[test]
fn best_move_rank_histogram_counts_each_bucket_and_research_calls() {
    let position = quiet_position();
    let pst = weights().unwrap();
    let mut scored: Vec<_> = legal_moves(&position)
        .into_iter()
        .map(|mv| {
            let mut child = position.clone();
            child.make_move_unchecked(mv, engine_rules());
            assert!(
                legal_moves(&child)
                    .iter()
                    .all(|&reply| child.captured_squares(reply).iter().all(Option::is_none))
            );
            (mv, -evaluate(&pst, &child))
        })
        .collect();
    scored.sort_by_key(|&(_, score)| score);
    let best = *scored.last().unwrap();
    let lesser: Vec<_> = scored
        .iter()
        .copied()
        .filter(|&(_, score)| score < best.1)
        .collect();
    assert!(lesser.len() >= 4, "4番目以降の前に劣る手を並べられる局面");
    for rank in 1..=5 {
        let mut ordered: Vec<_> = lesser.iter().take(rank - 1).map(|&(mv, _)| mv).collect();
        ordered.push(best.0);
        for &(mv, _) in &scored {
            if !ordered.contains(&mv) {
                ordered.push(mv);
            }
        }
        with_root_searcher(&position, &[], |searcher| {
            for (index, mv) in ordered.iter().enumerate() {
                searcher.history[position.side_to_move().index()][mv.from.dense_index()]
                    [mv.to.dense_index()] = (ordered.len() - index) as i32;
            }
            assert_eq!(
                searcher.negamax(&mut position.clone(), 1, -INFINITY, INFINITY, 0),
                Some(best.1)
            );
            let mut expected = [0; 4];
            expected[(rank - 1).min(3)] = 1;
            assert_eq!(ranks(searcher.stats), expected, "探索順位{rank}");
            assert_eq!(ranks(searcher.stats).iter().sum::<u64>(), 1);
            assert_eq!(searcher.stats.beta_cutoffs, 0);
            // 2手目以降にαを更新すると零窓から全窓へ再探索する。
            if rank > 1 {
                assert!(searcher.stats.quiesce_nodes > ordered.len() as u64);
            }
            assert_relations(searcher.stats);
        });
        // 同じ順序でβを最善値に置くと、その順位で打ち切り、度数には入らない。
        with_root_searcher(&position, &[], |searcher| {
            for (index, mv) in ordered.iter().enumerate() {
                searcher.history[position.side_to_move().index()][mv.from.dense_index()]
                    [mv.to.dense_index()] = (ordered.len() - index) as i32;
            }
            assert_eq!(
                searcher.negamax(&mut position.clone(), 1, -INFINITY, best.1, 0),
                Some(best.1)
            );
            assert_eq!(searcher.stats.beta_cutoffs, 1);
            assert_eq!(searcher.stats.first_move_beta_cutoffs, u64::from(rank == 1));
            assert_eq!(ranks(searcher.stats), [0; 4]);
        });
    }
}

/// debugging-tools.md「探索統計の項目」: 下限更新なしや中断した走査は順位に含めない。
#[test]
fn fail_low_and_interrupted_nodes_do_not_enter_rank_histogram() {
    let position = quiet_position();
    with_root_searcher(&position, &[], |searcher| {
        searcher
            .negamax(&mut position.clone(), 1, INFINITY - 1, INFINITY, 0)
            .unwrap();
        assert_eq!(ranks(searcher.stats), [0; 4]);
    });
    with_root_searcher(&position, &[], |searcher| {
        searcher
            .shared
            .external_stop
            .store(true, AtomicOrdering::Relaxed);
        assert_eq!(
            searcher.negamax(&mut position.clone(), 1, -INFINITY, INFINITY, 0),
            None
        );
        assert_eq!(ranks(searcher.stats), [0; 4]);
    });
}

/// debugging-tools.md「探索統計の項目」: Threads=2の実探索で全ワーカー終了後の値を合算する。
/// AddAssignを期待値の計算に使わず、各項目を独立に足して照合する。
#[test]
fn two_worker_search_stats_equal_the_sum_of_worker_counts() {
    let snapshot = snapshot_for(&Position::initial());
    let outcome = run_search_team(
        &weights().unwrap(),
        &snapshot.position,
        snapshot.rules,
        &snapshot.root_moves,
        &snapshot.history_keys,
        &depth_limits(5),
        &AtomicBool::new(false),
        worker_count(2),
        &small_tt(),
        None,
        Instant::now(),
        &AtomicU64::new(0),
        false,
    );
    assert_eq!(outcome.worker_stats.len(), 2);
    let counts = |stats: SearchStats| -> Vec<(String, u64)> {
        stats
            .to_string()
            .split_whitespace()
            .skip(1)
            .map(|item| {
                let (name, count) = item.split_once('=').unwrap();
                (name.to_owned(), count.parse().unwrap())
            })
            .collect()
    };
    let first = counts(outcome.worker_stats[0]);
    let second = counts(outcome.worker_stats[1]);
    let combined = counts(outcome.result.stats);
    for ((a, b), total) in first.iter().zip(&second).zip(&combined) {
        assert_eq!(a.0, total.0);
        assert_eq!(b.0, total.0);
        assert_eq!(a.1 + b.1, total.1, "{}", total.0);
    }
    assert!(outcome.result.stats.normal_nodes > 0);
    assert!(outcome.result.stats.quiesce_nodes > 0);
    assert_relations(outcome.result.stats);
}

/// debugging-tools.md「探索統計の項目」: Threads=1の同じ固定深さ探索で全項目が一致する。
#[test]
fn single_worker_fixed_depth_stats_are_reproducible() {
    let snapshot = snapshot_for(&Position::initial());
    let run = || {
        crate::search::search(
            &weights().unwrap(),
            &snapshot,
            &depth_limits(4),
            DEFAULT_THREADS,
            &mut small_tt(),
        )
        .unwrap()
        .stats
    };
    let first = run();
    assert_eq!(first, run());
    assert!(first.normal_nodes > 0);
    assert!(first.quiesce_nodes > 0);
    assert!(first.beta_cutoffs > 0);
    assert_relations(first);
}

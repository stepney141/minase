//! 対局ペアの配布と番号順の取り込み。

use minase::harness::CompletedPair;
use minase::stats::GsprtDecision;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

/// ワーカーの原因を、標準エラーへの出力まで型のまま保持する。
#[derive(Debug)]
pub(super) enum WorkerError {
    Match(minase::harness::MatchEngineError),
    SavePair { number: u64, source: std::io::Error },
    Checkpoint(std::io::Error),
    Results(mpsc::RecvError),
    Jobs(mpsc::SendError<u64>),
    Panicked,
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Match(error) => error.fmt(f),
            Self::SavePair { number, source } => {
                write!(f, "failed to save pair {number}: {source}")
            }
            Self::Checkpoint(error) => write!(f, "failed to checkpoint active wall time: {error}"),
            Self::Results(error) => write!(f, "worker result channel disconnected: {error}"),
            Self::Jobs(error) => write!(f, "worker job channel disconnected: {error}"),
            Self::Panicked => f.write_str("a match worker panicked"),
        }
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Match(error) => Some(error),
            Self::SavePair { source, .. } | Self::Checkpoint(source) => Some(source),
            Self::Results(error) => Some(error),
            Self::Jobs(error) => Some(error),
            Self::Panicked => None,
        }
    }
}

/// 最初に検知した失敗と、その後に回収した失敗を回収順に保持する。
#[derive(Debug)]
pub(super) struct WorkerPoolErrors {
    pub(super) first: WorkerError,
    pub(super) later: Vec<WorkerError>,
}

struct StopWorkersOnDrop<'a>(&'a AtomicBool);

impl Drop for StopWorkersOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// 対局ペアを配布し、完了1件につき次のジョブを補充してチェックポイントを記録する。
///
/// 継続判定と完了処理は進捗 `state` を共有する。失敗や打ち切りの後はワーカー
/// 全員を停止・回収し、結果の通信路に残った失敗だけを拾う。
#[allow(clippy::too_many_arguments)]
pub(super) fn run_worker_pool<S>(
    concurrency: usize,
    mut pending_jobs: VecDeque<u64>,
    stop: &AtomicBool,
    mut state: S,
    run: impl Fn(u64, &AtomicBool) -> Result<Option<CompletedPair>, WorkerError> + Sync,
    should_continue: impl Fn(&S) -> bool,
    mut accept: impl FnMut(CompletedPair, &mut S, &mut VecDeque<u64>) -> Option<u64>,
    mut checkpoint: impl FnMut() -> std::io::Result<()>,
) -> Result<(), WorkerPoolErrors> {
    let worker_count = if stop.load(Ordering::Acquire) {
        0
    } else {
        concurrency.min(pending_jobs.len())
    };
    let (job_sender, job_receiver) = mpsc::channel();
    let job_receiver = Mutex::new(job_receiver);
    thread::scope(|scope| {
        let stop_workers = StopWorkersOnDrop(stop);
        let (result_sender, result_receiver) = mpsc::channel();
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let result_sender = result_sender.clone();
            let job_receiver = &job_receiver;
            let run = &run;
            workers.push(scope.spawn(move || {
                let worker_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_worker_loop(job_receiver, &result_sender, stop, run);
                }));
                if worker_result.is_err() {
                    stop.store(true, Ordering::Release);
                    let _ = result_sender.send(Err(WorkerError::Panicked));
                }
            }));
        }
        drop(result_sender);

        for _ in 0..worker_count {
            let pair_number = pending_jobs
                .pop_front()
                .expect("worker count is bounded by pending jobs");
            job_sender
                .send(pair_number)
                .expect("workers must be waiting for initial jobs");
        }
        let mut first = (|| {
            while should_continue(&state) {
                let pair = result_receiver.recv().map_err(WorkerError::Results)??;
                if let Some(job) = accept(pair, &mut state, &mut pending_jobs)
                    && !stop.load(Ordering::Acquire)
                {
                    job_sender.send(job).map_err(WorkerError::Jobs)?;
                }
                checkpoint().map_err(WorkerError::Checkpoint)?;
            }
            Ok::<(), WorkerError>(())
        })()
        .err();

        drop(stop_workers);
        drop(job_sender);
        let mut worker_panicked = false;
        for worker in workers {
            worker_panicked |= worker.join().is_err();
        }
        let mut later = Vec::new();
        for error in result_receiver
            .try_iter()
            .filter_map(Result::err)
            .chain(worker_panicked.then_some(WorkerError::Panicked))
        {
            if first.is_none() {
                first = Some(error);
            } else {
                later.push(error);
            }
        }
        match first {
            Some(first) => Err(WorkerPoolErrors { first, later }),
            None => Ok(()),
        }
    })
}

/// 完了したペアを番号順に取り込み、実験を続ける場合は次の1件を返す。
///
/// ジョブの補充は統計へ取り込めた件数ではなく、完了結果を1件受信した事実に
/// 対応させる。これにより、若い番号のペアが遅れても空いた並列枠を維持する。
pub(super) fn accept_completed_pair(
    pair: CompletedPair,
    completed: &mut BTreeMap<u64, CompletedPair>,
    next_to_integrate: &mut u64,
    pending_jobs: &mut VecDeque<u64>,
    mut integrate: impl FnMut(CompletedPair) -> bool,
) -> Option<u64> {
    assert!(
        completed.insert(pair.number, pair).is_none(),
        "a pair number must be completed at most once"
    );
    while let Some(pair) = completed.remove(next_to_integrate) {
        *next_to_integrate = next_to_integrate
            .checked_add(1)
            .expect("pair number overflow");
        if !integrate(pair) {
            return None;
        }
    }

    pending_jobs.pop_front()
}

/// GSPRTの境界に達したら停止を全ワーカーへ通知し、継続しないことを返す。
///
/// 呼び出し側は新しいペアを投入せず、停止ペア番号より後の結果は保存しても統計へ入れない。
/// 番号順の取り込みは[`accept_completed_pair`]に従う。
pub(super) fn continue_after_decision(
    use_gsprt: bool,
    decision: GsprtDecision,
    stop: &AtomicBool,
) -> bool {
    let keep_running = !use_gsprt || decision == GsprtDecision::Continue;
    if !keep_running {
        stop.store(true, Ordering::Release);
    }
    keep_running
}

/// ジョブを受信してペアを実行し、停止で打ち切られなかった結果だけを送る。
fn run_worker_loop(
    job_receiver: &Mutex<Receiver<u64>>,
    result_sender: &mpsc::Sender<Result<CompletedPair, WorkerError>>,
    stop: &AtomicBool,
    mut run: impl FnMut(u64, &AtomicBool) -> Result<Option<CompletedPair>, WorkerError>,
) {
    loop {
        let job = job_receiver
            .lock()
            .expect("the job receiver mutex must not be poisoned")
            .recv();
        let Ok(pair_number) = job else {
            break;
        };
        let pair = match run(pair_number, stop) {
            Ok(Some(pair)) => pair,
            Ok(None) => break,
            Err(error) => {
                stop.store(true, Ordering::Release);
                let _ = result_sender.send(Err(error));
                break;
            }
        };
        if result_sender.send(Ok(pair)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase::harness::*;
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    fn completed_pair(number: u64, category: usize) -> CompletedPair {
        let game = GameRecord {
            candidate_color: StoredColor::Black,
            candidate_seed: 1,
            baseline_seed: 2,
            wall_time_ns: 1,
            candidate_cpu_time_ns: Some(1),
            baseline_cpu_time_ns: Some(1),
            candidate_peak_rss_bytes: Some(1),
            baseline_peak_rss_bytes: Some(1),
            turns: Vec::new(),
            termination: TerminationRecord::Cutoff,
        };
        CompletedPair {
            number,
            output: String::new(),
            result: PairResult {
                category: Some(category),
                failures: FailureCounts::default(),
            },
            record: PairRecord {
                pair_number: number,
                pair_seed: 1,
                opening: OpeningRecord {
                    seed: 1,
                    moves: Vec::new(),
                },
                games: [game.clone(), game],
                category: Some(u8::try_from(category).unwrap()),
            },
        }
    }

    struct SetAtomicOnDrop(Arc<AtomicBool>);

    impl Drop for SetAtomicOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn worker_pool_keeps_both_concurrent_errors_and_stops() {
        let both_started = Barrier::new(2);
        let stop = AtomicBool::new(false);
        let third_started = AtomicBool::new(false);
        let errors = run_worker_pool(
            2,
            VecDeque::from([1, 2, 3]),
            &stop,
            (),
            |number, _| {
                if number == 3 {
                    third_started.store(true, Ordering::Relaxed);
                    return Ok(None);
                }
                both_started.wait();
                Err(WorkerError::SavePair {
                    number,
                    source: std::io::ErrorKind::PermissionDenied.into(),
                })
            },
            |()| true,
            |_, (), _| unreachable!("failed jobs cannot produce completed pairs"),
            || unreachable!("failed jobs cannot trigger a checkpoint"),
        )
        .unwrap_err();

        assert!(
            matches!(
                (&errors.first, errors.later.as_slice()),
                (
                    WorkerError::SavePair { number: 1, .. },
                    [WorkerError::SavePair { number: 2, .. }],
                ) | (
                    WorkerError::SavePair { number: 2, .. },
                    [WorkerError::SavePair { number: 1, .. }],
                )
            ),
            "{errors:?}"
        );
        assert!(!third_started.load(Ordering::Relaxed));
        assert!(stop.load(Ordering::Acquire));
    }

    #[test]
    fn worker_pool_returns_panicked_error_and_stops() {
        let stop = AtomicBool::new(false);
        let errors = run_worker_pool(
            1,
            VecDeque::from([1]),
            &stop,
            (),
            |_, _| panic!("test worker panic"),
            |()| true,
            |_, (), _| unreachable!("panicked jobs cannot produce completed pairs"),
            || unreachable!("panicked jobs cannot trigger a checkpoint"),
        )
        .unwrap_err();

        assert!(matches!(errors.first, WorkerError::Panicked), "{errors:?}");
        assert!(errors.later.is_empty(), "{errors:?}");
        assert!(stop.load(Ordering::Acquire));
    }

    // [`accept_completed_pair`]は遅い先頭ペアを待つ間も、完了1件につき並列枠を1件補充する。
    #[test]
    fn out_of_order_completions_replenish_one_slot_each() {
        let mut completed = BTreeMap::new();
        let mut next_to_integrate = 1;
        let mut pending_jobs = VecDeque::from([3, 4, 5]);
        let mut integrated = Vec::new();

        let replacement = accept_completed_pair(
            completed_pair(2, 2),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                true
            },
        );
        assert_eq!(replacement, Some(3));
        assert!(integrated.is_empty());

        let replacement = accept_completed_pair(
            completed_pair(3, 2),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                true
            },
        );
        assert_eq!(replacement, Some(4));
        assert!(integrated.is_empty());

        let replacement = accept_completed_pair(
            completed_pair(1, 2),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                true
            },
        );
        assert_eq!(replacement, Some(5));
        assert_eq!(integrated, [1, 2, 3]);
        assert_eq!(next_to_integrate, 4);
        assert!(completed.is_empty());
    }

    // [`continue_after_decision`]は境界で停止し、ワーカーは未完了ペアを結果へ送らない。
    #[test]
    fn decision_stop_discards_an_unfinished_worker_result() {
        let stop = Arc::new(AtomicBool::new(false));
        thread::scope(|scope| {
            let (job_sender, job_receiver) = mpsc::channel();
            let job_receiver = Mutex::new(job_receiver);
            let (result_sender, result_receiver) = mpsc::channel();
            let (started_sender, started_receiver) = mpsc::channel();
            let worker_stop = Arc::clone(&stop);
            let worker = scope.spawn(move || {
                run_worker_loop(
                    &job_receiver,
                    &result_sender,
                    &worker_stop,
                    |number, stop| {
                        started_sender.send(number).unwrap();
                        while !stop.load(Ordering::Acquire) {
                            thread::yield_now();
                        }
                        Ok(None)
                    },
                );
            });
            let _stop_on_unwind = SetAtomicOnDrop(Arc::clone(&stop));

            job_sender.send(1).unwrap();
            assert_eq!(
                started_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap(),
                1
            );
            assert!(!continue_after_decision(
                true,
                GsprtDecision::AcceptH1,
                &stop
            ));
            drop(job_sender);
            worker.join().unwrap();
            assert!(matches!(
                result_receiver.try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ));
        });
    }

    // [`accept_completed_pair`]は番号順で境界を判定し、境界到達後は補充しない。
    // 理由は`docs/adr/judge-adoption-by-pentanomial-gsprt.md`にある。
    #[test]
    fn decision_boundary_prevents_replenishment() {
        let mut completed = BTreeMap::new();
        let mut next_to_integrate = 1;
        let mut pending_jobs = (2..=10).collect::<VecDeque<_>>();
        let mut integrated = Vec::new();

        let replacement = accept_completed_pair(
            completed_pair(1, 4),
            &mut completed,
            &mut next_to_integrate,
            &mut pending_jobs,
            |pair| {
                integrated.push(pair.number);
                false
            },
        );
        assert_eq!(replacement, None);
        assert_eq!(integrated, [1]);
        assert_eq!(pending_jobs.front(), Some(&2));
    }
}

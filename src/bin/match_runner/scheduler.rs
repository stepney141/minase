//! 対局ペアの配布と番号順の取り込み。

use minase::harness::CompletedPair;
use minase::stats::GsprtDecision;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

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

/// GSPRTの判定に応じて実験を続けるかを返し、停止時は全ワーカーへ通知する。
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
pub(super) fn run_worker_loop(
    job_receiver: &Mutex<Receiver<u64>>,
    result_sender: &mpsc::Sender<Result<CompletedPair, String>>,
    stop: &AtomicBool,
    mut run: impl FnMut(u64, &AtomicBool) -> Result<Option<CompletedPair>, String>,
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
    use minase::stats::{gsprt_decision, gsprt_llr};
    use std::{sync::Arc, thread, time::Duration};

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

    // match-harness-efficiency.md「並列枠の維持」: ペア1が遅れても、後続結果を
    // 1件受信するたびに空いた枠へ次のペアを1件だけ補充する。統計への取り込みは
    // ペア番号順を維持する。
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

    // match-harness-efficiency.md「並列枠の維持」: GSPRT境界は停止フラグを
    // 設定し、実行中のワーカーが打ち切った未完了ペアを結果へ送らない。
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

    // match-harness-efficiency.md「並列枠の維持」: 番号順の取り込みでGSPRT境界へ
    // 到達した場合は、同じ受信によって空いた枠へ新しいペアを投入しない。
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

    // match-harness-efficiency.md「並列枠の維持」: 同じ固定結果列は、完了順が
    // 入れ替わってもペンタノミアル度数、LLR、判定、停止ペア番号が一致する。
    #[test]
    fn completion_order_does_not_change_gsprt_stopping_result() {
        fn integrate_arrivals(
            arrivals: impl IntoIterator<Item = u64>,
        ) -> ([u64; 5], f64, GsprtDecision, u64) {
            let mut completed = BTreeMap::new();
            let mut next_to_integrate = 1;
            let mut pending_jobs = VecDeque::new();
            let mut results = [0_u64; 5];
            let mut decision = GsprtDecision::Continue;
            let mut stop_pair = 0;

            for number in arrivals {
                if decision != GsprtDecision::Continue {
                    break;
                }
                let replacement = accept_completed_pair(
                    completed_pair(number, 4),
                    &mut completed,
                    &mut next_to_integrate,
                    &mut pending_jobs,
                    |pair| {
                        let category = pair
                            .result
                            .category
                            .expect("the synthetic result must be valid");
                        results[category] += 1;
                        stop_pair = pair.number;
                        decision = gsprt_decision(gsprt_llr(&results));
                        decision == GsprtDecision::Continue
                    },
                );
                assert_eq!(replacement, None);
            }

            assert_ne!(decision, GsprtDecision::Continue);
            (results, gsprt_llr(&results), decision, stop_pair)
        }

        let sequential = integrate_arrivals(1..=1_000);
        let delayed_head = integrate_arrivals((2..=1_000).chain(std::iter::once(1)));
        assert_eq!(sequential.0, delayed_head.0);
        assert_eq!(sequential.1, delayed_head.1);
        assert_eq!(sequential.2, delayed_head.2);
        assert_eq!(sequential.3, delayed_head.3);
    }
}

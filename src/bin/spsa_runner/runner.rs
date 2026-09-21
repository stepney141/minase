//! ペア単位の割当と、反復の完了順での更新。

use super::{
    model::{Issued, IterationRecord, PairObservation, PairValues, Settings, issue},
    storage::{invalid, validate_chain},
};
use minase::harness::{CompletedPair, FailureCounts};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

#[derive(Debug)]
pub(super) struct Summary {
    pub theta: Vec<f64>,
    pub applied: u64,
    pub valid_pairs: u64,
    pub discarded_pairs: u64,
    pub failures: FailureCounts,
}
impl Summary {
    fn add(&mut self, record: &IterationRecord) {
        self.theta.clone_from(&record.theta);
        self.applied += 1;
        self.failures.add(record.failures);
        for pair in &record.pairs {
            if pair.category.is_some() {
                self.valid_pairs += 1;
            } else {
                self.discarded_pairs += 1;
            }
        }
    }
}

struct Scheduler {
    summary: Summary,
    saved: BTreeSet<u64>,
    next: u64,
    active: BTreeMap<u64, Issued>,
}
impl Scheduler {
    fn take(&mut self, settings: &Settings) -> Option<(u64, PairValues)> {
        for (&k, iteration) in &mut self.active {
            if let Some(pair) = iteration.pending.pop_front() {
                return Some((k, pair));
            }
        }
        while self.next <= settings.iterations && self.saved.contains(&self.next) {
            self.next += 1;
        }
        if self.next > settings.iterations {
            return None;
        }
        let k = self.next;
        self.next += 1;
        let mut iteration = issue(settings, &self.summary.theta, k);
        let pair = iteration.pending.pop_front().expect("positive pair count");
        self.active.insert(k, iteration);
        Some((k, pair))
    }
}

/// エラーまたはパニック時には、スコープがワーカーを回収する前に停止を通知する。
struct StopOnError<'a>(&'a AtomicBool, bool);
impl Drop for StopOnError<'_> {
    fn drop(&mut self) {
        if self.1 {
            self.0.store(true, Ordering::Release);
        }
    }
}

/// エンジン実行と保存を注入できる同一の更新ループを、実行と合成試験に使う。
pub(super) fn run(
    settings: &Settings,
    saved: &BTreeMap<u64, IterationRecord>,
    execute: impl Fn(&PairValues, &AtomicBool) -> io::Result<Option<CompletedPair>> + Sync,
    persist: impl Fn(&IterationRecord) -> io::Result<()> + Sync,
) -> io::Result<Summary> {
    let theta = validate_chain(settings, saved)?;
    let mut summary = Summary {
        theta,
        applied: 0,
        valid_pairs: 0,
        discarded_pairs: 0,
        failures: FailureCounts::default(),
    };
    // 集計だけを復元する。θの正は適用番号順で検査した最終値である。
    for r in saved.values() {
        summary.applied += 1;
        summary.failures.add(r.failures);
        for p in &r.pairs {
            if p.category.is_some() {
                summary.valid_pairs += 1;
            } else {
                summary.discarded_pairs += 1;
            }
        }
    }
    let scheduler = Mutex::new(Scheduler {
        summary,
        saved: saved.keys().copied().collect(),
        next: 1,
        active: BTreeMap::new(),
    });
    let stop = AtomicBool::new(false);
    thread::scope(|scope| -> io::Result<()> {
        let mut scope_guard = StopOnError(&stop, true);
        let mut workers = Vec::new();
        for _ in 0..settings.concurrency {
            workers.push(thread::Builder::new().spawn_scoped(scope, || {
                let mut guard = StopOnError(&stop, true);
                let result = (|| {
                    loop {
                        let job = {
                            let mut state = scheduler
                                .lock()
                                .map_err(|_| io::Error::other("scheduler lock poisoned"))?;
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            state.take(settings)
                        };
                        let Some((k, values)) = job else {
                            break;
                        };
                        let Some(pair) = execute(&values, &stop)? else {
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            return Err(io::Error::other(
                                "pair executor stopped without a stop request",
                            ));
                        };
                        if pair.number != values.number
                            || pair.record.pair_number != values.number
                            || pair.result.category != pair.record.category.map(usize::from)
                            || pair.result.category.is_some_and(|c| c > 4)
                        {
                            return Err(invalid(
                                "pair executor returned inconsistent identity or category",
                            ));
                        }
                        let observation = PairObservation::from_completed(values, pair);
                        let mut state = scheduler
                            .lock()
                            .map_err(|_| io::Error::other("scheduler lock poisoned"))?;
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        let iteration = state.active.get_mut(&k).expect("issued iteration exists");
                        iteration.finished.push(observation);
                        if iteration.finished.len() == settings.pairs_per_iteration {
                            let iteration =
                                state.active.remove(&k).expect("completed iteration exists");
                            let record = iteration.complete(
                                settings,
                                &state.summary.theta,
                                state.summary.applied + 1,
                            );
                            // 確定ファイルができた後にだけ、次の発行が使うθを進める。
                            if let Err(error) = persist(&record) {
                                stop.store(true, Ordering::Release);
                                return Err(error);
                            }
                            state.summary.add(&record);
                        }
                    }
                    Ok(())
                })();
                if result.is_ok() {
                    guard.1 = false;
                }
                result
            })?);
        }
        let mut first_error = None;
        for worker in workers {
            let result = worker
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("SPSA worker panicked")));
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        scope_guard.1 = false;
        Ok(())
    })?;
    Ok(scheduler
        .into_inner()
        .map_err(|_| io::Error::other("scheduler lock poisoned"))?
        .summary)
}

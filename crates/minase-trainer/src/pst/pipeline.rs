//! バッチ番号で担当を固定する、容量制限付きの先行構築。

use super::{Error, Result};
use std::{sync::mpsc::sync_channel, thread};

// producerはバッチ構築、または構築と検証を行う。consumerは番号順に更新・集計する。
// 各ワーカーが保持するのは送信済みの1件と構築中の1件まで。
pub(super) fn ordered<T: Send>(
    batches: usize,
    workers: usize,
    producer: impl Fn(usize) -> Result<T> + Sync,
    mut consumer: impl FnMut(usize, T) -> Result<()>,
) -> Result<()> {
    crate::nn::parallel::check_threads(workers)?;
    let workers = workers.min(batches);
    thread::scope(|scope| {
        let producer = &producer;
        let mut receivers = Vec::with_capacity(workers);
        let mut handles = Vec::with_capacity(workers);
        let mut result = Ok(());
        for worker in 0..workers {
            let (sender, receiver) = sync_channel(1);
            match thread::Builder::new().spawn_scoped(scope, move || {
                for index in (worker..batches).step_by(workers) {
                    let item = producer(index);
                    let failed = item.is_err();
                    if sender.send(item).is_err() || failed {
                        break;
                    }
                }
            }) {
                Ok(handle) => {
                    handles.push(handle);
                    receivers.push(receiver);
                }
                Err(error) => {
                    result = Err(error.into());
                    break;
                }
            }
        }
        if result.is_ok() {
            result = (|| {
                for index in 0..batches {
                    let item = receivers[index % workers].recv().map_err(|_| {
                        Error::Invalid("PST batch worker stopped before sending a result")
                    })??;
                    consumer(index, item)?;
                }
                Ok(())
            })();
        }
        // エラー後も送信待ちのワーカーを解放してから、すべてjoinする。
        drop(receivers);
        for handle in handles {
            if handle.join().is_err() && result.is_ok() {
                result = Err(Error::Invalid("PST batch worker panicked"));
            }
        }
        result
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Barrier, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct Item<'a> {
        index: usize,
        live: &'a AtomicUsize,
    }
    impl Drop for Item<'_> {
        fn drop(&mut self) {
            self.live.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn fixed_workers_build_ahead_but_consume_in_order_with_bounded_storage() {
        let workers = 3;
        let barrier = Barrier::new(workers);
        let ahead = (Mutex::new(false), Condvar::new());
        let assignments = Mutex::new(Vec::new());
        let live = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let mut consumed = Vec::new();
        ordered(
            19,
            workers,
            |index| {
                assignments
                    .lock()
                    .unwrap()
                    .push((index, thread::current().id()));
                if index < workers {
                    barrier.wait();
                }
                if index == workers {
                    *ahead.0.lock().unwrap() = true;
                    ahead.1.notify_one();
                }
                let count = live.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                Ok(Item { index, live: &live })
            },
            |index, item| {
                if index == 0 {
                    // 更新側が最初の結果を保持している間にも、次の構築が進む。
                    let (ready, timeout) = ahead
                        .1
                        .wait_timeout_while(
                            ahead.0.lock().unwrap(),
                            std::time::Duration::from_secs(3),
                            |ready| !*ready,
                        )
                        .unwrap();
                    assert!(*ready && !timeout.timed_out());
                }
                assert_eq!(item.index, index);
                consumed.push(index);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(consumed, (0..19).collect::<Vec<_>>());
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert!(peak.load(Ordering::SeqCst) <= 2 * workers + 1);
        let assignments = assignments.into_inner().unwrap();
        let ids: Vec<_> = (0..workers)
            .map(|first| {
                assignments
                    .iter()
                    .find(|(index, _)| *index == first)
                    .unwrap()
                    .1
            })
            .collect();
        for (index, id) in assignments {
            assert_eq!(id, ids[index % workers]);
        }
        assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
    }

    #[test]
    fn producer_consumer_errors_and_panics_release_all_queued_work() {
        for failure in 0..4 {
            let live = AtomicUsize::new(0);
            let mut consumed = Vec::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                ordered(
                    100,
                    3,
                    |index| {
                        if failure == 0 && index == 7 {
                            return Err(Error::Invalid("injected producer error"));
                        }
                        assert!(!(failure == 1 && index == 5), "injected producer panic");
                        live.fetch_add(1, Ordering::SeqCst);
                        Ok(Item { index, live: &live })
                    },
                    |index, item| {
                        if failure == 2 && index == 3 {
                            return Err(Error::Invalid("injected consumer error"));
                        }
                        assert!(!(failure == 3 && index == 3), "injected consumer panic");
                        consumed.push(item.index);
                        Ok(())
                    },
                )
            }));
            if failure == 3 {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert_eq!(live.load(Ordering::SeqCst), 0);
            let stop = [7, 5, 3, 3][failure];
            assert_eq!(consumed, (0..stop).collect::<Vec<_>>());
        }
    }

    #[test]
    fn empty_input_and_worker_counts_larger_than_the_batch_count_finish() {
        for (batches, workers) in [(0, 3), (1, 5), (5, 1)] {
            let mut result = Vec::new();
            ordered(batches, workers, Ok, |_, index| {
                result.push(index);
                Ok(())
            })
            .unwrap();
            assert_eq!(result, (0..batches).collect::<Vec<_>>());
        }
        assert!(ordered(1, 0, Ok, |_, _: usize| Ok(())).is_err());
    }
}

//! バッチの並列構築と順序を保った投入を、容量3の供給と返却の2チャネルでつなぐ。
//! 期限は群の前、各バッチの処理前、投入前に確認し、完成したバッチも順列の順に渡す。
//! 供給側のエラーを受信側の「途中終了」で隠さずに返す。
//! 指定した更新数で供給を区切り、投入済みの結果を回収してからホストへ戻る。
//! CUDAの失敗、時間切れ、正常終了の全経路で両チャネルを閉じ、供給スレッドを回収する。

use super::{
    Error, Result,
    backend::{Buffer, StepMetrics, TrainerBackend},
};
use crate::{
    data::{Dataset, EpochReader, SpillStore},
    nnue::{
        data::{BatchSlot, FeatureSet, build_batches, build_targets},
        gpu,
        schedule::LrSchedule,
    },
};
use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::Instant,
};

struct Prepared {
    host: Buffer,
    targets: Vec<f32>,
}

#[derive(Default)]
pub(super) struct Training {
    pub loss: f64,
    pub compute_seconds: f64,
    pub steps: u64,
    pub positions: usize,
    /// この区間の最初に投入した更新の倍率。
    pub first_multiplier: Option<f64>,
    /// この区間の最後に投入した更新の倍率。
    pub last_multiplier: Option<f64>,
    pub gpu_used_bytes: Option<usize>,
    pub gpu_total_bytes: Option<usize>,
}

impl Training {
    /// 周をまたぐ区間の記録を更新順に合算する。
    pub fn append(&mut self, next: Self) -> Result<()> {
        self.loss += next.loss;
        self.compute_seconds += next.compute_seconds;
        self.steps += next.steps;
        self.positions = self
            .positions
            .checked_add(next.positions)
            .ok_or(Error::Invalid("training position count overflow"))?;
        if self.first_multiplier.is_none() {
            self.first_multiplier = next.first_multiplier;
        }
        self.last_multiplier = next.last_multiplier;
        self.gpu_used_bytes = self.gpu_used_bytes.max(next.gpu_used_bytes);
        self.gpu_total_bytes = next.gpu_total_bytes;
        Ok(())
    }

    fn collect(&mut self, metrics: StepMetrics) -> Result<()> {
        let positions = metrics.positions;
        if !metrics.loss_sum.is_finite() {
            return Err(gpu::Error::Invalid("non-finite training loss").into());
        }
        self.loss += metrics.loss_sum;
        self.compute_seconds += metrics.compute_seconds;
        self.steps += 1;
        self.positions = self
            .positions
            .checked_add(positions)
            .ok_or(gpu::Error::Invalid("training position count overflow"))?;
        Ok(())
    }
}

pub(super) struct Supply<'a> {
    pub dataset: &'a Dataset,
    pub indices: &'a [u64],
    pub teacher_cp_per_logit: &'a [f64],
    pub workers: usize,
    pub options: BatchOptions,
}

fn produce(
    supply: &Supply<'_>,
    buffers: [Buffer; 3],
    recycled: Receiver<Buffer>,
    sender: SyncSender<Prepared>,
) -> gpu::Result<()> {
    let mut buffers = VecDeque::from(buffers);
    let workers = supply
        .workers
        .min(supply.indices.len().div_ceil(supply.options.batch));
    let group_size = supply
        .options
        .batch
        .checked_mul(workers)
        .ok_or(gpu::Error::Invalid("worker group size overflow"))?;
    for group in supply.indices.chunks(group_size) {
        if Instant::now() >= supply.options.deadline {
            break;
        }
        let ranges: Vec<_> = group.chunks(supply.options.batch).collect();
        // 完了順には依存せず、順列中のバッチ順に回収する。
        for batch in build_batches(supply.dataset, supply.options.feature_set, &ranges)? {
            if Instant::now() >= supply.options.deadline {
                return Ok(());
            }
            let mut host = match buffers.pop_front() {
                Some(buffer) => buffer,
                None => match recycled.recv() {
                    Ok(buffer) => buffer,
                    Err(_) => return Ok(()), // 計算側の終了で待機を解除する。
                },
            };
            let targets = build_targets(
                &batch,
                supply.teacher_cp_per_logit,
                supply.dataset.teacher_lambdas(),
            )?;
            host.fill(batch)?;
            if sender.send(Prepared { host, targets }).is_err() {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 周の部分列を、指定した更新回数だけ学ぶ。
pub(super) fn train_batches(
    supply: Supply<'_>,
    backend: &mut impl TrainerBackend,
) -> Result<Option<Training>> {
    train_batches_inner(supply.options, backend, |buffers, recycled, sender| {
        produce(&supply, buffers, recycled, sender)
    })
}

/// 周とsuperbatchの境界のうち、先に到達する位置までの更新条件。
#[derive(Clone, Copy)]
pub(super) struct BatchOptions {
    pub feature_set: FeatureSet,
    pub batch: usize,
    pub schedule: LrSchedule,
    pub batches_per_superbatch: NonZeroU64,
    pub steps: u64,
    pub deadline: Instant,
}

/// 両供給元の更新と結果回収を同じ順序で実行する。
fn train_batches_inner(
    options: BatchOptions,
    backend: &mut impl TrainerBackend,
    produce: impl FnOnce([Buffer; 3], Receiver<Buffer>, SyncSender<Prepared>) -> gpu::Result<()> + Send,
) -> Result<Option<Training>> {
    let (mut queue, buffers) = backend.queue(options.batch)?;
    let memory = backend.memory()?;
    let (sender, receiver) = sync_channel(3);
    let (recycle, recycled) = sync_channel(3);
    let total_batches = options.steps;
    std::thread::scope(|scope| {
        let producer = std::thread::Builder::new()
            .name("nnue-train-supply".into())
            .spawn_scoped(scope, || produce(buffers, recycled, sender))?;
        let consumed = (|| -> Result<Option<Training>> {
            let mut training = Training {
                gpu_used_bytes: memory.map(|(used, _)| used),
                gpu_total_bytes: memory.map(|(_, total)| total),
                ..Training::default()
            };
            let mut submitted = 0;
            while submitted < total_batches && Instant::now() < options.deadline {
                let Ok(mut prepared) = receiver.recv() else {
                    break;
                };
                if Instant::now() >= options.deadline {
                    break;
                }
                let multiplier = options
                    .schedule
                    .multiplier(backend.step(), options.batches_per_superbatch);
                let result = backend.train_step(
                    &mut queue,
                    &mut prepared.host,
                    &prepared.targets,
                    multiplier,
                )?;
                if training.first_multiplier.is_none() {
                    training.first_multiplier = Some(multiplier);
                }
                training.last_multiplier = Some(multiplier);
                // 3面だけを循環させる。fillはその面の転送完了を待ってから書き込む。
                let _ = recycle.send(prepared.host);
                if let Some(previous) = result {
                    training.collect(previous)?;
                }
                submitted += 1;
            }
            if let Some(last) = backend.flush_pending_loss()? {
                training.collect(last)?;
            }
            if Instant::now() >= options.deadline {
                return Ok(None);
            }
            if submitted != total_batches {
                return Err(Error::Invalid(
                    "batch worker stopped before completing the requested updates",
                )
                .into());
            }
            Ok(Some(training))
        })();
        let drained = backend.flush_pending_loss();
        // CUDA失敗、時間切れ、正常終了の全経路で、送信と面待ちの双方を解除する。
        drop(receiver);
        drop(recycle);
        let supplied = producer
            .join()
            .map_err(|_| crate::nnue::data::Error::WorkerPanicked)?;
        // 元のワーカーエラーを、受信側の「途中終了」で隠さない。
        supplied?;
        drained?;
        consumed
    })
}

/// 一時ファイルから最大workersバッチを先に取り出し、並列に構築して順に投入する。
/// 復元済み記録の上限はworkers×batch件で、最後以外のバッチは満杯になる。
pub(super) fn train_spill_batches(
    store: &SpillStore,
    reader: &mut EpochReader<'_>,
    workers: usize,
    scales: &[f64],
    options: BatchOptions,
    backend: &mut impl TrainerBackend,
) -> Result<Option<Training>> {
    let feature_set = options.feature_set;
    let batch_size = options.batch;
    let deadline = options.deadline;
    let mut remaining = options.steps;
    train_batches_inner(options, backend, move |buffers, recycled, sender| {
        let mut buffers = VecDeque::from(buffers);
        loop {
            if Instant::now() >= deadline {
                return Ok(());
            }
            let mut records = Vec::new();
            for _ in 0..workers {
                if remaining == 0 {
                    break;
                }
                if Instant::now() >= deadline {
                    return Ok(());
                }
                match reader
                    .next_chunk(batch_size)
                    .map_err(crate::nnue::data::Error::from)?
                {
                    Some(chunk) => {
                        remaining -= 1;
                        records.push(chunk);
                    }
                    None => break,
                }
            }
            if records.is_empty() {
                return Ok(());
            }
            let batches = std::thread::scope(|scope| {
                let handles: Vec<_> = records
                    .iter()
                    .map(|records| {
                        std::thread::Builder::new().spawn_scoped(scope, move || {
                            BatchSlot::from_spill_records(feature_set, records)
                        })
                    })
                    .collect();
                let results: Vec<_> = handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .map_err(crate::nnue::data::Error::Io)
                            .and_then(|handle| {
                                handle
                                    .join()
                                    .map_err(|_| crate::nnue::data::Error::WorkerPanicked)?
                            })
                    })
                    .collect();
                results
                    .into_iter()
                    .collect::<crate::nnue::data::Result<Vec<_>>>()
            })?;
            drop(records);
            for batch in batches {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                let mut host = match buffers.pop_front() {
                    Some(buffer) => buffer,
                    None => match recycled.recv() {
                        Ok(buffer) => buffer,
                        Err(_) => return Ok(()),
                    },
                };
                let targets = build_targets(&batch, scales, store.teacher_lambdas())?;
                host.fill(batch)?;
                if sender.send(Prepared { host, targets }).is_err() {
                    return Ok(());
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::train::{
        Progress,
        backend::{Queue, Validation},
    };
    use minase_core::{Color, Position};
    use serde_json::Value;
    use std::path::Path;

    struct Deferred {
        steps: u64,
        pending: Option<StepMetrics>,
        flushes: usize,
        fail_second: bool,
    }
    impl TrainerBackend for Deferred {
        fn name(&self) -> &str {
            "deferred test backend"
        }
        fn feature_set(&self) -> FeatureSet {
            FeatureSet::P
        }
        fn step(&self) -> u64 {
            self.steps
        }
        fn memory(&self) -> Result<Option<(usize, usize)>> {
            Ok(None)
        }
        fn queue(&self, _: usize) -> Result<(Queue, [Buffer; 3])> {
            Ok((Queue::Cpu, std::array::from_fn(|_| Buffer::Cpu(None))))
        }
        fn train_step(
            &mut self,
            _: &mut Queue,
            _: &mut Buffer,
            targets: &[f32],
            _: f64,
        ) -> Result<Option<StepMetrics>> {
            if self.fail_second && self.steps == 1 {
                return Err(Error::Invalid("injected update failure").into());
            }
            self.steps += 1;
            Ok(self.pending.replace(StepMetrics {
                loss_sum: self.steps as f64,
                positions: targets.len(),
                compute_seconds: 0.01,
            }))
        }
        fn flush_pending_loss(&mut self) -> Result<Option<StepMetrics>> {
            self.flushes += 1;
            Ok(self.pending.take())
        }
        fn validate_step(&mut self, _: &BatchSlot, _: &[f32]) -> Result<Validation> {
            panic!("validation is outside the supply pipeline")
        }
        fn statistics(&mut self, _: Option<&Validation>) -> Result<Value> {
            panic!("statistics are outside the supply pipeline")
        }
        fn save_checkpoint(&mut self, _: &Path, _: &Progress) -> Result<()> {
            panic!("saving is outside the supply pipeline")
        }
        fn export(
            &mut self,
            _: &Path,
            _: &[(Position, Color)],
            _: Option<(&Path, u64)>,
        ) -> Result<Value> {
            panic!("export is outside the supply pipeline")
        }
    }
    #[test]
    fn delayed_losses_are_drained_on_completion_failure_and_expiration() {
        for mode in 0..3 {
            let mut backend = Deferred {
                steps: 0,
                pending: None,
                flushes: 0,
                fail_second: mode == 1,
            };
            let options = BatchOptions {
                feature_set: FeatureSet::P,
                batch: 2,
                schedule: "constant:1".parse().unwrap(),
                batches_per_superbatch: 3.try_into().unwrap(),
                steps: 3,
                deadline: if mode == 2 {
                    Instant::now()
                } else {
                    Instant::now() + std::time::Duration::from_secs(60)
                },
            };
            let result = train_batches_inner(options, &mut backend, |buffers, _, sender| {
                for host in buffers {
                    if sender
                        .send(Prepared {
                            host,
                            targets: vec![0.5; 2],
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(())
            });
            assert!(backend.pending.is_none());
            assert!(backend.flushes > 0);
            match mode {
                0 => {
                    let result = result.unwrap().unwrap();
                    assert_eq!(result.steps, 3);
                    assert_eq!(result.positions, 6);
                    assert_eq!(result.loss, 6.0);
                }
                1 => assert!(result.is_err()),
                _ => assert!(result.unwrap().is_none()),
            }
        }
    }
}

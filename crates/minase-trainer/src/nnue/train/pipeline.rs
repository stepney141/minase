//! バッチの並列構築と順序を保った投入を、容量3の供給と返却の2チャネルでつなぐ。
//! 期限は群の前、各バッチの処理前、投入前に確認し、完成したバッチも順列の順に渡す。
//! 供給側のエラーを受信側の「途中終了」で隠さずに返す。
//! 途中検証の前には投入済み更新の損失をすべて回収し、検証中は次の更新を投入しない。
//! CUDAの失敗、時間切れ、正常終了の全経路で両チャネルを閉じ、供給スレッドを回収する。

use super::{
    Error, Result,
    backend::{Backend, Buffer, StepResult},
};
use crate::{
    data::{Dataset, EpochReader, SpillStore},
    nnue::{
        data::{BatchSlot, InputFeatureSet, build_batches, build_targets},
        gpu,
        schedule::LrSchedule,
    },
};
use std::{
    collections::VecDeque,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::Instant,
};

struct Prepared {
    host: Buffer,
    targets: Vec<f32>,
    positions: usize,
}

#[derive(Default)]
pub(super) struct Training {
    pub bce: f64,
    pub compute_seconds: f64,
    /// 途中検証の秒数の合計。パラメータの回収とログ保存を含み、学習秒数から除く。
    pub interval_validation_seconds: f64,
    pub steps: u64,
    pub positions: usize,
    /// このエポックの最初に投入した更新の倍率。
    pub first_multiplier: Option<f64>,
    /// このエポックの最後に投入した更新の倍率。
    pub last_multiplier: Option<f64>,
    pub gpu_used_bytes: Option<usize>,
    pub gpu_total_bytes: Option<usize>,
}

impl Training {
    fn collect(&mut self, result: StepResult, positions: usize) -> gpu::Result<()> {
        let metrics = result.collect()?;
        if !metrics.losses.bce.is_finite() {
            return Err(gpu::Error::Invalid("non-finite training loss"));
        }
        self.bce += metrics.losses.bce * positions as f64;
        self.compute_seconds += f64::from(metrics.compute_ms) / 1000.0;
        self.steps += 1;
        self.positions += positions;
        Ok(())
    }
}

pub(super) struct Supply<'a> {
    pub dataset: &'a Dataset,
    pub feature_set: InputFeatureSet,
    pub indices: &'a [u64],
    pub teacher_cp_per_logit: &'a [f64],
    pub batch: usize,
    /// 累積更新回数に対する途中検証の間隔。0なら無効で、エポック境界は除く。
    pub validation_interval: u64,
    /// 完了済み更新回数から学習率の倍率を求める規則。
    pub schedule: LrSchedule,
    /// `ceil(訓練局面数 / バッチサイズ)`。1以上である。
    pub steps_per_epoch: u64,
    pub workers: usize,
    pub deadline: Instant,
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
        .min(supply.indices.len().div_ceil(supply.batch));
    let group_size = supply
        .batch
        .checked_mul(workers)
        .ok_or(gpu::Error::Invalid("worker group size overflow"))?;
    for group in supply.indices.chunks(group_size) {
        if Instant::now() >= supply.deadline {
            break;
        }
        let ranges: Vec<_> = group.chunks(supply.batch).collect();
        // 完了順には依存せず、順列中のバッチ順に回収する。
        for batch in build_batches(supply.dataset, supply.feature_set, &ranges)? {
            if Instant::now() >= supply.deadline {
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
            let positions = batch.len();
            host.fill(batch)?;
            if sender
                .send(Prepared {
                    host,
                    targets,
                    positions,
                })
                .is_err()
            {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 更新順に損失を集計し、指定間隔で未回収の結果を空にしてから途中検証を呼ぶ。
/// 検証の閉包はバックエンドを共有借用し、期限切れで検証を中断したら`false`を返す。
pub(super) fn train_epoch(
    supply: Supply<'_>,
    backend: &mut Backend,
    mut interval_validation: impl FnMut(&Supply<'_>, &Backend) -> Result<bool>,
) -> Result<Option<Training>> {
    let options = EpochOptions {
        feature_set: supply.feature_set,
        batch: supply.batch,
        validation_interval: supply.validation_interval,
        schedule: supply.schedule,
        steps_per_epoch: supply.steps_per_epoch,
        deadline: supply.deadline,
    };
    train_epoch_inner(
        options,
        backend,
        |backend| interval_validation(&supply, backend),
        |buffers, recycled, sender| produce(&supply, buffers, recycled, sender),
    )
}

/// 供給元によらない更新順、検証間隔と期限。
pub(super) struct EpochOptions {
    /// 入力特徴の集合。
    pub feature_set: InputFeatureSet,
    /// 1更新あたりの最大局面数。
    pub batch: usize,
    /// 途中検証の更新間隔。
    pub validation_interval: u64,
    /// 学習率を決める規則。
    pub schedule: LrSchedule,
    /// 1エポックの更新回数。
    pub steps_per_epoch: u64,
    /// エポックを完了する期限。
    pub deadline: Instant,
}

/// 両供給元の更新と結果回収を同じ順序で実行する。
fn train_epoch_inner(
    supply: EpochOptions,
    backend: &mut Backend,
    mut interval_validation: impl FnMut(&Backend) -> Result<bool>,
    produce: impl FnOnce([Buffer; 3], Receiver<Buffer>, SyncSender<Prepared>) -> gpu::Result<()> + Send,
) -> Result<Option<Training>> {
    let (mut queue, buffers) = backend.queue(supply.feature_set, supply.batch)?;
    let memory = backend.memory()?;
    let (sender, receiver) = sync_channel(3);
    let (recycle, recycled) = sync_channel(3);
    let total_batches = supply.steps_per_epoch;
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
            let mut pending: Option<(StepResult, usize)> = None;
            let mut submitted = 0;
            while submitted < total_batches && Instant::now() < supply.deadline {
                let Ok(mut prepared) = receiver.recv() else {
                    break;
                };
                if Instant::now() >= supply.deadline {
                    break;
                }
                let multiplier = supply
                    .schedule
                    .multiplier(backend.step(), supply.steps_per_epoch);
                let result = backend.enqueue(
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
                if let Some((previous, positions)) = pending.replace((result, prepared.positions)) {
                    training.collect(previous, positions)?;
                }
                submitted += 1;
                let step = backend.step();
                if supply.validation_interval > 0
                    && step.is_multiple_of(supply.validation_interval)
                    && !step.is_multiple_of(supply.steps_per_epoch)
                {
                    if let Some((last, positions)) = pending.take() {
                        training.collect(last, positions)?;
                    }
                    if Instant::now() >= supply.deadline {
                        return Ok(None);
                    }
                    let validation_started = Instant::now();
                    if !interval_validation(backend)? {
                        return Ok(None);
                    }
                    training.interval_validation_seconds +=
                        validation_started.elapsed().as_secs_f64();
                }
            }
            if let Some((last, positions)) = pending {
                training.collect(last, positions)?;
            }
            if Instant::now() >= supply.deadline {
                return Ok(None);
            }
            if submitted != total_batches {
                return Err(
                    Error::Invalid("batch worker stopped before completing the epoch").into(),
                );
            }
            Ok(Some(training))
        })();
        // CUDA失敗、時間切れ、正常終了の全経路で、送信と面待ちの双方を解除する。
        drop(receiver);
        drop(recycle);
        let supplied = producer
            .join()
            .map_err(|_| crate::nnue::data::Error::WorkerPanicked)?;
        // 元のワーカーエラーを、受信側の「途中終了」で隠さない。
        supplied?;
        consumed
    })
}

/// 一時ファイルから最大workersバッチを先に取り出し、並列に構築して順に投入する。
/// 復元済み記録の上限はworkers×batch件で、最後以外のバッチは満杯になる。
pub(super) fn train_spill_epoch(
    store: &SpillStore,
    mut reader: EpochReader<'_>,
    workers: usize,
    scales: &[f64],
    options: EpochOptions,
    backend: &mut Backend,
    interval_validation: impl FnMut(&Backend) -> Result<bool>,
) -> Result<Option<Training>> {
    let feature_set = options.feature_set;
    let batch_size = options.batch;
    let deadline = options.deadline;
    train_epoch_inner(
        options,
        backend,
        interval_validation,
        move |buffers, recycled, sender| {
            let mut buffers = VecDeque::from(buffers);
            loop {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                let mut records = Vec::new();
                for _ in 0..workers {
                    if Instant::now() >= deadline {
                        return Ok(());
                    }
                    match reader
                        .next_chunk(batch_size)
                        .map_err(crate::nnue::data::Error::from)?
                    {
                        Some(chunk) => records.push(chunk),
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
                    let positions = batch.len();
                    host.fill(batch)?;
                    if sender
                        .send(Prepared {
                            host,
                            targets,
                            positions,
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            }
        },
    )
}

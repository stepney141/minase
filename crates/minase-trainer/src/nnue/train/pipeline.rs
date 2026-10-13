//! バッチの並列構築と順序を保った投入を、容量3の供給と返却の2チャネルでつなぐ。
//! 期限は群の前、各バッチの処理前、投入前に確認し、完成したバッチも順列の順に渡す。
//! 供給側のエラーを受信側の「途中終了」で隠さずに返す。
//! CUDAの失敗、時間切れ、正常終了の全経路で両チャネルを閉じ、供給スレッドを回収する。

use super::{
    Error, Result,
    backend::{Backend, Buffer, StepResult},
};
use crate::{
    data::Dataset,
    nnue::{
        data::{InputFeatureSet, build_batches, build_targets},
        gpu,
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
    pub auxiliary: f64,
    pub compute_seconds: f64,
    pub steps: u64,
    pub positions: usize,
    pub gpu_used_bytes: Option<usize>,
    pub gpu_total_bytes: Option<usize>,
}

impl Training {
    fn collect(&mut self, result: StepResult, positions: usize) -> gpu::Result<()> {
        let metrics = result.collect()?;
        if !metrics.losses.bce.is_finite() || !metrics.losses.auxiliary.is_finite() {
            return Err(gpu::Error::Invalid("non-finite training loss"));
        }
        self.bce += metrics.losses.bce * positions as f64;
        self.auxiliary += metrics.losses.auxiliary * positions as f64;
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
    pub teacher_ks: &'a [f64],
    pub batch: usize,
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
            let targets =
                build_targets(&batch, supply.teacher_ks, supply.dataset.teacher_lambdas())?;
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

pub(super) fn train_epoch(supply: Supply<'_>, backend: &mut Backend) -> Result<Option<Training>> {
    let (mut queue, buffers) = backend.queue(supply.feature_set, supply.batch)?;
    let memory = backend.memory()?;
    let (sender, receiver) = sync_channel(3);
    let (recycle, recycled) = sync_channel(3);
    let total_batches = supply.indices.len().div_ceil(supply.batch);
    std::thread::scope(|scope| {
        let producer = std::thread::Builder::new()
            .name("nnue-train-supply".into())
            .spawn_scoped(scope, || produce(&supply, buffers, recycled, sender))?;
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
                let result = backend.enqueue(&mut queue, &mut prepared.host, &prepared.targets)?;
                // 3面だけを循環させる。fillはその面の転送完了を待ってから書き込む。
                let _ = recycle.send(prepared.host);
                if let Some((previous, positions)) = pending.replace((result, prepared.positions)) {
                    training.collect(previous, positions)?;
                }
                submitted += 1;
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

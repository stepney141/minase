use super::{
    BatchTicket, Device, Error, HostRing, Result,
    device::{Kernels, pinned_zeros},
    engine::{self, Buffers, DeviceModel},
    optimizer::Optimizer,
};
use crate::{
    data::MAX_BATCH_SIZE,
    model::{Model, Parameters},
    reference::{AdamW, Losses, RemovalReference},
};
use cudarc::driver::{CudaEvent, CudaSlice, PinnedHostSlice, sys::CUevent_flags};
use std::sync::Arc;

// 回収用メモリと計測イベントは2面だけを事前確保する。結果の所有者が残る間は
// Arc::get_mutが失敗するため、未回収の値やイベントの世代を上書きできない。
struct Readback {
    losses: PinnedHostSlice<f64>,
    start: CudaEvent,
    end: CudaEvent,
    target_start: CudaEvent,
    target_end: CudaEvent,
}
impl Readback {
    fn new(device: &Device) -> Result<Self> {
        Ok(Self {
            losses: pinned_zeros(device, 2)?,
            start: device
                .context
                .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
            end: device
                .context
                .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
            target_start: device
                .context
                .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
            target_end: device
                .context
                .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
        })
    }
}

/// 完了した更新の損失と、計算ストリームで測ったカーネルの時間。
#[derive(Clone, Copy, Debug)]
pub struct StepMetrics {
    /// 更新前のモデルに対するバッチ平均の損失。
    pub losses: Losses,
    /// 前進の開始からAdamW群の完了までのミリ秒。入力転送と損失転送を除く。
    pub compute_ms: f32,
    /// 転送ストリームで測った教師値の転送ミリ秒。入力面の転送時間とは別。
    pub target_transfer_ms: f32,
    /// 教師値の転送バイト数。端数バッチでも確保した面の容量全体を送る。
    pub target_transfer_bytes: usize,
    /// この更新を含むAdamWの更新回数。
    pub step: u64,
}

/// 非同期に投入した1更新の回収券。次の更新を投入してから`collect`する。
/// 末尾の更新は次の投入を待たずに回収できる。
#[must_use = "collect the losses after submitting the next update"]
pub struct StepResult {
    readback: Arc<Readback>,
    batch_len: usize,
    target_bytes: usize,
    step: u64,
}
impl StepResult {
    /// この更新の損失転送だけを待ち、平均損失とCUDAイベントの時間を返す。
    /// 回収券を消費すると、対応する固定メモリの面を次回以降に再利用できる。
    pub fn collect(self) -> Result<StepMetrics> {
        let values = self.readback.losses.as_slice()?;
        Ok(StepMetrics {
            losses: Losses {
                bce: values[0] / self.batch_len as f64,
                auxiliary: values[1] / self.batch_len as f64,
            },
            compute_ms: self.readback.start.elapsed_ms(&self.readback.end)?,
            target_transfer_ms: self
                .readback
                .target_start
                .elapsed_ms(&self.readback.target_end)?,
            target_transfer_bytes: self.target_bytes,
            step: self.step,
        })
    }
}

/// モデル、AdamW状態、作業配列をデバイスに保持して1更新を投入する。
///
/// `HostRing::upload`の券と教師値を`enqueue`へ渡す。損失は2面の固定メモリへ
/// 非同期に転送し、次の更新を投入した後に`StepResult::collect`で回収する。
/// 同時に保持できる未回収結果は2つで、3つ目の投入前に最初の結果を回収する。
pub struct Step {
    device: Device,
    kernels: Kernels,
    model: DeviceModel,
    optimizer: Optimizer,
    buffers: Buffers,
    spare_targets: CudaSlice<f32>,
    spare_host_targets: PinnedHostSlice<f32>,
    readbacks: [Arc<Readback>; 2],
    next_readback: usize,
    capacity: usize,
    failed: bool,
}
impl Step {
    /// モデルと最適化器を一度だけ転送し、最大バッチ分の配列を確保する。
    pub fn new(
        device: Device,
        model: &Model,
        optimizer: &AdamW,
        reference: &RemovalReference,
        capacity: usize,
    ) -> Result<Self> {
        if !(1..=MAX_BATCH_SIZE).contains(&capacity) {
            return Err(Error::Invalid("step capacity"));
        }
        let optimizer = Optimizer::new(&device, model, optimizer)?;
        let kernels = Kernels::compile(&device)?;
        let buffers = Buffers::new(&device, model.shape(), capacity)?;
        let model = DeviceModel::new(&device, model, reference)?;
        let spare_targets = device.compute.alloc_zeros(capacity)?;
        let spare_host_targets = pinned_zeros(&device, capacity)?;
        let readbacks = [
            Arc::new(Readback::new(&device)?),
            Arc::new(Readback::new(&device)?),
        ];
        Ok(Self {
            device,
            kernels,
            model,
            optimizer,
            buffers,
            spare_targets,
            spare_host_targets,
            readbacks,
            next_readback: 0,
            capacity,
            failed: false,
        })
    }

    /// モデルと最適化器が常駐するデバイス。
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// 投入済みの更新までを含むAdamWの更新回数。
    pub fn step(&self) -> u64 {
        self.optimizer.step()
    }

    /// 入力面を受け取り、前進、補助損失、出力部、縮約、後退、AdamWを順に投入する。
    ///
    /// 環は`device()`と同じストリームと特徴集合で作る必要がある。
    /// 入力面の完了は後退の直後に記録する。更新と読み出しは計算ストリーム順に
    /// 進み、ホストで計算完了を待つ処理はここには置かない。
    pub fn enqueue(
        &mut self,
        ring: &mut HostRing,
        ticket: &BatchTicket,
        targets: &[f32],
    ) -> Result<StepResult> {
        if self.failed {
            return Err(Error::Invalid("step has a failed submission"));
        }
        ring.validate_device(&self.device, self.model.shape)?;
        let n = ring.input(ticket)?.len;
        if n == 0
            || n > self.capacity
            || targets.len() != n
            || targets
                .iter()
                .any(|&x| !x.is_finite() || !(0.0..=1.0).contains(&x))
        {
            return Err(Error::Invalid("step batch shape, capacity, or targets"));
        }
        if self.optimizer.step() == u64::MAX {
            return Err(Error::Invalid("AdamW step overflow"));
        }
        let readback = Arc::get_mut(&mut self.readbacks[self.next_readback]).ok_or(
            Error::Invalid("collect the oldest step result before reusing its slot"),
        )?;

        // 教師値も2面にし、次の投入で前回の出力部が読む値を上書きしない。
        std::mem::swap(&mut self.buffers.targets, &mut self.spare_targets);
        std::mem::swap(&mut self.buffers.host_targets, &mut self.spare_host_targets);
        let submitted = (|| -> Result<()> {
            let input = ring.input(ticket)?;
            self.buffers.host_targets.as_mut_slice()?[..n].copy_from_slice(targets);
            readback.target_start.record(&self.device.transfer)?;
            self.device
                .transfer
                .memcpy_htod(&self.buffers.host_targets, &mut self.buffers.targets)?;
            readback.target_end.record(&self.device.transfer)?;
            self.device.compute.wait(&readback.target_end)?;
            readback.start.record(&self.device.compute)?;
            engine::forward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
            )?;
            engine::removal_forward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
            )?;
            engine::output_backward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
            )?;
            engine::gradients(
                &self.device,
                &self.kernels,
                self.model.shape,
                n,
                &mut self.buffers,
            )?;
            engine::backward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
            )?;
            ring.finish(ticket.clone())?;
            self.optimizer
                .update(&self.device, &self.kernels, &mut self.model, &self.buffers)?;
            readback.end.record(&self.device.compute)?;
            // PinnedHostSlice自体を渡すと、cudarcは転送完了をイベントで追跡する。
            // ホストからの参照はcollectまで作らず、次の縮約より前に損失を保存する。
            self.device
                .compute
                .memcpy_dtoh(&self.buffers.loss_sum, &mut readback.losses)?;
            Ok(())
        })();
        if let Err(error) = submitted {
            // 一部の群だけが更新された可能性があるため再投入・保存を拒否する。
            self.failed = true;
            return Err(error);
        }
        let result = StepResult {
            readback: Arc::clone(&self.readbacks[self.next_readback]),
            batch_len: n,
            target_bytes: self.capacity * size_of::<f32>(),
            step: self.optimizer.step(),
        };
        self.next_readback ^= 1;
        Ok(result)
    }

    /// 投入済み更新の完了を待ち、全パラメータとAdamW状態を回収する。
    /// 等価性検査や保存用であり、更新ごとの計算経路からは呼ばない。
    pub fn snapshot(&self) -> Result<(Parameters, AdamW)> {
        if self.failed {
            return Err(Error::Invalid("step has a failed submission"));
        }
        self.optimizer.snapshot(&self.device)
    }

    // 等価性検査では、次の前進が読む合成重みと共有パラメータの展開も照合する。
    pub(super) fn snapshot_expanded(&self) -> Result<(Vec<f32>, Vec<f32>, Vec<f32>)> {
        if self.failed {
            return Err(Error::Invalid("step has a failed submission"));
        }
        Ok((
            self.device.compute.clone_dtoh(&self.model.weights)?,
            self.device.compute.clone_dtoh(&self.model.bias)?,
            self.device.compute.clone_dtoh(&self.model.output)?,
        ))
    }
}

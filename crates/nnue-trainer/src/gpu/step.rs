use super::{
    BatchTicket, Device, Error, HostRing, Result, THREADS, WARPS_PER_BLOCK,
    device::{Kernels, pinned_zeros},
    engine::{self, Buffers, DeviceModel},
    optimizer::Optimizer,
    ring::DeviceInput,
};
use crate::{
    data::{CANONICAL_ROWS, FEATURE_COLUMNS, MAX_BATCH_SIZE},
    model::{Model, Parameter, Parameters, Shape},
    reference::{AdamW, Losses, RemovalReference},
};
use cudarc::driver::{
    CudaEvent, CudaSlice, LaunchArgs, LaunchConfig, PinnedHostSlice, sys::CUevent_flags,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy)]
pub(super) enum Kernel {
    ForwardAcc,
    ForwardLinear,
    RemovalForward,
    OutputBackward,
    GradientPartials,
    ReducePartials { blocks: usize, columns: usize },
    FirstLayerBackwardPartials,
    FirstLayerBackwardReduce,
    LinearBackwardPartials,
    LinearBackwardReduce,
    ReduceVirtual,
    OptimizerRows(Parameter),
    OptimizerVirtual,
    OptimizerFirst,
    OptimizerHead(Parameter),
}

impl Kernel {
    fn name(self) -> &'static str {
        match self {
            Self::ForwardAcc => "forward_acc",
            Self::ForwardLinear => "forward_linear",
            Self::RemovalForward => "removal_forward",
            Self::OutputBackward => "output_backward",
            Self::GradientPartials => "gradient_partials",
            Self::ReducePartials { .. } => "reduce_partials",
            Self::FirstLayerBackwardPartials => "first_layer_backward_partials",
            Self::FirstLayerBackwardReduce => "first_layer_backward_reduce",
            Self::LinearBackwardPartials => "linear_backward_partials",
            Self::LinearBackwardReduce => "linear_backward_reduce",
            Self::ReduceVirtual => "reduce_virtual",
            Self::OptimizerRows(_) => "optimizer_rows",
            Self::OptimizerVirtual => "optimizer_virtual",
            Self::OptimizerFirst => "optimizer_first",
            Self::OptimizerHead(_) => "optimizer_head",
        }
    }
}

struct KernelEvents {
    kernel: Kernel,
    config: LaunchConfig,
    start: CudaEvent,
    end: CudaEvent,
}

pub(super) struct KernelTiming {
    pub name: &'static str,
    pub config: LaunchConfig,
    pub elapsed_ms: f32,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

// イベントは更新ごとの回収券が所有する。次の更新の投入では再記録しない。
pub(super) struct KernelProfile {
    shape: Shape,
    batch: usize,
    first_entries: usize,
    linear_entries: usize,
    first_segments: usize,
    linear_segments: usize,
    events: Vec<KernelEvents>,
}

impl KernelProfile {
    fn new(shape: Shape, input: &DeviceInput) -> Self {
        Self {
            shape,
            batch: input.len,
            first_entries: input.first_len,
            linear_entries: input.linear_len,
            first_segments: input.first_segments,
            linear_segments: input.linear_segments,
            events: Vec::with_capacity(18),
        }
    }

    fn collect(&self) -> Result<Vec<KernelTiming>> {
        self.events
            .iter()
            .map(|event| {
                let (read_bytes, write_bytes) = self.array_bytes(event.kernel);
                Ok(KernelTiming {
                    name: event.kernel.name(),
                    config: event.config,
                    elapsed_ms: event.start.elapsed_ms(&event.end)?,
                    read_bytes,
                    write_bytes,
                })
            })
            .collect()
    }

    // キャッシュと共有メモリを含まない、主要配列の論理アクセス量の概算。
    // 逆索引の長さは実バッチの値。除去の有効列とAdamWの観測行は上限で数える。
    fn array_bytes(&self, kernel: Kernel) -> (u64, u64) {
        let b = self.batch as u64;
        let h = self.shape.width() as u64;
        let u = self.shape.units() as u64;
        let r = self.shape.rows() as u64;
        let f = self.first_entries as u64;
        let e = self.linear_entries as u64;
        let first_segments = self.first_segments as u64;
        let linear_segments = self.linear_segments as u64;
        let g = h.div_ceil(4);
        let q = b.div_ceil(THREADS as u64);
        let head_blocks = b.div_ceil(WARPS_PER_BLOCK as u64);
        let l = CANONICAL_ROWS as u64;
        let columns = FEATURE_COLUMNS as u64;
        let squares = columns - 1;
        let parameter_len = |p| self.shape.len(p) as u64;
        match kernel {
            Kernel::ForwardAcc => (4 * (2 * b * columns * g + f * h + 2 * b * h), 8 * b * h),
            Kernel::ForwardLinear => (8 * (b * columns + e), 8 * b),
            // 除去前と全144列のacc/output、除去時weights、有効列の微分を数える。
            // 小さいlinear/reference等は除外し、有効列数を駒の出現数で上限化する。
            Kernel::RemovalForward => (
                8 * b * columns * (h + u) + 4 * f * h + 32 * e.min(squares * b) * h,
                b * (squares * 38 + 16 * h + 20),
            ),
            Kernel::OutputBackward => (
                40 * b * h + 8 * b * u + 37 * b,
                32 * b * h + 20 * b + 16 * head_blocks,
            ),
            Kernel::GradientPartials => (4 * b * (4 * h + 1), 8 * q * (3 * u + 1)),
            Kernel::ReducePartials { blocks, columns } => {
                (8 * blocks as u64 * columns as u64, 8 * columns as u64)
            }
            Kernel::FirstLayerBackwardPartials => (
                8 * first_segments * THREADS as u64 + 28 * f * h,
                8 * first_segments * h,
            ),
            Kernel::FirstLayerBackwardReduce => {
                (8 * (r - 1) * h + 8 * first_segments * h, 4 * r * h)
            }
            Kernel::LinearBackwardPartials => {
                (8 * linear_segments * 32 + 17 * e, 16 * linear_segments)
            }
            Kernel::LinearBackwardReduce => (8 * l + 16 * linear_segments, 8 * l),
            Kernel::ReduceVirtual => (4 * (r - 1) * h, 4 * parameter_len(Parameter::Virtual)),
            Kernel::OptimizerRows(p) => (17 * parameter_len(p), 16 * parameter_len(p)),
            Kernel::OptimizerVirtual => (
                17 * parameter_len(Parameter::Virtual),
                12 * parameter_len(Parameter::Virtual),
            ),
            Kernel::OptimizerFirst => (
                21 * parameter_len(Parameter::FirstLayer),
                16 * parameter_len(Parameter::FirstLayer),
            ),
            Kernel::OptimizerHead(p) => {
                let n = parameter_len(p);
                let expanded = if p == Parameter::OutputBias {
                    1
                } else {
                    n * h / u
                };
                (20 * n, 12 * n + 4 * expanded)
            }
        }
    }
}

/// # Safety
/// `LaunchArgs::launch`と同じく、引数の型、順序、配列範囲と書き込みの排他性を保証する。
pub(super) unsafe fn launch_kernel(
    args: &mut LaunchArgs<'_>,
    config: LaunchConfig,
    kernel: Kernel,
    profile: Option<&mut KernelProfile>,
) -> Result<()> {
    if profile.is_some() {
        // cudarcは入力依存のwait後、起動の直前・直後に計算ストリームで
        // record_eventを呼ぶ。Noneなら従来のlaunchと同じで計測イベントを作らない。
        args.record_kernel_launch(CUevent_flags::CU_EVENT_DEFAULT);
    }
    // SAFETY: 呼び出し元が元のlaunchと同じ契約を保証し、引数と格子は変更しない。
    let events = unsafe { args.launch(config)? };
    if let Some(profile) = profile {
        let (start, end) = events.ok_or(Error::Invalid("missing kernel timing events"))?;
        profile.events.push(KernelEvents {
            kernel,
            config,
            start,
            end,
        });
    }
    Ok(())
}

// 回収用メモリと計測イベントは2面だけを事前確保する。結果の所有者が残る間は
// Arc::get_mutが失敗するため、未回収の値やイベントの世代を上書きできない。
struct Readback {
    losses: PinnedHostSlice<f64>,
    nonfinite: PinnedHostSlice<u32>,
    start: CudaEvent,
    end: CudaEvent,
    target_start: CudaEvent,
    target_end: CudaEvent,
}
impl Readback {
    fn new(device: &Device) -> Result<Self> {
        Ok(Self {
            losses: pinned_zeros(device, 2)?,
            nonfinite: pinned_zeros(device, 1)?,
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
    failed: Arc<AtomicBool>,
    batch_len: usize,
    target_bytes: usize,
    step: u64,
    profile: Option<KernelProfile>,
}
impl StepResult {
    pub(super) fn collect_profiled(mut self) -> Result<(StepMetrics, Vec<KernelTiming>)> {
        // 損失の転送完了まで待てば、全カーネルの終了イベントも完了している。
        let profile = self.profile.take();
        let metrics = self.collect()?;
        let profile = match profile {
            Some(profile) => profile.collect()?,
            None => Vec::new(),
        };
        Ok((metrics, profile))
    }

    /// この更新の損失転送だけを待ち、平均損失とCUDAイベントの時間を返す。
    /// 回収券を消費すると、対応する固定メモリの面を次回以降に再利用できる。
    pub fn collect(self) -> Result<StepMetrics> {
        let values = self.readback.losses.as_slice()?;
        let status = self.readback.nonfinite.as_slice()?[0];
        let error = if values.iter().any(|value| !value.is_finite()) {
            Some("training loss")
        } else if status & 1 != 0 {
            Some("loss gradient")
        } else if status & 2 != 0 {
            Some("AdamW state or parameter")
        } else {
            None
        };
        if let Some(error) = error {
            self.failed.store(true, Ordering::Relaxed);
            return Err(Error::NonFinite(error));
        }
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
    nonfinite: CudaSlice<u32>,
    spare_targets: CudaSlice<f32>,
    spare_host_targets: PinnedHostSlice<f32>,
    readbacks: [Arc<Readback>; 2],
    next_readback: usize,
    capacity: usize,
    removal_penalty: f64,
    failed: Arc<AtomicBool>,
    profile: bool,
}
impl Step {
    /// モデルと最適化器を一度だけ転送し、最大バッチ分の配列を確保する。
    pub fn new(
        device: Device,
        model: &Model,
        optimizer: &AdamW,
        reference: &RemovalReference,
        capacity: usize,
        removal_penalty: f64,
    ) -> Result<Self> {
        if !(1..=MAX_BATCH_SIZE).contains(&capacity) {
            return Err(Error::Invalid("step capacity"));
        }
        if !removal_penalty.is_finite() || removal_penalty < 0.0 {
            return Err(Error::Invalid(
                "removal penalty must be finite and nonnegative",
            ));
        }
        let optimizer = Optimizer::new(&device, model, optimizer)?;
        let kernels = Kernels::compile(&device)?;
        let buffers = Buffers::new(&device, model.shape(), capacity)?;
        let model = DeviceModel::new(&device, model, reference)?;
        let spare_targets = device.compute.alloc_zeros(capacity)?;
        let nonfinite = device.compute.alloc_zeros(1)?;
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
            nonfinite,
            spare_targets,
            spare_host_targets,
            readbacks,
            next_readback: 0,
            capacity,
            removal_penalty,
            failed: Arc::new(AtomicBool::new(false)),
            profile: false,
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

    pub(super) fn enable_profiling(&mut self) {
        self.profile = true;
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
        if self.failed.load(Ordering::Relaxed) {
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
        let mut profile = if self.profile {
            Some(KernelProfile::new(self.model.shape, ring.input(ticket)?))
        } else {
            None
        };

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
                profile.as_mut(),
            )?;
            engine::removal_forward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
                self.removal_penalty,
                profile.as_mut(),
            )?;
            engine::output_backward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
                profile.as_mut(),
            )?;
            engine::gradients(
                &self.device,
                &self.kernels,
                self.model.shape,
                n,
                &mut self.buffers,
                profile.as_mut(),
            )?;
            engine::backward(
                &self.device,
                &self.kernels,
                &self.model,
                input,
                &mut self.buffers,
                profile.as_mut(),
            )?;
            ring.finish(ticket.clone())?;
            self.device.compute.memset_zeros(&mut self.nonfinite)?;
            self.optimizer.update(
                &self.device,
                &self.kernels,
                &mut self.model,
                &self.buffers,
                &mut self.nonfinite,
                profile.as_mut(),
            )?;
            readback.end.record(&self.device.compute)?;
            // PinnedHostSlice自体を渡すと、cudarcは転送完了をイベントで追跡する。
            // ホストからの参照はcollectまで作らず、次の縮約より前に損失を保存する。
            self.device
                .compute
                .memcpy_dtoh(&self.buffers.loss_sum, &mut readback.losses)?;
            self.device
                .compute
                .memcpy_dtoh(&self.nonfinite, &mut readback.nonfinite)?;
            Ok(())
        })();
        if let Err(error) = submitted {
            // 一部の群だけが更新された可能性があるため再投入・保存を拒否する。
            self.failed.store(true, Ordering::Relaxed);
            return Err(error);
        }
        let result = StepResult {
            readback: Arc::clone(&self.readbacks[self.next_readback]),
            failed: Arc::clone(&self.failed),
            batch_len: n,
            target_bytes: self.capacity * size_of::<f32>(),
            step: self.optimizer.step(),
            profile,
        };
        self.next_readback ^= 1;
        Ok(result)
    }

    /// 投入済み更新の完了を待ち、全パラメータとAdamW状態を回収する。
    /// 等価性検査や保存用であり、更新ごとの計算経路からは呼ばない。
    pub fn snapshot(&self) -> Result<(Parameters, AdamW)> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(Error::Invalid("step has a failed submission"));
        }
        self.optimizer.snapshot(&self.device)
    }

    // 等価性検査では、次の前進が読む合成重みと共有パラメータの展開も照合する。
    pub(super) fn snapshot_expanded(&self) -> Result<(Vec<f32>, Vec<f32>, Vec<f32>)> {
        if self.failed.load(Ordering::Relaxed) {
            return Err(Error::Invalid("step has a failed submission"));
        }
        Ok((
            self.device.compute.clone_dtoh(&self.model.weights)?,
            self.device.compute.clone_dtoh(&self.model.bias)?,
            self.device.compute.clone_dtoh(&self.model.output)?,
        ))
    }
}

use super::{
    Device, Error, HostRing, Result, THREADS, WARPS_PER_BLOCK,
    device::{Kernels, pinned_zeros},
    launch_config, require_len,
    ring::DeviceInput,
    step::{Kernel, KernelProfile, launch_kernel},
};
use crate::{
    data::{BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, max_segment_count},
    model::{Model, Parameter, Shape},
    reference::{self, Forward, Losses, Output, Removal, RemovalReference, Validation},
};
use cudarc::driver::{CudaSlice, DeviceRepr, PinnedHostSlice, PushKernelArg, ValidAsZeroBits};
use minase::eval::features::FEATURE_COUNT;

/// カーネル1〜7の全出力と検証統計をホストへ回収した結果。
pub struct Evaluation {
    /// CPU参照と同じ配置の前進の出力。
    pub forward: Forward,
    /// CPU参照と同じ配置の補助損失、除去の微分、および部分和。
    pub removal: Removal,
    /// CPU参照と同じ配置の出力部の寄与。
    pub output: Output,
    /// バッチ平均の二値交差エントロピーと補助損失。
    pub losses: Losses,
    /// 第1層の勾配[R,H]。埋め草行を含む。
    pub first_layer: Vec<f32>,
    /// 線形経路の正準行の勾配[6841,2]。
    pub linear: Vec<f32>,
    /// 仮想行の勾配[13681,H]。P型では空。
    pub virtual_rows: Vec<f32>,
    /// 鏡映共有を合算した出力重みの勾配[2, units]。
    pub output_weights: Vec<f32>,
    /// 鏡映共有を合算したバイアスの勾配[units]。
    pub biases: Vec<f32>,
    /// 出力バイアスの勾配。
    pub output_bias: f32,
    /// 検証の統計量。
    pub validation: Validation,
}

pub(super) struct DeviceModel {
    pub shape: Shape,
    pub weights: CudaSlice<f32>,
    pub linear: CudaSlice<f32>,
    pub bias: CudaSlice<f32>,
    pub output: CudaSlice<f32>,
    pub output_bias: CudaSlice<f32>,
    reference: CudaSlice<i64>,
}
impl DeviceModel {
    pub(super) fn new(
        device: &Device,
        model: &Model,
        reference: &RemovalReference,
    ) -> Result<Self> {
        model.parameters().validate()?;
        let s = &device.compute;
        Ok(Self {
            shape: model.shape(),
            weights: s.clone_htod(&reference::fold(model)?)?,
            linear: s.clone_htod(&model.parameters()[Parameter::Linear])?,
            bias: s.clone_htod(&model.expanded_biases())?,
            output: s.clone_htod(&model.expanded_output_weights())?,
            output_bias: s.clone_htod(&model.parameters()[Parameter::OutputBias])?,
            reference: s.clone_htod(reference.rows().as_flattened())?,
        })
    }
}
pub(super) struct Buffers {
    acc: CudaSlice<f32>,
    lin: CudaSlice<f32>,
    f: CudaSlice<f32>,
    h: CudaSlice<f32>,
    logits: CudaSlice<f32>,
    loss: CudaSlice<f32>,
    g: CudaSlice<f32>,
    d_acc: CudaSlice<f32>,
    d_output: CudaSlice<f32>,
    c: CudaSlice<f32>,
    pub targets: CudaSlice<f32>,
    pub host_targets: PinnedHostSlice<f32>,
    removal_s: CudaSlice<f32>,
    removal_acc: CudaSlice<f32>,
    removal_output: CudaSlice<f32>,
    removal_linear: CudaSlice<f32>,
    e_r: CudaSlice<f64>,
    delta_r: CudaSlice<f64>,
    ref_r: CudaSlice<i32>,
    s_r: CudaSlice<f32>,
    eligible: CudaSlice<u8>,
    wrong_r: CudaSlice<u8>,
    pen_r: CudaSlice<f64>,
    d_r: CudaSlice<f32>,
    removal_loss: CudaSlice<f64>,
    pub d_first: CudaSlice<f32>,
    pub d_linear: CudaSlice<f32>,
    pub d_virtual: Option<CudaSlice<f32>>,
    first_partials: CudaSlice<f64>,
    linear_partials: CudaSlice<f64>,
    loss_partials: CudaSlice<f64>,
    gradient_partials: CudaSlice<f64>,
    validation_partials: CudaSlice<f64>,
    pub loss_sum: CudaSlice<f64>,
    pub gradient_sums: CudaSlice<f64>,
    validation_sums: CudaSlice<f64>,
}
impl Buffers {
    pub(super) fn new(device: &Device, shape: Shape, capacity: usize) -> Result<Self> {
        let s = &device.compute;
        let activations = capacity * 2 * shape.width();
        let blocks = capacity.div_ceil(THREADS);
        let head_blocks = capacity.div_ceil(WARPS_PER_BLOCK);
        let columns = 3 * shape.units() + 1;
        Ok(Self {
            acc: s.alloc_zeros(activations)?,
            lin: s.alloc_zeros(capacity * 2)?,
            f: s.alloc_zeros(activations)?,
            h: s.alloc_zeros(activations)?,
            logits: s.alloc_zeros(capacity)?,
            loss: s.alloc_zeros(capacity)?,
            g: s.alloc_zeros(capacity)?,
            d_acc: s.alloc_zeros(activations)?,
            d_output: s.alloc_zeros(activations)?,
            c: s.alloc_zeros(capacity * 2)?,
            targets: s.alloc_zeros(capacity)?,
            host_targets: pinned_zeros(device, capacity)?,
            removal_s: s.alloc_zeros(capacity)?,
            removal_acc: s.alloc_zeros(activations)?,
            removal_output: s.alloc_zeros(activations)?,
            removal_linear: s.alloc_zeros(capacity * 2)?,
            e_r: s.alloc_zeros(capacity * 144)?,
            delta_r: s.alloc_zeros(capacity * 144)?,
            ref_r: s.alloc_zeros(capacity * 144)?,
            s_r: s.alloc_zeros(capacity * 144)?,
            eligible: s.alloc_zeros(capacity * 144)?,
            wrong_r: s.alloc_zeros(capacity * 144)?,
            pen_r: s.alloc_zeros(capacity * 144)?,
            d_r: s.alloc_zeros(capacity * 144)?,
            removal_loss: s.alloc_zeros(capacity)?,
            d_first: s.alloc_zeros(shape.len(Parameter::FirstLayer))?,
            d_linear: s.alloc_zeros(shape.len(Parameter::Linear))?,
            d_virtual: match shape.feature_set() {
                InputFeatureSet::P => None,
                InputFeatureSet::HalfKpHmB8 => Some(s.alloc_zeros(shape.len(Parameter::Virtual))?),
            },
            first_partials: s.alloc_zeros(
                max_segment_count(
                    shape.feature_set().first_layer_rows(),
                    capacity * 2 * FEATURE_COLUMNS,
                ) * shape.width(),
            )?,
            linear_partials: s
                .alloc_zeros(max_segment_count(CANONICAL_ROWS, capacity * FEATURE_COLUMNS) * 2)?,
            loss_partials: s.alloc_zeros(head_blocks * 2)?,
            gradient_partials: s.alloc_zeros(blocks * columns)?,
            validation_partials: s.alloc_zeros(head_blocks * 7)?,
            loss_sum: s.alloc_zeros(2)?,
            gradient_sums: s.alloc_zeros(columns)?,
            validation_sums: s.alloc_zeros(7)?,
        })
    }
}

/// 起動時にカーネルをコンパイルし、モデルと最大バッチ分の作業配列を保持する。
/// 同期して回収するAPIは等価性検査用。入力の再利用制御はHostRingに置く。
pub struct Evaluator {
    device: Device,
    kernels: Kernels,
    model: DeviceModel,
    ring: HostRing,
    buffers: Buffers,
    capacity: usize,
}
impl Evaluator {
    /// CPUのモデルをアップロードし、カーネルと3面・2面の環を用意する。
    pub fn new(
        device: Device,
        model: &Model,
        reference: &RemovalReference,
        capacity: usize,
    ) -> Result<Self> {
        let ring = HostRing::new(&device, model.shape().feature_set(), capacity)?;
        let kernels = Kernels::compile(&device)?;
        let buffers = Buffers::new(&device, model.shape(), capacity)?;
        let model = DeviceModel::new(&device, model, reference)?;
        Ok(Self {
            device,
            kernels,
            model,
            ring,
            buffers,
            capacity,
        })
    }
    /// 利用中のGPU文脈。
    pub fn device(&self) -> &Device {
        &self.device
    }

    fn prepare(&mut self, batch: &BatchSlot, targets: &[f32]) -> Result<super::BatchTicket> {
        validate_inputs(self.model.shape, self.capacity, batch, targets)?;
        let b = &mut self.buffers;
        b.host_targets.as_mut_slice()?[..targets.len()].copy_from_slice(targets);
        self.device
            .transfer
            .memcpy_htod(&b.host_targets, &mut b.targets)?;
        let targets_done = self.device.transfer.record_event(None)?;
        self.device.compute.wait(&targets_done)?;
        self.ring.upload(batch)
    }

    /// カーネル1〜7と検証を実行し、全出力を回収する。モデルの更新は行わない。
    pub fn evaluate(&mut self, batch: &BatchSlot, targets: &[f32]) -> Result<Evaluation> {
        let ticket = self.prepare(batch, targets)?;
        let input = self.ring.input(&ticket)?;
        forward(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
            None,
        )?;
        removal_forward(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
            None,
        )?;
        output_backward(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
            None,
        )?;
        gradients(
            &self.device,
            &self.kernels,
            self.model.shape,
            batch.len(),
            &mut self.buffers,
            None,
        )?;
        backward(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
            None,
        )?;
        validation(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
        )?;
        self.ring.finish(ticket)?;
        self.read_evaluation(batch.len())
    }

    /// カーネル1・9だけで検証する。出力部の後退と勾配縮約は起動しない。
    pub fn validate(&mut self, batch: &BatchSlot, targets: &[f32]) -> Result<Validation> {
        let ticket = self.prepare(batch, targets)?;
        let input = self.ring.input(&ticket)?;
        forward(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
            None,
        )?;
        validation(
            &self.device,
            &self.kernels,
            &self.model,
            input,
            &mut self.buffers,
        )?;
        self.ring.finish(ticket)?;
        self.read_validation(batch.len())
    }
    fn read_validation(&self, n: usize) -> Result<Validation> {
        let sums = read(&self.device, &self.buffers.validation_sums, 7)?;
        // 符号一致数と飽和数を整数に変換する前に、生の縮約値を検査する。
        // NaNを整数へ変換してもエラーにならず、異常を隠してしまうため。
        if sums.iter().any(|value| !value.is_finite()) {
            return Err(Error::NonFinite("validation loss or NNUE output"));
        }
        Ok(Validation::from_sums(&sums, n, self.model.shape.width()))
    }
    fn read_evaluation(&self, n: usize) -> Result<Evaluation> {
        let b = &self.buffers;
        let h = self.model.shape.width();
        let u = self.model.shape.units();
        let a = n * 2 * h;
        let sums = read(&self.device, &b.gradient_sums, 3 * u + 1)?;
        let losses = read(&self.device, &b.loss_sum, 2)?;
        let virtual_rows = match &b.d_virtual {
            Some(values) => read(
                &self.device,
                values,
                self.model.shape.len(Parameter::Virtual),
            )?,
            None => Vec::new(),
        };
        Ok(Evaluation {
            forward: Forward {
                acc: read(&self.device, &b.acc, a)?,
                lin: pairs(read(&self.device, &b.lin, n * 2)?),
            },
            removal: Removal {
                e_r: read(&self.device, &b.e_r, n * 144)?,
                delta_r: read(&self.device, &b.delta_r, n * 144)?,
                ref_r: read(&self.device, &b.ref_r, n * 144)?,
                s_r: read(&self.device, &b.s_r, n * 144)?,
                eligible: read(&self.device, &b.eligible, n * 144)?
                    .iter()
                    .map(|&x| x != 0)
                    .collect(),
                wrong_r: read(&self.device, &b.wrong_r, n * 144)?
                    .iter()
                    .map(|&x| x != 0)
                    .collect(),
                pen_r: read(&self.device, &b.pen_r, n * 144)?,
                d_r: read(&self.device, &b.d_r, n * 144)?,
                s: read(&self.device, &b.removal_s, n)?,
                d_acc_removal: read(&self.device, &b.removal_acc, a)?,
                d_output: read(&self.device, &b.removal_output, a)?,
                linear: pairs(read(&self.device, &b.removal_linear, n * 2)?),
                loss: read(&self.device, &b.removal_loss, n)?,
            },
            output: Output {
                f: read(&self.device, &b.f, a)?,
                h: read(&self.device, &b.h, a)?,
                logits: read(&self.device, &b.logits, n)?,
                loss: read(&self.device, &b.loss, n)?,
                g: read(&self.device, &b.g, n)?,
                d_acc: read(&self.device, &b.d_acc, a)?,
                d_output: read(&self.device, &b.d_output, a)?,
                c: pairs(read(&self.device, &b.c, n * 2)?),
            },
            losses: Losses {
                bce: losses[0] / n as f64,
                auxiliary: losses[1] / n as f64,
            },
            first_layer: read(
                &self.device,
                &b.d_first,
                self.model.shape.len(Parameter::FirstLayer),
            )?,
            linear: read(
                &self.device,
                &b.d_linear,
                self.model.shape.len(Parameter::Linear),
            )?,
            virtual_rows,
            output_weights: sums[..2 * u].iter().map(|&x| x as f32).collect(),
            biases: sums[2 * u..3 * u].iter().map(|&x| x as f32).collect(),
            output_bias: sums[3 * u] as f32,
            validation: self.read_validation(n)?,
        })
    }
}
fn pairs(values: Vec<f32>) -> Vec<[f32; 2]> {
    values.as_chunks::<2>().0.to_vec()
}
fn read<T: DeviceRepr + ValidAsZeroBits + Copy>(
    device: &Device,
    src: &CudaSlice<T>,
    len: usize,
) -> Result<Vec<T>> {
    require_len(src.len(), len)?;
    let mut host = pinned_zeros(device, len)?;
    device.compute.memcpy_dtoh(&src.slice(..len), &mut host)?;
    Ok(host.as_slice()?.to_vec())
}
pub(super) fn validate_inputs(
    shape: Shape,
    capacity: usize,
    batch: &BatchSlot,
    targets: &[f32],
) -> Result<()> {
    if batch.is_empty()
        || batch.len() > capacity
        || batch.feature_set() != shape.feature_set()
        || targets.len() != batch.len()
        || targets
            .iter()
            .any(|&x| !x.is_finite() || !(0.0..=1.0).contains(&x))
    {
        return Err(Error::Invalid("batch shape, capacity, or targets"));
    }
    Ok(())
}

pub(super) fn forward(
    d: &Device,
    k: &Kernels,
    m: &DeviceModel,
    input: &DeviceInput,
    b: &mut Buffers,
    mut profile: Option<&mut KernelProfile>,
) -> Result<()> {
    let n = input.len;
    let width = m.shape.width();
    require_len(input.features.len(), n * 2 * FEATURE_COLUMNS)?;
    require_len(input.linear_features.len(), n * FEATURE_COLUMNS)?;
    require_len(m.weights.len(), m.shape.len(Parameter::FirstLayer))?;
    require_len(m.bias.len(), width)?;
    require_len(m.linear.len(), CANONICAL_ROWS * 2)?;
    require_len(b.acc.len(), n * 2 * width)?;
    require_len(b.lin.len(), n * 2)?;
    let (n, h, p) = scalars(m.shape, n);
    let cfg = launch_config(n as usize * 2 * width.div_ceil(4))?;
    let mut args = d.compute.launch_builder(&k.acc);
    args.arg(&input.features)
        .arg(&m.weights)
        .arg(&m.bias)
        .arg(&mut b.acc)
        .arg(&n)
        .arg(&h)
        .arg(&p);
    // SAFETY: 配列長は上で検査済み。特徴番号は非公開フィールドを持つBatchSlotが
    // 検証済み局面から作った範囲内の番号か番兵。カーネルは端数を検査し、accだけを書く。
    // 全配列・関数は同じDeviceに属し、cudarcのイベント追跡が寿命と転送依存を保つ。
    unsafe {
        launch_kernel(&mut args, cfg, Kernel::ForwardAcc, profile.as_deref_mut())?;
    }
    let cfg = launch_config(n as usize * 2)?;
    let mut args = d.compute.launch_builder(&k.linear);
    args.arg(&input.linear_features)
        .arg(&m.linear)
        .arg(&mut b.lin)
        .arg(&n);
    // SAFETY: 上でB×145、6841×2、B×2を検査済み。正準行と番兵はBatchSlotの不変条件。
    // forward_linearの引数順・型と一致し、lin以外を書き換えない。
    unsafe {
        launch_kernel(&mut args, cfg, Kernel::ForwardLinear, profile)?;
    }
    Ok(())
}
fn scalars(shape: Shape, n: usize) -> (u32, u32, u32) {
    (
        n as u32,
        shape.width() as u32,
        u32::from(shape.feature_set() == crate::data::InputFeatureSet::P),
    )
}
fn check_head(m: &DeviceModel, input: &DeviceInput, b: &Buffers) -> Result<()> {
    let n = input.len;
    require_len(b.acc.len(), n * 2 * m.shape.width())?;
    require_len(b.lin.len(), n * 2)?;
    require_len(input.piece_counts.len(), n)?;
    require_len(m.output.len(), 2 * m.shape.width())?;
    require_len(m.output_bias.len(), 1)?;
    require_len(b.targets.len(), n)?;
    Ok(())
}
pub(super) fn removal_forward(
    d: &Device,
    k: &Kernels,
    m: &DeviceModel,
    input: &DeviceInput,
    b: &mut Buffers,
    profile: Option<&mut KernelProfile>,
) -> Result<()> {
    check_head(m, input, b)?;
    let n = input.len;
    let width = m.shape.width();
    require_len(input.features.len(), n * 2 * FEATURE_COLUMNS)?;
    require_len(input.linear_features.len(), n * FEATURE_COLUMNS)?;
    require_len(m.weights.len(), m.shape.len(Parameter::FirstLayer))?;
    require_len(m.linear.len(), CANONICAL_ROWS * 2)?;
    require_len(m.reference.len(), CANONICAL_ROWS * 2)?;
    for len in [
        b.e_r.len(),
        b.delta_r.len(),
        b.ref_r.len(),
        b.s_r.len(),
        b.eligible.len(),
        b.wrong_r.len(),
        b.pen_r.len(),
        b.d_r.len(),
    ] {
        require_len(len, n * 144)?;
    }
    require_len(b.removal_s.len(), n)?;
    require_len(b.removal_acc.len(), n * 2 * width)?;
    require_len(b.removal_output.len(), n * 2 * width)?;
    require_len(b.removal_linear.len(), n * 2)?;
    require_len(b.removal_loss.len(), n)?;
    let cfg = cudarc::driver::LaunchConfig {
        grid_dim: (n as u32, 1, 1),
        block_dim: (THREADS as u32, 1, 1),
        shared_mem_bytes: 0,
    };
    let (n, h, p) = scalars(m.shape, n);
    let scale = m.shape.k();
    let mut args = d.compute.launch_builder(&k.removal);
    args.arg(&input.features)
        .arg(&input.linear_features)
        .arg(&input.piece_counts)
        .arg(&m.weights)
        .arg(&m.linear)
        .arg(&m.output)
        .arg(&m.output_bias)
        .arg(&b.acc)
        .arg(&b.lin)
        .arg(&m.reference)
        .arg(&mut b.e_r)
        .arg(&mut b.delta_r)
        .arg(&mut b.ref_r)
        .arg(&mut b.s_r)
        .arg(&mut b.eligible)
        .arg(&mut b.wrong_r)
        .arg(&mut b.pen_r)
        .arg(&mut b.d_r)
        .arg(&mut b.removal_s)
        .arg(&mut b.removal_acc)
        .arg(&mut b.removal_output)
        .arg(&mut b.removal_linear)
        .arg(&mut b.removal_loss)
        .arg(&n)
        .arg(&h)
        .arg(&p)
        .arg(&scale);
    // SAFETY: 各配列の長さを検査済み。特徴番号と駒数はBatchSlotの不変条件で保証。
    // referenceは検査済みRemovalReferenceのi64端点で、CUDAのlong longと同じ表現。
    // 1ブロックが1局面を担当し、共有配列の範囲は144列、幅はShapeで2〜512に制限。
    unsafe {
        launch_kernel(&mut args, cfg, Kernel::RemovalForward, profile)?;
    }
    Ok(())
}

pub(super) fn output_backward(
    d: &Device,
    k: &Kernels,
    m: &DeviceModel,
    input: &DeviceInput,
    b: &mut Buffers,
    mut profile: Option<&mut KernelProfile>,
) -> Result<()> {
    check_head(m, input, b)?;
    let n = input.len;
    for array in [
        &b.f,
        &b.h,
        &b.d_acc,
        &b.d_output,
        &b.removal_acc,
        &b.removal_output,
    ] {
        require_len(array.len(), n * 2 * m.shape.width())?;
    }
    for array in [&b.logits, &b.loss, &b.g, &b.removal_s] {
        require_len(array.len(), n)?;
    }
    for array in [&b.c, &b.removal_linear] {
        require_len(array.len(), n * 2)?;
    }
    require_len(b.removal_loss.len(), n)?;
    let blocks = n.div_ceil(WARPS_PER_BLOCK);
    require_len(b.loss_partials.len(), blocks * 2)?;
    let mut cfg = launch_config(blocks * THREADS)?;
    cfg.shared_mem_bytes = (2 * WARPS_PER_BLOCK * size_of::<f64>()) as u32;
    let (n, h, p) = scalars(m.shape, n);
    let scale = m.shape.k();
    let mut args = d.compute.launch_builder(&k.head);
    args.arg(&b.acc)
        .arg(&b.lin)
        .arg(&input.piece_counts)
        .arg(&m.output)
        .arg(&m.output_bias)
        .arg(&b.targets)
        .arg(&b.removal_s)
        .arg(&b.removal_acc)
        .arg(&b.removal_output)
        .arg(&b.removal_linear)
        .arg(&b.removal_loss)
        .arg(&mut b.f)
        .arg(&mut b.h)
        .arg(&mut b.logits)
        .arg(&mut b.loss)
        .arg(&mut b.g)
        .arg(&mut b.d_acc)
        .arg(&mut b.d_output)
        .arg(&mut b.c)
        .arg(&mut b.loss_partials)
        .arg(&n)
        .arg(&h)
        .arg(&p)
        .arg(&scale);
    // SAFETY: 全配列の範囲と共有メモリ2×4×f64を上で検査・指定済み。
    // 1ワープが1局面を書き、末尾の未使用ワープもブロック同期に参加する。
    // CUDAの引数順・型と一致し、出力には排他的な参照を渡す。
    unsafe {
        launch_kernel(
            &mut args,
            cfg,
            Kernel::OutputBackward,
            profile.as_deref_mut(),
        )?;
    }
    reduce(d, k, &b.loss_partials, &mut b.loss_sum, blocks, 2, profile)
}

pub(super) fn backward(
    d: &Device,
    k: &Kernels,
    m: &DeviceModel,
    input: &DeviceInput,
    b: &mut Buffers,
    mut profile: Option<&mut KernelProfile>,
) -> Result<()> {
    let n = input.len;
    let shape = m.shape;
    let width = shape.width();
    require_len(
        input.first_segment_starts.len(),
        shape.feature_set().first_layer_rows() + 1,
    )?;
    require_len(input.first_segment_offsets.len(), input.first_segments + 1)?;
    require_len(input.first_entries.len(), input.first_len)?;
    require_len(input.features.len(), n * 2 * FEATURE_COLUMNS)?;
    require_len(m.weights.len(), shape.len(Parameter::FirstLayer))?;
    require_len(m.output.len(), 2 * width)?;
    require_len(b.acc.len(), n * 2 * width)?;
    require_len(b.d_acc.len(), n * 2 * width)?;
    require_len(b.d_r.len(), n * 144)?;
    require_len(b.d_first.len(), shape.len(Parameter::FirstLayer))?;
    require_len(b.first_partials.len(), input.first_segments * width)?;
    let rows = shape.rows() as u32;
    let (_, h, p) = scalars(shape, n);
    if input.first_segments != 0 {
        let cfg = cudarc::driver::LaunchConfig {
            grid_dim: (input.first_segments as u32, 1, 1),
            block_dim: (THREADS as u32, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut args = d.compute.launch_builder(&k.first_backward_partials);
        args.arg(&input.first_segment_offsets)
            .arg(&input.first_entries)
            .arg(&input.features)
            .arg(&m.weights)
            .arg(&b.acc)
            .arg(&m.output)
            .arg(&b.d_r)
            .arg(&b.d_acc)
            .arg(&mut b.first_partials)
            .arg(&h)
            .arg(&p);
        // SAFETY: 入出力の長さを検査済み。BatchSlotの区分は同じ行の128要素以内で、
        // 区分数のブロックだけを起動する。各スレッドは区分内の別々のunitを担当する。
        // 列144はd_rを読まず、回転はH未満。部分和の各要素は1回だけf64で書く。
        unsafe {
            launch_kernel(
                &mut args,
                cfg,
                Kernel::FirstLayerBackwardPartials,
                profile.as_deref_mut(),
            )?;
        }
    }
    let cfg = launch_config(shape.rows() * width)?;
    let mut args = d.compute.launch_builder(&k.first_backward_reduce);
    args.arg(&input.first_segment_starts)
        .arg(&b.first_partials)
        .arg(&mut b.d_first)
        .arg(&rows)
        .arg(&h);
    // SAFETY: 行ごとの区分範囲と部分和の容量を検査済み。各スレッドは(row,unit)の
    // 1要素だけを書き、前段が埋めた区分だけを昇順に読む。埋め草行と空行は0を書く。
    // HalfKPの埋め草行では範囲外になるsegment_startsを読まない。
    unsafe {
        launch_kernel(
            &mut args,
            cfg,
            Kernel::FirstLayerBackwardReduce,
            profile.as_deref_mut(),
        )?;
    }

    require_len(input.linear_segment_starts.len(), CANONICAL_ROWS + 1)?;
    require_len(
        input.linear_segment_offsets.len(),
        input.linear_segments + 1,
    )?;
    require_len(input.linear_entries.len(), input.linear_len)?;
    require_len(input.piece_counts.len(), n)?;
    require_len(b.c.len(), n * 2)?;
    require_len(b.d_linear.len(), CANONICAL_ROWS * 2)?;
    require_len(b.linear_partials.len(), input.linear_segments * 2)?;
    let rows = CANONICAL_ROWS as u32;
    let scale = shape.k();
    if input.linear_segments != 0 {
        let segments = input.linear_segments as u32;
        let cfg = cudarc::driver::LaunchConfig {
            grid_dim: (input.linear_segments.div_ceil(WARPS_PER_BLOCK) as u32, 1, 1),
            block_dim: (THREADS as u32, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut args = d.compute.launch_builder(&k.linear_backward_partials);
        args.arg(&input.linear_segment_offsets)
            .arg(&input.linear_entries)
            .arg(&input.piece_counts)
            .arg(&b.d_r)
            .arg(&b.c)
            .arg(&mut b.linear_partials)
            .arg(&segments)
            .arg(&scale);
        // SAFETY: 区分表、逆索引、B局面分の入力、2端点の部分和の容量を検査済み。
        // 1ワープは128要素以内の1区分を担当し、全レーンが固定順シャッフルに参加する。
        // 末尾の余剰ワープは一斉に終了し、有効ワープのレーン0だけが2端点を書く。
        unsafe {
            launch_kernel(
                &mut args,
                cfg,
                Kernel::LinearBackwardPartials,
                profile.as_deref_mut(),
            )?;
        }
    }
    let cfg = launch_config(CANONICAL_ROWS)?;
    let mut args = d.compute.launch_builder(&k.linear_backward_reduce);
    args.arg(&input.linear_segment_starts)
        .arg(&b.linear_partials)
        .arg(&mut b.d_linear)
        .arg(&rows);
    // SAFETY: 6842個の区分開始位置、全区分の部分和、6841×2の出力を検査済み。
    // 各スレッドが1行の2端点を区分順に合算し、空行は0を書く。
    unsafe {
        launch_kernel(
            &mut args,
            cfg,
            Kernel::LinearBackwardReduce,
            profile.as_deref_mut(),
        )?;
    }

    if shape.feature_set() == InputFeatureSet::HalfKpHmB8 {
        let d_virtual = b
            .d_virtual
            .as_mut()
            .ok_or(Error::Invalid("missing virtual gradient"))?;
        require_len(d_virtual.len(), shape.len(Parameter::Virtual))?;
        let cfg = launch_config((FEATURE_COUNT + 1) * width.div_ceil(4))?;
        let mut args = d.compute.launch_builder(&k.virtual_reduce);
        args.arg(&b.d_first).arg(d_virtual).arg(&h);
        // SAFETY: d_firstは109441×H、d_virtualは13681×Hを検査済み。
        // 各スレッドは8区分の同一基底行だけを読み、出力の別々の4列以内を書き込む。
        // 埋め草は入力を読まず0にし、H未満の列だけを処理する。
        unsafe {
            launch_kernel(&mut args, cfg, Kernel::ReduceVirtual, profile)?;
        }
    }
    Ok(())
}
pub(super) fn gradients(
    d: &Device,
    k: &Kernels,
    shape: Shape,
    n: usize,
    b: &mut Buffers,
    mut profile: Option<&mut KernelProfile>,
) -> Result<()> {
    let columns = 3 * shape.units() + 1;
    require_len(b.d_acc.len(), n * 2 * shape.width())?;
    require_len(b.d_output.len(), n * 2 * shape.width())?;
    require_len(b.g.len(), n)?;
    require_len(b.gradient_partials.len(), n.div_ceil(THREADS) * columns)?;
    let cfg = launch_config(n)?;
    let (n32, h, _) = scalars(shape, n);
    let units = shape.units() as u32;
    let chunk = THREADS as u32;
    let mut args = d.compute.launch_builder(&k.gradients);
    args.arg(&b.d_acc)
        .arg(&b.d_output)
        .arg(&b.g)
        .arg(&mut b.gradient_partials)
        .arg(&n32)
        .arg(&h)
        .arg(&units)
        .arg(&chunk);
    // SAFETY: 入出力の長さを検査済み。unitsは検査済みShapeのHまたはH/2で0ではない。
    // 各ブロックは独立した部分和を書き、末尾の局面はBで打ち切る。
    unsafe {
        launch_kernel(
            &mut args,
            cfg,
            Kernel::GradientPartials,
            profile.as_deref_mut(),
        )?;
    }
    reduce(
        d,
        k,
        &b.gradient_partials,
        &mut b.gradient_sums,
        n.div_ceil(THREADS),
        columns,
        profile,
    )
}
fn reduce(
    d: &Device,
    k: &Kernels,
    partials: &CudaSlice<f64>,
    sums: &mut CudaSlice<f64>,
    blocks: usize,
    columns: usize,
    profile: Option<&mut KernelProfile>,
) -> Result<()> {
    require_len(partials.len(), blocks * columns)?;
    require_len(sums.len(), columns)?;
    let cfg = launch_config(columns)?;
    let blocks = blocks as u32;
    let columns = columns as u32;
    let mut args = d.compute.launch_builder(&k.reduce);
    args.arg(partials).arg(sums).arg(&blocks).arg(&columns);
    // SAFETY: partials[blocks,columns]とsums[columns]を検査済み。各スレッドは
    // 1列だけを固定順で読み、対応する出力1要素だけを書き込む。
    unsafe {
        launch_kernel(
            &mut args,
            cfg,
            Kernel::ReducePartials {
                blocks: blocks as usize,
                columns: columns as usize,
            },
            profile,
        )?;
    }
    Ok(())
}
fn validation(
    d: &Device,
    k: &Kernels,
    m: &DeviceModel,
    input: &DeviceInput,
    b: &mut Buffers,
) -> Result<()> {
    check_head(m, input, b)?;
    let n = input.len;
    require_len(input.outcomes.len(), n)?;
    let blocks = n.div_ceil(WARPS_PER_BLOCK);
    require_len(b.validation_partials.len(), blocks * 7)?;
    let mut cfg = launch_config(blocks * THREADS)?;
    cfg.shared_mem_bytes = (7 * WARPS_PER_BLOCK * size_of::<f64>()) as u32;
    let (n32, h, p) = scalars(m.shape, n);
    let scale = m.shape.k();
    let mut args = d.compute.launch_builder(&k.validation);
    args.arg(&b.acc)
        .arg(&b.lin)
        .arg(&input.piece_counts)
        .arg(&input.outcomes)
        .arg(&m.output)
        .arg(&m.output_bias)
        .arg(&b.targets)
        .arg(&mut b.validation_partials)
        .arg(&n32)
        .arg(&h)
        .arg(&p)
        .arg(&scale);
    // SAFETY: 入出力の長さを検査済み。共有メモリは7×4個のf64を確保。
    // 1ワープが1局面を担当し、末尾の未使用ワープもブロック同期に参加する。
    // ブロック内で各出力列を1スレッドだけが書く。
    unsafe {
        args.launch(cfg)?;
    }
    reduce(
        d,
        k,
        &b.validation_partials,
        &mut b.validation_sums,
        blocks,
        7,
        None,
    )
}

use super::{
    Device, Error, HostRing, Result, THREADS, WARPS_PER_BLOCK,
    device::{Kernels, pinned_zeros},
    launch_config, require_len,
    ring::DeviceInput,
    step::{Kernel, KernelProfile, launch_kernel},
};
use crate::nnue::{
    data::{BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, max_segment_count},
    model::{Model, Parameter, Shape},
    reference::{self, Forward, Losses, Output, Validation},
};
use cudarc::driver::{CudaSlice, DeviceRepr, PinnedHostSlice, PushKernelArg, ValidAsZeroBits};
use minase::eval::features::FEATURE_COUNT;

/// 前進、後退の全出力と検証統計をホストへ回収した結果。
pub struct Evaluation {
    /// CPU参照と同じ配置の前進の出力。
    pub forward: Forward,
    /// 出力部の局面ごとの寄与。oとW1は共有を畳み込んだCPU参照と同じ配置。
    pub output: Output,
    /// バッチ平均の二値交差エントロピー。
    pub losses: Losses,
    /// 第1層の勾配`[R,H]`。埋め草行を含む。
    pub first_layer: Vec<f32>,
    /// 線形経路の正準行の勾配`[6841,2]`。
    pub linear: Vec<f32>,
    /// 仮想行の勾配`[13681,H]`。P型では空。
    pub virtual_rows: Vec<f32>,
    /// 鏡映共有を合算した出力重みの勾配[2, units]。
    pub output_weights: Vec<f32>,
    /// 鏡映共有を合算したバイアスの勾配`[units]`。
    pub biases: Vec<f32>,
    /// 出力バイアスの勾配。
    pub output_bias: f32,
    /// 隠れ層の中間値`[B,15]`。
    pub hidden1_z: Vec<f32>,
    /// 隠れ層の中間値`[B,30]`。
    pub hidden1_u: Vec<f32>,
    /// 隠れ層の中間値`[B,32]`。
    pub hidden2_a: Vec<f32>,
    /// 隠れ層の中間値`[B,32]`。
    pub hidden2_y: Vec<f32>,
    /// W1、b1、W2、b2、vの自由パラメータの勾配。
    pub head_gradients: [Vec<f32>; 5],
    /// 検証の統計量。
    pub validation: Validation,
}

/// CUDAの勾配列。直接出力、累算値バイアス、出力バイアス、W1、b1、W2、b2、vの順。
pub(super) const HEAD_PARAMETERS: [Parameter; 8] = [
    Parameter::OutputWeights,
    Parameter::Biases,
    Parameter::OutputBias,
    Parameter::Hidden1Weights,
    Parameter::Hidden1Biases,
    Parameter::Hidden2Weights,
    Parameter::Hidden2Biases,
    Parameter::Hidden2Output,
];
pub(super) fn gradient_columns(shape: Shape) -> usize {
    HEAD_PARAMETERS.iter().map(|&p| shape.len(p)).sum()
}
pub(super) fn gradient_offset(shape: Shape, parameter: Parameter) -> usize {
    HEAD_PARAMETERS
        .iter()
        .take_while(|&&p| p != parameter)
        .map(|&p| shape.len(p))
        .sum()
}
pub(super) fn expanded_len(shape: Shape, parameter: Parameter) -> usize {
    if parameter == Parameter::Biases {
        shape.width()
    } else {
        shape.len(parameter)
    }
}

pub(super) struct DeviceModel {
    pub shape: Shape,
    pub weights: CudaSlice<f32>,
    pub linear: CudaSlice<f32>,
    pub bias: CudaSlice<f32>,
    pub output: CudaSlice<f32>,
    pub output_bias: CudaSlice<f32>,
    pub w1: CudaSlice<f32>,
    pub b1: CudaSlice<f32>,
    pub w2: CudaSlice<f32>,
    pub b2: CudaSlice<f32>,
    pub v: CudaSlice<f32>,
}
impl DeviceModel {
    pub(super) fn new(device: &Device, model: &Model) -> Result<Self> {
        model.parameters().validate()?;
        let s = &device.compute;
        Ok(Self {
            shape: model.shape(),
            weights: s.clone_htod(&reference::fold(model)?)?,
            linear: s.clone_htod(&model.parameters()[Parameter::Linear])?,
            bias: s.clone_htod(&model.expanded_biases())?,
            output: s.clone_htod(&model.parameters()[Parameter::OutputWeights])?,
            output_bias: s.clone_htod(&model.parameters()[Parameter::OutputBias])?,
            w1: s.clone_htod(&model.parameters()[Parameter::Hidden1Weights])?,
            b1: s.clone_htod(&model.parameters()[Parameter::Hidden1Biases])?,
            w2: s.clone_htod(&model.parameters()[Parameter::Hidden2Weights])?,
            b2: s.clone_htod(&model.parameters()[Parameter::Hidden2Biases])?,
            v: s.clone_htod(&model.parameters()[Parameter::Hidden2Output])?,
        })
    }
}
pub(super) struct Buffers {
    acc: CudaSlice<f32>,
    lin: CudaSlice<f32>,
    f: CudaSlice<f32>,
    logits: CudaSlice<f32>,
    loss: CudaSlice<f32>,
    g: CudaSlice<f32>,
    d_acc: CudaSlice<f32>,
    c: CudaSlice<f32>,
    d_output: CudaSlice<f32>,
    d_output_bias: CudaSlice<f32>,
    d_hidden1_weights: CudaSlice<f32>,
    d_hidden1_biases: CudaSlice<f32>,
    d_hidden2_weights: CudaSlice<f32>,
    d_hidden2_biases: CudaSlice<f32>,
    d_hidden2_output: CudaSlice<f32>,
    hidden1_z: CudaSlice<f32>,
    hidden1_u: CudaSlice<f32>,
    hidden2_a: CudaSlice<f32>,
    hidden2_y: CudaSlice<f32>,
    pub targets: CudaSlice<f32>,
    pub host_targets: PinnedHostSlice<f32>,
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
        let columns = gradient_columns(shape);
        Ok(Self {
            acc: s.alloc_zeros(activations)?,
            lin: s.alloc_zeros(capacity * 2)?,
            f: s.alloc_zeros(capacity * shape.len(Parameter::OutputWeights))?,
            logits: s.alloc_zeros(capacity)?,
            loss: s.alloc_zeros(capacity)?,
            g: s.alloc_zeros(capacity)?,
            d_acc: s.alloc_zeros(activations)?,
            c: s.alloc_zeros(capacity * 2)?,
            d_output: s.alloc_zeros(capacity * shape.len(Parameter::OutputWeights))?,
            d_output_bias: s.alloc_zeros(capacity * shape.len(Parameter::OutputBias))?,
            d_hidden1_weights: s.alloc_zeros(capacity * shape.len(Parameter::Hidden1Weights))?,
            d_hidden1_biases: s.alloc_zeros(capacity * shape.len(Parameter::Hidden1Biases))?,
            d_hidden2_weights: s.alloc_zeros(capacity * shape.len(Parameter::Hidden2Weights))?,
            d_hidden2_biases: s.alloc_zeros(capacity * shape.len(Parameter::Hidden2Biases))?,
            d_hidden2_output: s.alloc_zeros(capacity * shape.len(Parameter::Hidden2Output))?,
            hidden1_z: s.alloc_zeros(capacity * 15)?,
            hidden1_u: s.alloc_zeros(capacity * 30)?,
            hidden2_a: s.alloc_zeros(capacity * 32)?,
            hidden2_y: s.alloc_zeros(capacity * 32)?,
            targets: s.alloc_zeros(capacity)?,
            host_targets: pinned_zeros(device, capacity)?,
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
            loss_partials: s.alloc_zeros(head_blocks)?,
            gradient_partials: s.alloc_zeros(blocks * columns)?,
            validation_partials: s.alloc_zeros(head_blocks * 12)?,
            loss_sum: s.alloc_zeros(1)?,
            gradient_sums: s.alloc_zeros(columns)?,
            validation_sums: s.alloc_zeros(12)?,
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
    pub fn new(device: Device, model: &Model, capacity: usize) -> Result<Self> {
        let ring = HostRing::new(&device, model.shape().feature_set(), capacity)?;
        let kernels = Kernels::compile(&device)?;
        let buffers = Buffers::new(&device, model.shape(), capacity)?;
        let model = DeviceModel::new(&device, model)?;
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

    /// 前進、後退と検証を実行し、全出力を回収する。
    /// モデルの更新は行わない。
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

    /// 前進と検証だけを実行する。
    /// 出力部の後退と勾配縮約は起動しない。
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
        let mut sums = [0.0; 12];
        sums.copy_from_slice(&read(&self.device, &self.buffers.validation_sums, 12)?);
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
        let u = self.model.shape.output_units();
        let bias_units = self.model.shape.bias_units();
        let a = n * 2 * h;
        let shape = self.model.shape;
        let sums = read(&self.device, &b.gradient_sums, gradient_columns(shape))?;
        let head_gradients = std::array::from_fn(|i| {
            let p = HEAD_PARAMETERS[i + 3];
            let offset = gradient_offset(shape, p);
            sums[offset..offset + shape.len(p)]
                .iter()
                .map(|&x| x as f32)
                .collect()
        });
        let losses = read(&self.device, &b.loss_sum, 1)?;
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
            output: Output {
                f: read(&self.device, &b.f, n * 2 * u)?,
                logits: read(&self.device, &b.logits, n)?,
                loss: read(&self.device, &b.loss, n)?,
                g: read(&self.device, &b.g, n)?,
                d_acc: read(&self.device, &b.d_acc, a)?,
                d_output: read(
                    &self.device,
                    &b.d_output,
                    n * shape.len(Parameter::OutputWeights),
                )?,
                d_output_bias: read(
                    &self.device,
                    &b.d_output_bias,
                    n * shape.len(Parameter::OutputBias),
                )?,
                d_hidden1_weights: read(
                    &self.device,
                    &b.d_hidden1_weights,
                    n * shape.len(Parameter::Hidden1Weights),
                )?,
                d_hidden1_biases: read(
                    &self.device,
                    &b.d_hidden1_biases,
                    n * shape.len(Parameter::Hidden1Biases),
                )?,
                d_hidden2_weights: read(
                    &self.device,
                    &b.d_hidden2_weights,
                    n * shape.len(Parameter::Hidden2Weights),
                )?,
                d_hidden2_biases: read(
                    &self.device,
                    &b.d_hidden2_biases,
                    n * shape.len(Parameter::Hidden2Biases),
                )?,
                d_hidden2_output: read(
                    &self.device,
                    &b.d_hidden2_output,
                    n * shape.len(Parameter::Hidden2Output),
                )?,
                c: pairs(read(&self.device, &b.c, n * 2)?),
            },
            losses: Losses {
                bce: losses[0] / n as f64,
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
            biases: sums[2 * u..2 * u + bias_units]
                .iter()
                .map(|&x| x as f32)
                .collect(),
            output_bias: sums[2 * u + bias_units] as f32,
            hidden1_z: read(&self.device, &b.hidden1_z, n * 15)?,
            hidden1_u: read(&self.device, &b.hidden1_u, n * 30)?,
            hidden2_a: read(&self.device, &b.hidden2_a, n * 32)?,
            hidden2_y: read(&self.device, &b.hidden2_y, n * 32)?,
            head_gradients,
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
    if len == 0 {
        return Ok(Vec::new());
    }
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
        u32::from(shape.feature_set() == crate::nnue::data::InputFeatureSet::P),
    )
}
fn check_head(m: &DeviceModel, input: &DeviceInput, b: &Buffers) -> Result<()> {
    let n = input.len;
    require_len(b.acc.len(), n * 2 * m.shape.width())?;
    require_len(b.lin.len(), n * 2)?;
    require_len(input.piece_counts.len(), n)?;
    require_len(m.output.len(), m.shape.len(Parameter::OutputWeights))?;
    require_len(m.output_bias.len(), m.shape.len(Parameter::OutputBias))?;
    require_len(m.w1.len(), m.shape.len(Parameter::Hidden1Weights))?;
    require_len(m.b1.len(), m.shape.len(Parameter::Hidden1Biases))?;
    require_len(m.w2.len(), m.shape.len(Parameter::Hidden2Weights))?;
    require_len(m.b2.len(), m.shape.len(Parameter::Hidden2Biases))?;
    require_len(m.v.len(), m.shape.len(Parameter::Hidden2Output))?;
    require_len(b.targets.len(), n)?;
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
    require_len(b.f.len(), n * m.shape.len(Parameter::OutputWeights))?;
    require_len(b.d_acc.len(), n * 2 * m.shape.width())?;
    for array in [&b.logits, &b.loss, &b.g] {
        require_len(array.len(), n)?;
    }
    require_len(b.c.len(), n * 2)?;
    require_len(b.d_output.len(), n * m.shape.len(Parameter::OutputWeights))?;
    require_len(
        b.d_output_bias.len(),
        n * m.shape.len(Parameter::OutputBias),
    )?;
    require_len(
        b.d_hidden1_weights.len(),
        n * m.shape.len(Parameter::Hidden1Weights),
    )?;
    require_len(
        b.d_hidden1_biases.len(),
        n * m.shape.len(Parameter::Hidden1Biases),
    )?;
    require_len(
        b.d_hidden2_weights.len(),
        n * m.shape.len(Parameter::Hidden2Weights),
    )?;
    require_len(
        b.d_hidden2_biases.len(),
        n * m.shape.len(Parameter::Hidden2Biases),
    )?;
    require_len(
        b.d_hidden2_output.len(),
        n * m.shape.len(Parameter::Hidden2Output),
    )?;
    require_len(b.hidden1_z.len(), n * 15)?;
    require_len(b.hidden1_u.len(), n * 30)?;
    require_len(b.hidden2_a.len(), n * 32)?;
    require_len(b.hidden2_y.len(), n * 32)?;
    let blocks = n.div_ceil(WARPS_PER_BLOCK);
    require_len(b.loss_partials.len(), blocks)?;
    let mut cfg = launch_config(blocks * THREADS)?;
    cfg.shared_mem_bytes = (WARPS_PER_BLOCK * size_of::<f64>()) as u32;
    let (n, h, p) = scalars(m.shape, n);
    let scale = m.shape.cp_per_logit();
    let mut args = d.compute.launch_builder(&k.head);
    args.arg(&b.acc)
        .arg(&b.lin)
        .arg(&input.piece_counts)
        .arg(&m.output)
        .arg(&m.output_bias)
        .arg(&m.w1)
        .arg(&m.b1)
        .arg(&m.w2)
        .arg(&m.b2)
        .arg(&m.v)
        .arg(&b.targets)
        .arg(&mut b.f)
        .arg(&mut b.logits)
        .arg(&mut b.loss)
        .arg(&mut b.g)
        .arg(&mut b.d_acc)
        .arg(&mut b.c)
        .arg(&mut b.d_output)
        .arg(&mut b.d_output_bias)
        .arg(&mut b.d_hidden1_weights)
        .arg(&mut b.d_hidden1_biases)
        .arg(&mut b.d_hidden2_weights)
        .arg(&mut b.d_hidden2_biases)
        .arg(&mut b.d_hidden2_output)
        .arg(&mut b.hidden1_z)
        .arg(&mut b.hidden1_u)
        .arg(&mut b.hidden2_a)
        .arg(&mut b.hidden2_y)
        .arg(&mut b.loss_partials)
        .arg(&n)
        .arg(&h)
        .arg(&p)
        .arg(&scale);
    // SAFETY: 全配列の範囲と共有メモリ4×f64を上で検査・指定済み。
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
    reduce(d, k, &b.loss_partials, &mut b.loss_sum, blocks, 1, profile)
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
    require_len(b.d_acc.len(), n * 2 * width)?;
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
            .arg(&b.d_acc)
            .arg(&mut b.first_partials)
            .arg(&h)
            .arg(&p);
        // SAFETY: 入出力の長さを検査済み。BatchSlotの区分は同じ行の128要素以内で、
        // 区分数のブロックだけを起動する。各スレッドは区分内の別々のunitを担当する。
        // 回転はH未満。部分和の各要素は1回だけf64で書く。
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
    require_len(b.c.len(), n * 2)?;
    require_len(b.d_linear.len(), CANONICAL_ROWS * 2)?;
    require_len(b.linear_partials.len(), input.linear_segments * 2)?;
    let rows = CANONICAL_ROWS as u32;
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
            .arg(&b.c)
            .arg(&mut b.linear_partials)
            .arg(&segments);
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
    let columns = gradient_columns(shape);
    require_len(b.d_acc.len(), n * 2 * shape.width())?;
    for (parameter, array) in [
        (Parameter::OutputWeights, &b.d_output),
        (Parameter::OutputBias, &b.d_output_bias),
        (Parameter::Hidden1Weights, &b.d_hidden1_weights),
        (Parameter::Hidden1Biases, &b.d_hidden1_biases),
        (Parameter::Hidden2Weights, &b.d_hidden2_weights),
        (Parameter::Hidden2Biases, &b.d_hidden2_biases),
        (Parameter::Hidden2Output, &b.d_hidden2_output),
    ] {
        require_len(array.len(), n * shape.len(parameter))?;
    }
    require_len(b.gradient_partials.len(), n.div_ceil(THREADS) * columns)?;
    let cfg = launch_config(n)?;
    let (n32, h, _) = scalars(shape, n);
    let bias_units = shape.bias_units() as u32;
    let output_units = shape.output_units() as u32;
    let chunk = THREADS as u32;
    let mut args = d.compute.launch_builder(&k.gradients);
    args.arg(&b.d_acc)
        .arg(&b.d_output)
        .arg(&b.d_output_bias)
        .arg(&b.d_hidden1_weights)
        .arg(&b.d_hidden1_biases)
        .arg(&b.d_hidden2_weights)
        .arg(&b.d_hidden2_biases)
        .arg(&b.d_hidden2_output)
        .arg(&mut b.gradient_partials)
        .arg(&n32)
        .arg(&h)
        .arg(&bias_units)
        .arg(&output_units)
        .arg(&chunk);
    // SAFETY: 入出力の長さを検査済み。bias_unitsとoutput_unitsは検査済みShapeから得た正の値。
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
    require_len(b.validation_partials.len(), blocks * 12)?;
    let mut cfg = launch_config(blocks * THREADS)?;
    cfg.shared_mem_bytes = (12 * WARPS_PER_BLOCK * size_of::<f64>()) as u32;
    let (n32, h, p) = scalars(m.shape, n);
    let scale = m.shape.cp_per_logit();
    let mut args = d.compute.launch_builder(&k.validation);
    args.arg(&b.acc)
        .arg(&b.lin)
        .arg(&input.piece_counts)
        .arg(&input.outcomes)
        .arg(&m.output)
        .arg(&m.output_bias)
        .arg(&m.w1)
        .arg(&m.b1)
        .arg(&m.w2)
        .arg(&m.b2)
        .arg(&m.v)
        .arg(&b.targets)
        .arg(&mut b.validation_partials)
        .arg(&n32)
        .arg(&h)
        .arg(&p)
        .arg(&scale);
    // SAFETY: 入出力の長さを検査済み。共有メモリは12×4個のf64を確保。
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
        12,
        None,
    )
}

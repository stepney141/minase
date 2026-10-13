use super::{
    Device, Error, Result,
    device::{Kernels, pinned_zeros},
    engine::{Buffers, DeviceModel},
    launch_config, require_len,
    step::{Kernel, KernelProfile, launch_kernel},
};
use crate::nn::AdamWGroup;
use crate::nnue::{
    data::InputFeatureSet,
    model::{Model, Parameter, Parameters, Shape},
    reference::{AdamW, AdamWConfig},
};
use cudarc::driver::{CudaSlice, PushKernelArg};

struct Group {
    parameter: Parameter,
    values: CudaSlice<f32>,
    moment: CudaSlice<f32>,
    variance: CudaSlice<f32>,
    observed: Option<CudaSlice<u8>>,
}

struct Update<'a> {
    correction1: f64,
    correction2: f64,
    group: AdamWGroup,
    nonfinite: &'a mut CudaSlice<u32>,
    profile: Option<&'a mut KernelProfile>,
}

/// 自由パラメータ、観測マスク、AdamWの状態をデバイス上に保持する。
pub(super) struct Optimizer {
    shape: Shape,
    groups: [Option<Group>; 6],
    step: u64,
    config: AdamWConfig,
}
impl Optimizer {
    pub fn new(device: &Device, model: &Model, optimizer: &AdamW) -> Result<Self> {
        let shape = model.shape();
        if shape != optimizer.m().shape() || shape != optimizer.v().shape() {
            return Err(Error::Invalid("AdamW parameter shape"));
        }
        model.parameters().validate()?;
        optimizer.config().validate()?;
        optimizer.m().validate()?;
        optimizer.v().validate()?;
        if Parameter::ALL
            .iter()
            .any(|&p| optimizer.v()[p].iter().any(|&v| v < 0.0))
        {
            return Err(Error::Invalid("negative AdamW variance"));
        }
        let mut groups = std::array::from_fn(|_| None);
        let stream = &device.compute;
        for p in Parameter::ALL {
            if shape.len(p) == 0 {
                continue;
            }
            let mask = match p {
                Parameter::Linear => Some(model.masks().canonical()),
                Parameter::FirstLayer => Some(model.masks().first_layer()),
                Parameter::Virtual => Some(model.masks().virtual_rows()),
                _ => None,
            };
            let observed = match mask {
                Some(mask) => {
                    let bytes: Vec<_> = mask.iter().map(|&seen| u8::from(seen)).collect();
                    Some(stream.clone_htod(&bytes)?)
                }
                None => None,
            };
            groups[p as usize] = Some(Group {
                parameter: p,
                values: stream.clone_htod(&model.parameters()[p])?,
                moment: stream.clone_htod(&optimizer.m()[p])?,
                variance: stream.clone_htod(&optimizer.v()[p])?,
                observed,
            });
        }
        Ok(Self {
            shape,
            groups,
            step: optimizer.step(),
            config: optimizer.config(),
        })
    }

    pub fn step(&self) -> u64 {
        self.step
    }

    /// 全群をLinear、P型ならFirstLayer、HalfKPならVirtual→FirstLayer、Biases、OutputWeights、OutputBiasの順に計算ストリームへ投入する。
    /// 補正係数はCPUと同じ`powf`で`1/(1−0.9^t)`と`1/(1−0.999^t)`を求める。
    /// 出力部は`gradient_sums`の位置0から出力重み、2Uからバイアス、3Uから出力バイアスをf32にして使う。
    /// ホストへの転送は行わない。
    pub fn update(
        &mut self,
        d: &Device,
        k: &Kernels,
        model: &mut DeviceModel,
        b: &Buffers,
        nonfinite: &mut CudaSlice<u32>,
        mut profile: Option<&mut KernelProfile>,
    ) -> Result<()> {
        let step = self
            .step
            .checked_add(1)
            .ok_or(Error::Invalid("AdamW step overflow"))?;
        self.validate(model, b)?;
        require_len(nonfinite.len(), 1)?;
        // CPU参照と同じpowfで分母を求め、逆数を全群のカーネルで共有する。
        let correction1 = 1.0 / (1.0 - 0.9_f64.powf(step as f64));
        let correction2 = 1.0 / (1.0 - 0.999_f64.powf(step as f64));
        let shape = self.shape;
        let config = self.config;
        self.group_mut(Parameter::Linear)?.rows(
            d,
            k,
            &b.d_linear,
            &mut model.linear,
            2,
            Update {
                correction1,
                correction2,
                group: config.linear,
                nonfinite,
                profile: profile.as_deref_mut(),
            },
        )?;
        match shape.feature_set() {
            InputFeatureSet::P => self.group_mut(Parameter::FirstLayer)?.rows(
                d,
                k,
                &b.d_first,
                &mut model.weights,
                shape.width(),
                Update {
                    correction1,
                    correction2,
                    group: config.ft,
                    nonfinite,
                    profile: profile.as_deref_mut(),
                },
            )?,
            InputFeatureSet::HalfKpHmB8 => {
                self.group_mut(Parameter::Virtual)?.virtual_rows(
                    d,
                    k,
                    b.d_virtual
                        .as_ref()
                        .ok_or(Error::Invalid("missing virtual gradient"))?,
                    shape.width(),
                    Update {
                        correction1,
                        correction2,
                        group: config.ft,
                        nonfinite,
                        profile: profile.as_deref_mut(),
                    },
                )?;
                // 別々の群を分割して借り、更新済みの仮想行を読み取り専用にする。
                let (first, virtual_rows) = self.groups.split_at_mut(Parameter::Virtual as usize);
                first[Parameter::FirstLayer as usize]
                    .as_mut()
                    .ok_or(Error::Invalid("missing first-layer optimizer"))?
                    .first(
                        d,
                        k,
                        &b.d_first,
                        &virtual_rows[0]
                            .as_ref()
                            .ok_or(Error::Invalid("missing virtual optimizer"))?
                            .values,
                        &mut model.weights,
                        (
                            shape.width(),
                            Update {
                                correction1,
                                correction2,
                                group: config.ft,
                                nonfinite,
                                profile: profile.as_deref_mut(),
                            },
                        ),
                    )?;
            }
        }
        self.group_mut(Parameter::Biases)?.head(
            d,
            k,
            &b.gradient_sums,
            &mut model.bias,
            shape,
            Update {
                correction1,
                correction2,
                group: config.head,
                nonfinite,
                profile: profile.as_deref_mut(),
            },
        )?;
        self.group_mut(Parameter::OutputWeights)?.head(
            d,
            k,
            &b.gradient_sums,
            &mut model.output,
            shape,
            Update {
                correction1,
                correction2,
                group: config.head,
                nonfinite,
                profile: profile.as_deref_mut(),
            },
        )?;
        self.group_mut(Parameter::OutputBias)?.head(
            d,
            k,
            &b.gradient_sums,
            &mut model.output_bias,
            shape,
            Update {
                correction1,
                correction2,
                group: config.head,
                nonfinite,
                profile,
            },
        )?;
        self.step = step;
        Ok(())
    }

    fn group_mut(&mut self, p: Parameter) -> Result<&mut Group> {
        self.groups[p as usize]
            .as_mut()
            .ok_or(Error::Invalid("missing optimizer group"))
    }

    fn validate(&self, model: &DeviceModel, b: &Buffers) -> Result<()> {
        if model.shape != self.shape {
            return Err(Error::Invalid("optimizer and device model shape"));
        }
        let s = self.shape;
        for p in Parameter::ALL {
            match &self.groups[p as usize] {
                Some(group) => {
                    require_len(group.values.len(), s.len(p))?;
                    require_len(group.moment.len(), s.len(p))?;
                    require_len(group.variance.len(), s.len(p))?;
                    let rows = match p {
                        Parameter::Linear => s.len(p) / 2,
                        Parameter::FirstLayer | Parameter::Virtual => s.len(p) / s.width(),
                        _ => 0,
                    };
                    if rows != 0 {
                        require_len(
                            group
                                .observed
                                .as_ref()
                                .ok_or(Error::Invalid("missing optimizer mask"))?
                                .len(),
                            rows,
                        )?;
                    }
                }
                None if s.len(p) == 0 => {}
                None => return Err(Error::Invalid("missing optimizer group")),
            }
        }
        require_len(model.linear.len(), s.len(Parameter::Linear))?;
        require_len(model.weights.len(), s.len(Parameter::FirstLayer))?;
        require_len(model.bias.len(), s.width())?;
        require_len(model.output.len(), 2 * s.width())?;
        require_len(model.output_bias.len(), 1)?;
        require_len(b.d_linear.len(), s.len(Parameter::Linear))?;
        require_len(b.d_first.len(), s.len(Parameter::FirstLayer))?;
        require_len(b.gradient_sums.len(), 3 * s.units() + 1)?;
        if s.feature_set() == InputFeatureSet::HalfKpHmB8 {
            require_len(
                b.d_virtual
                    .as_ref()
                    .ok_or(Error::Invalid("missing virtual gradient"))?
                    .len(),
                s.len(Parameter::Virtual),
            )?;
        }
        Ok(())
    }

    /// 検査・保存用に自由パラメータと最適化器を同期して回収する。
    pub fn snapshot(&self, device: &Device) -> Result<(Parameters, AdamW)> {
        let mut parameters = Parameters::zeros(self.shape);
        let mut m = Parameters::zeros(self.shape);
        let mut v = Parameters::zeros(self.shape);
        for p in Parameter::ALL {
            if let Some(group) = &self.groups[p as usize] {
                copy_to_host(device, &group.values, &mut parameters[p])?;
                copy_to_host(device, &group.moment, &mut m[p])?;
                copy_to_host(device, &group.variance, &mut v[p])?;
            }
        }
        Ok((
            parameters,
            AdamW {
                m,
                v,
                step: self.step,
                config: self.config,
            },
        ))
    }
}

impl Group {
    fn rows(
        &mut self,
        d: &Device,
        k: &Kernels,
        gradient: &CudaSlice<f32>,
        expanded: &mut CudaSlice<f32>,
        row_width: usize,
        update: Update<'_>,
    ) -> Result<()> {
        let count = self.values.len() as u32;
        let row_width = row_width as u32;
        let (lower, upper) = match self.parameter {
            Parameter::Linear => (-4096.0_f32, 4095.875_f32),
            Parameter::FirstLayer => (-8.0_f32, 8.0_f32),
            _ => return Err(Error::Invalid("optimizer row parameter")),
        };
        let observed = self
            .observed
            .as_ref()
            .ok_or(Error::Invalid("missing optimizer mask"))?;
        let mut args = d.compute.launch_builder(&k.optimizer_rows);
        args.arg(&mut self.values)
            .arg(gradient)
            .arg(&mut self.moment)
            .arg(&mut self.variance)
            .arg(observed)
            .arg(expanded)
            .arg(&count)
            .arg(&row_width)
            .arg(&lower)
            .arg(&upper)
            .arg(&update.correction1)
            .arg(&update.correction2)
            .arg(&update.group.lr)
            .arg(&update.group.weight_decay)
            .arg(update.nonfinite);
        // SAFETY: update冒頭で全配列と行マスクの長さを検査済み。列数は2または
        // Shapeの幅であり、末尾を検査する1スレッドが1要素を排他的に書く。
        // 非有限フラグは1要素以上を確保済みで、atomicOrだけが共有して書く。
        unsafe {
            launch_kernel(
                &mut args,
                launch_config(count as usize)?,
                Kernel::OptimizerRows(self.parameter),
                update.profile,
            )?;
        }
        Ok(())
    }

    fn virtual_rows(
        &mut self,
        d: &Device,
        k: &Kernels,
        gradient: &CudaSlice<f32>,
        width: usize,
        update: Update<'_>,
    ) -> Result<()> {
        let count = self.values.len() as u32;
        let h = width as u32;
        let observed = self
            .observed
            .as_ref()
            .ok_or(Error::Invalid("missing virtual optimizer mask"))?;
        let mut args = d.compute.launch_builder(&k.optimizer_virtual);
        args.arg(&mut self.values)
            .arg(gradient)
            .arg(&mut self.moment)
            .arg(&mut self.variance)
            .arg(observed)
            .arg(&count)
            .arg(&h)
            .arg(&update.correction1)
            .arg(&update.correction2)
            .arg(&update.group.lr)
            .arg(&update.group.weight_decay)
            .arg(update.nonfinite);
        // SAFETY: 13681×Hの各状態と13681行のマスクを検査済み。Hは検査済みShape
        // の正の幅。配列の所属と寿命は所有元Deviceとcudarcのイベント追跡が保つ。
        // 非有限フラグの1要素を全スレッドがatomicOrで更新する。
        unsafe {
            launch_kernel(
                &mut args,
                launch_config(count as usize)?,
                Kernel::OptimizerVirtual,
                update.profile,
            )?;
        }
        Ok(())
    }

    fn first(
        &mut self,
        d: &Device,
        k: &Kernels,
        gradient: &CudaSlice<f32>,
        virtual_rows: &CudaSlice<f32>,
        expanded: &mut CudaSlice<f32>,
        (width, update): (usize, Update<'_>),
    ) -> Result<()> {
        let count = self.values.len() as u32;
        let h = width as u32;
        let observed = self
            .observed
            .as_ref()
            .ok_or(Error::Invalid("missing first-layer optimizer mask"))?;
        let mut args = d.compute.launch_builder(&k.optimizer_first);
        args.arg(&mut self.values)
            .arg(gradient)
            .arg(&mut self.moment)
            .arg(&mut self.variance)
            .arg(observed)
            .arg(virtual_rows)
            .arg(expanded)
            .arg(&count)
            .arg(&h)
            .arg(&update.correction1)
            .arg(&update.correction2)
            .arg(&update.group.lr)
            .arg(&update.group.weight_decay)
            .arg(update.nonfinite);
        // SAFETY: WとCは109441×H、Vは13681×Hで長さを検査済み。基底行の対応は
        // paddingを含めCUDA側で範囲内に写る。同一ストリーム上の先行起動がVを更新済み。
        // 非有限フラグの1要素を全スレッドがatomicOrで更新する。
        unsafe {
            launch_kernel(
                &mut args,
                launch_config(count as usize)?,
                Kernel::OptimizerFirst,
                update.profile,
            )?;
        }
        Ok(())
    }

    fn head(
        &mut self,
        d: &Device,
        k: &Kernels,
        gradient_sums: &CudaSlice<f64>,
        expanded: &mut CudaSlice<f32>,
        shape: Shape,
        update: Update<'_>,
    ) -> Result<()> {
        let count = self.values.len() as u32;
        let (gradient_offset, units, h, limit) = match self.parameter {
            Parameter::Biases => (2 * shape.units(), shape.units(), shape.width(), 8.0_f32),
            Parameter::OutputWeights => (0, shape.units(), shape.width(), 1.98_f32),
            Parameter::OutputBias => (3 * shape.units(), 1, 1, 8.0_f32),
            _ => return Err(Error::Invalid("optimizer head parameter")),
        };
        let gradient_offset = gradient_offset as u32;
        let units = units as u32;
        let h = h as u32;
        let mut args = d.compute.launch_builder(&k.optimizer_head);
        args.arg(&mut self.values)
            .arg(gradient_sums)
            .arg(&mut self.moment)
            .arg(&mut self.variance)
            .arg(expanded)
            .arg(&gradient_offset)
            .arg(&count)
            .arg(&units)
            .arg(&h)
            .arg(&limit)
            .arg(&update.correction1)
            .arg(&update.correction2)
            .arg(&update.group.lr)
            .arg(&update.group.weight_decay)
            .arg(update.nonfinite);
        // SAFETY: 3×units+1個の勾配と各群の自由・展開配列を検査済み。Pの両半分は
        // 同じスレッドが書く。unitsはHまたはH/2で正であり、全要素の書込先は重ならない。
        // 非有限フラグの1要素だけは全スレッドがatomicOrで共有する。
        unsafe {
            launch_kernel(
                &mut args,
                launch_config(count as usize)?,
                Kernel::OptimizerHead(self.parameter),
                update.profile,
            )?;
        }
        Ok(())
    }
}

fn copy_to_host(device: &Device, src: &CudaSlice<f32>, dst: &mut [f32]) -> Result<()> {
    let mut host = pinned_zeros(device, dst.len())?;
    device.compute.memcpy_dtoh(src, &mut host)?;
    dst.copy_from_slice(host.as_slice()?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::reference::RemovalReference;

    #[test]
    #[ignore = "requires a CUDA GPU and NVRTC"]
    fn gpu_optimizer_checks_every_gradient_group_including_unobserved_rows() {
        let device = Device::new(0).unwrap();
        let kernels = Kernels::compile(&device).unwrap();
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let shape = Shape::new(set, 8, 1000.0).unwrap();
            let model = super::super::equivalence::random_model(shape, 71).unwrap();
            let adamw = AdamW::new(shape, AdamWConfig::default()).unwrap();
            for parameter in Parameter::ALL {
                if shape.len(parameter) == 0 {
                    continue;
                }
                let mut optimizer = Optimizer::new(&device, &model, &adamw).unwrap();
                let mut expanded =
                    DeviceModel::new(&device, &model, &RemovalReference::v0()).unwrap();
                let mut buffers = Buffers::new(&device, shape, 1).unwrap();
                let mut status = device.compute.alloc_zeros::<u32>(1).unwrap();
                match parameter {
                    Parameter::Linear | Parameter::FirstLayer | Parameter::Virtual => {
                        let gradient = match parameter {
                            Parameter::Linear => &mut buffers.d_linear,
                            Parameter::FirstLayer => &mut buffers.d_first,
                            Parameter::Virtual => buffers.d_virtual.as_mut().unwrap(),
                            _ => unreachable!(),
                        };
                        // 最後の埋め草行は未観測。更新しない行でも勾配を検査する。
                        let last = gradient.len() - 1;
                        device
                            .compute
                            .memcpy_htod(&[f32::NAN], &mut gradient.slice_mut(last..))
                            .unwrap();
                    }
                    Parameter::Biases | Parameter::OutputWeights | Parameter::OutputBias => {
                        let offset = match parameter {
                            Parameter::OutputWeights => 0,
                            Parameter::Biases => 2 * shape.units(),
                            Parameter::OutputBias => 3 * shape.units(),
                            _ => unreachable!(),
                        };
                        device
                            .compute
                            .memcpy_htod(
                                &[f64::INFINITY],
                                &mut buffers.gradient_sums.slice_mut(offset..offset + 1),
                            )
                            .unwrap();
                    }
                }
                optimizer
                    .update(
                        &device,
                        &kernels,
                        &mut expanded,
                        &buffers,
                        &mut status,
                        None,
                    )
                    .unwrap();
                assert_ne!(
                    device.compute.clone_dtoh(&status).unwrap()[0] & 1,
                    0,
                    "{parameter:?}"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires a CUDA GPU and NVRTC"]
    fn gpu_optimizer_rejects_nonfinite_moment_from_finite_gradient() {
        let device = Device::new(0).unwrap();
        let shape = Shape::new(InputFeatureSet::P, 8, 1000.0).unwrap();
        let model = super::super::equivalence::random_model(shape, 71).unwrap();
        let adamw = AdamW::new(shape, AdamWConfig::default()).unwrap();
        let mut optimizer = Optimizer::new(&device, &model, &adamw).unwrap();
        let mut expanded = DeviceModel::new(&device, &model, &RemovalReference::v0()).unwrap();
        let mut buffers = Buffers::new(&device, shape, 1).unwrap();
        let mut status = device.compute.alloc_zeros::<u32>(1).unwrap();
        device
            .compute
            .memcpy_htod(
                &[f64::from(f32::MAX)],
                &mut buffers.gradient_sums.slice_mut(..1),
            )
            .unwrap();
        let kernels = Kernels::compile(&device).unwrap();
        optimizer
            .update(
                &device,
                &kernels,
                &mut expanded,
                &buffers,
                &mut status,
                None,
            )
            .unwrap();
        let bits = device.compute.clone_dtoh(&status).unwrap()[0];
        assert_eq!(bits & 1, 0);
        assert_ne!(bits & 2, 0);
    }
}

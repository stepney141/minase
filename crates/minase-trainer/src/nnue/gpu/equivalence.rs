//! 合成局面と非零モデルによるGPUとCPU参照の等価性検査。
//! 浮動小数点は`|GPU−CPU| ≤ ATOL + RTOL×|CPU|`で比較し、ATOL=1e-6、RTOL=1e-5とする。
//! 整数と真偽値は完全一致を要求し、非有限値を拒否する。
//! 同じ入力を2回実行し、GPUの反復間のビット一致も検査する。
//! テストでは幅2、6、8、128、130ごとに同じ評価器へ64、4,096、100、1、129局面を順に渡す。
//! 回転、番兵、先獅子、補間率の境界、空の逆索引範囲、および3分類の教師値を含める。

use super::{Device, Error, Evaluation, Evaluator, Result};
use crate::data::{Outcome, Sample};
use crate::nnue::{
    data::{BatchSlot, CANONICAL_ROWS, InputFeatureSet, MAX_BATCH_SIZE, build_targets},
    model::{Masks, Model, Parameter, Parameters, Shape},
    options::TrainingOptions,
    reference,
};
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};

/// CPUとの比較に使う相対許容誤差。
pub const RTOL: f64 = 1e-5;
/// CPUとの比較に使う絶対許容誤差。
pub const ATOL: f64 = 1e-6;

/// 全中間値、縮約値、更新後の状態、および反復のビット一致の検査結果。
#[derive(Debug)]
pub struct Report {
    /// 実行デバイスの名前。
    pub device_name: String,
    /// 比較した数値の数。
    pub comparisons: usize,
    /// 観測した絶対誤差の最大値。
    pub max_absolute_error: f64,
    /// 参照値が非零の場合の相対誤差の最大値。
    pub max_relative_error: f64,
}
impl Report {
    pub(super) fn compare(&mut self, name: &str, expected: &[f32], actual: &[f32]) -> Result<()> {
        if expected.len() != actual.len() {
            return Err(Error::Equivalence(format!("{name}: length")));
        }
        for (i, (&a, &b)) in expected.iter().zip(actual).enumerate() {
            self.number(name, i, f64::from(a), f64::from(b))?;
        }
        Ok(())
    }
    pub(super) fn number(
        &mut self,
        name: &str,
        i: usize,
        expected: f64,
        actual: f64,
    ) -> Result<()> {
        let abs = (expected - actual).abs();
        self.comparisons += 1;
        self.max_absolute_error = self.max_absolute_error.max(abs);
        if expected != 0.0 {
            self.max_relative_error = self.max_relative_error.max(abs / expected.abs());
        }
        if !expected.is_finite() || !actual.is_finite() || abs > ATOL + RTOL * expected.abs() {
            return Err(Error::Equivalence(format!(
                "{name}[{i}]: CPU={expected:.12e}, GPU={actual:.12e}, abs={abs:.3e}"
            )));
        }
        Ok(())
    }
    pub(super) fn merge(&mut self, other: Self) {
        self.comparisons += other.comparisons;
        self.max_absolute_error = self.max_absolute_error.max(other.max_absolute_error);
        self.max_relative_error = self.max_relative_error.max(other.max_relative_error);
    }
}

/// 指定条件の合成入力を作り、GPUで2回実行してCPU参照と照合する。
/// 前進、全勾配、検証、1・3更新後の全状態と反復のビット一致を検査する。
pub fn run(
    set: InputFeatureSet,
    width: usize,
    batch: usize,
    seed: u64,
    training: TrainingOptions,
    head: crate::nnue::model::HeadKind,
) -> Result<Report> {
    super::check_head(head)?;
    training.validate()?;
    let shape = Shape::new(set, width, 1000.0, head)?;
    if !(1..=MAX_BATCH_SIZE).contains(&batch) {
        return Err(Error::Invalid("equivalence batch size"));
    }
    // GPU不在は大きな合成モデルを作る前に報告する。
    let device = Device::new(0)?;
    let model = random_model(shape, seed)?;
    let mut evaluator = Evaluator::new(device.clone(), &model, batch)?;
    let input = synthetic_batch(set, batch, seed)?;
    let mut report = check(
        &mut evaluator,
        &model,
        &input,
        &build_targets(&input, &[500.0, 1000.0, 1500.0], &[training.lambda; 3])?,
    )?;
    drop(evaluator);
    drop(model);
    report.merge(super::step_equivalence::run(
        device, shape, &input, seed, &training,
    )?);
    Ok(report)
}

/// 同じモデルを保持する評価器へ同一バッチを2回渡し、CPUおよびビット単位で比較する。
pub fn check(
    evaluator: &mut Evaluator,
    model: &Model,
    batch: &BatchSlot,
    targets: &[f32],
) -> Result<Report> {
    let weights = reference::fold(model)?;
    let f = reference::forward(model, batch, &weights)?;
    let output = reference::output_backward(model, batch, &f, targets)?;
    let mut gradients = Parameters::zeros(model.shape());
    let losses = reference::reduce_partials(model, &output, &mut gradients)?;
    reference::first_layer_backward(model, batch, &output, &mut gradients)?;
    reference::linear_backward(model, batch, &output, &mut gradients)?;
    reference::reduce_virtual(&mut gradients)?;
    let validation = reference::validation_forward(model, batch, &f, targets)?;
    let expected = Evaluation {
        forward: f,
        output,
        losses,
        first_layer: gradients[Parameter::FirstLayer].to_vec(),
        linear: gradients[Parameter::Linear].to_vec(),
        virtual_rows: gradients[Parameter::Virtual].to_vec(),
        output_weights: gradients[Parameter::OutputWeights].to_vec(),
        biases: gradients[Parameter::Biases].to_vec(),
        output_bias: gradients[Parameter::OutputBias][0],
        validation,
    };
    drop(gradients);
    drop(weights);
    let first = evaluator.evaluate(batch, targets)?;
    let second = evaluator.evaluate(batch, targets)?;
    let mut report = Report {
        device_name: evaluator.device().name().into(),
        comparisons: 0,
        max_absolute_error: 0.0,
        max_relative_error: 0.0,
    };
    for ((name, cpu), (_, gpu)) in arrays(&expected).into_iter().zip(arrays(&first)) {
        report.compare(name, cpu, gpu)?;
    }
    for ((name, a), (_, b)) in arrays(&first).into_iter().zip(arrays(&second)) {
        if a.len() != b.len() || a.iter().zip(b).any(|(a, b)| a.to_bits() != b.to_bits()) {
            return Err(Error::Equivalence(format!(
                "{name}: repeated GPU executions differ in bits"
            )));
        }
    }
    for (i, (a, b)) in statistics(&expected)
        .into_iter()
        .zip(statistics(&first))
        .enumerate()
    {
        report.number("statistics", i, a, b)?;
    }
    if statistics(&first)
        .iter()
        .zip(statistics(&second))
        .any(|(a, b)| a.to_bits() != b.to_bits())
    {
        return Err(Error::Equivalence(
            "repeated statistics differ in bits".into(),
        ));
    }
    let validation_only = evaluator.validate(batch, targets)?;
    let validation_again = evaluator.validate(batch, targets)?;
    for (i, ((a, b), c)) in expected
        .validation
        .sums()
        .into_iter()
        .zip(validation_only.sums())
        .zip(validation_again.sums())
        .enumerate()
    {
        report.number("validation only", i, a, b)?;
        if b.to_bits() != c.to_bits() || b.to_bits() != first.validation.sums()[i].to_bits() {
            return Err(Error::Equivalence(format!(
                "validation[{i}]: backward-free execution differs in bits"
            )));
        }
    }
    Ok(report)
}
fn arrays(e: &Evaluation) -> Vec<(&'static str, &[f32])> {
    vec![
        ("acc", &e.forward.acc),
        ("lin", e.forward.lin.as_flattened()),
        ("f", &e.output.f),
        ("h", &e.output.h),
        ("logits", &e.output.logits),
        ("loss", &e.output.loss),
        ("g", &e.output.g),
        ("d_acc", &e.output.d_acc),
        ("d_output", &e.output.d_output),
        ("c", e.output.c.as_flattened()),
        ("first layer", &e.first_layer),
        ("linear", &e.linear),
        ("virtual rows", &e.virtual_rows),
        ("output weights", &e.output_weights),
        ("biases", &e.biases),
        ("output bias", std::slice::from_ref(&e.output_bias)),
    ]
}
fn statistics(e: &Evaluation) -> Vec<f64> {
    let mut values = vec![e.losses.bce];
    values.extend(e.validation.sums());
    values.push(e.validation.loss());
    values.push(e.validation.nonlinear_std_cp());
    values.extend(e.validation.saturation());
    if let Some(accuracy) = e.validation.sign_accuracy() {
        values.push(accuracy);
    }
    values
}

pub(super) struct Random(pub(super) u64);
impl Random {
    fn next(&mut self) -> u64 {
        let result = minase_core::rng::splitmix64(self.0);
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        result
    }
    pub(super) fn centered(&mut self) -> f32 {
        ((self.next() >> 40) as f32 / 8_388_608.0) - 1.0
    }
}
pub(super) fn random_model(shape: Shape, seed: u64) -> Result<Model> {
    let mut canonical = vec![true; CANONICAL_ROWS];
    canonical[CANONICAL_ROWS - 1] = false;
    let mut input = vec![true; shape.feature_set().dimensions() + 1];
    input[shape.feature_set().dimensions()] = false;
    let masks = Masks::new(shape.feature_set(), canonical, input)?;
    let mut model = Model::initialize(
        shape,
        &vec![[0.0; 2]; minase::eval::features::FEATURE_COUNT],
        masks,
        seed,
        0.08,
    )?;
    let mut random = Random(seed ^ 0x9563_f342_a783_c621);
    for p in [
        Parameter::Linear,
        Parameter::OutputWeights,
        Parameter::OutputBias,
    ] {
        let scale = if p == Parameter::Linear { 12.0 } else { 0.3 };
        for x in model.parameter_mut(p) {
            *x = random.centered() * scale;
        }
    }
    if shape.feature_set() == InputFeatureSet::HalfKpHmB8 {
        for x in model.parameter_mut(Parameter::FirstLayer) {
            *x = random.centered() * 0.02;
        }
    }
    for (j, x) in model
        .parameter_mut(Parameter::Biases)
        .iter_mut()
        .enumerate()
    {
        *x = [-0.25, 0.0, 0.25, 1.0, 1.25][j % 5];
    }
    model.fix_unobserved();
    Ok(model)
}

pub(super) fn synthetic_batch(set: InputFeatureSet, n: usize, seed: u64) -> Result<BatchSlot> {
    if !(1..=MAX_BATCH_SIZE).contains(&n) {
        return Err(Error::Invalid("synthetic batch size"));
    }
    let mut random = Random(seed);
    let mut records = Vec::with_capacity(n);
    let mut scores = Vec::with_capacity(n);
    let mut classes = Vec::with_capacity(n);
    for b in 0..n {
        let side = if b % 2 == 0 {
            Color::Black
        } else {
            Color::White
        };
        let mut builder = PositionBuilder::new(side);
        let mut squares: Vec<_> = Square::all().collect();
        for j in (1..squares.len()).rev() {
            squares.swap(j, (random.next() % (j + 1) as u64) as usize);
        }
        let count = [2, 3, 4, 45, 90, 91, 92, 93, 144][b % 9];
        for (j, &square) in squares[..count].iter().enumerate() {
            let (color, kind) = if j < 2 {
                (if j == 0 { side } else { side.opposite() }, PieceKind::King)
            } else {
                let kinds = [
                    PieceKind::Pawn,
                    PieceKind::Rook,
                    PieceKind::Lion,
                    PieceKind::SilverGeneral,
                    PieceKind::Bishop,
                    PieceKind::GoldGeneral,
                ];
                (
                    if random.next() & 1 == 0 {
                        side
                    } else {
                        side.opposite()
                    },
                    kinds[(random.next() % kinds.len() as u64) as usize],
                )
            };
            let piece = PieceCode::new(color, kind).ok_or(Error::Invalid("synthetic piece"))?;
            builder
                .put(square, piece)
                .map_err(|_| Error::Invalid("synthetic placement"))?;
        }
        let mut position = builder
            .finish()
            .map_err(|_| Error::Invalid("synthetic position"))?;
        if b % 2 == 0 {
            // 相手の王の升は必ず手番側の駒がなく、先獅子状態の不変条件を満たす。
            position
                .set_lion_capture(Some(squares[1]))
                .map_err(|_| Error::Invalid("synthetic lion capture"))?;
        }
        let score = (random.centered() * 2000.0) as i16;
        records.push(Sample::from_position(
            &position,
            score,
            [Outcome::Loss, Outcome::Draw, Outcome::Win][b % 3],
            (b / 60000) as u32,
            (b % 60000) as u16,
            minase::training::records::Origin::RandomSelfplay,
        )?);
        scores.push(f32::from(score));
        classes.push((b % 3) as u32);
    }
    Ok(BatchSlot::from_records(set, &records, &scores, &classes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_inputs_cover_rotation_sentinels_phase_limits_and_classes() {
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let batch = synthetic_batch(set, 100, 42).unwrap();
            assert_eq!(batch, synthetic_batch(set, 100, 42).unwrap());
            assert_ne!(batch, synthetic_batch(set, 100, 43).unwrap());
            assert!(batch.features().contains(&crate::nnue::data::SENTINEL));
            assert!(batch.piece_counts().contains(&2));
            assert!(batch.piece_counts().contains(&92));
            assert!(batch.piece_counts().contains(&144));
            for c in 0..3 {
                assert!(batch.teacher_classes().contains(&c));
            }
            if set == InputFeatureSet::P {
                for rotation in [false, true] {
                    assert!(batch.first_layer_inverse().entries().iter().any(|&w| {
                        crate::nnue::data::PackedOccurrence::from_word(w)
                            .unwrap()
                            .rotated()
                            == rotation
                    }));
                }
            }
            for inverse in [batch.first_layer_inverse(), batch.linear_inverse()] {
                assert!(inverse.starts().windows(2).any(|w| w[0] == w[1]));
                assert!(inverse.entries().iter().any(|&w| {
                    crate::nnue::data::PackedOccurrence::from_word(w)
                        .unwrap()
                        .column()
                        == 144
                }));
            }
            let shape =
                Shape::new(set, 6, 1000.0, crate::nnue::model::HeadKind::DirectScrelu).unwrap();
            let targets = build_targets(&batch, &[500.0, 1000.0, 1500.0], &[1.0; 3]).unwrap();
            assert!(super::super::engine::validate_inputs(shape, 100, &batch, &targets).is_ok());
            assert!(super::super::engine::validate_inputs(shape, 99, &batch, &targets).is_err());
            assert!(
                super::super::engine::validate_inputs(shape, 100, &batch, &targets[..99]).is_err()
            );
            assert!(
                super::super::engine::validate_inputs(shape, 100, &batch, &vec![f32::NAN; 100])
                    .is_err()
            );
        }
    }
    #[test]
    fn comparison_rejects_nonfinite_values_and_wrong_outputs() {
        let mut r = Report {
            device_name: String::new(),
            comparisons: 0,
            max_absolute_error: 0.0,
            max_relative_error: 0.0,
        };
        assert!(r.number("x", 0, 1.0, 1.0000005).is_ok());
        assert!(r.number("zero", 0, 0.0, 0.5 * ATOL).is_ok());
        // 大きい値には相対項、0付近には絶対項を足した許容誤差を適用する。
        assert!(r.number("relative", 0, 100.0, 100.0 + 2.0 * ATOL).is_ok());
        assert!(r.number("absolute", 0, 0.001, 0.001 + 0.5 * ATOL).is_ok());
        assert!(r.number("additive", 0, 1.0, 1.0 + 1.05e-5).is_ok());
        assert!(r.number("additive", 0, 1.0, 1.0 + 1.2e-5).is_err());
        assert!(r.number("zero", 0, 0.0, 2.0 * ATOL).is_err());
        for x in [1.01, f64::NAN, f64::INFINITY] {
            assert!(r.number("x", 0, 1.0, x).is_err());
        }
    }

    fn matrix(set: InputFeatureSet, width: usize) {
        let device = Device::new(0).unwrap();
        let shape = Shape::new(
            set,
            width,
            1000.0,
            crate::nnue::model::HeadKind::DirectScrelu,
        )
        .unwrap();
        let model = random_model(shape, 71).unwrap();
        let mut evaluator = Evaluator::new(device.clone(), &model, 4096).unwrap();
        for n in [64, 4096, 100, 1, 129] {
            let batch = synthetic_batch(set, n, 43 + n as u64).unwrap();
            let targets = build_targets(&batch, &[500.0, 1000.0, 1500.0], &[1.0; 3]).unwrap();
            check(&mut evaluator, &model, &batch, &targets).unwrap();
        }
        drop(evaluator);
        drop(model);
    }

    #[test]
    #[ignore = "requires a CUDA GPU and NVRTC"]
    fn gpu_mixed_targets_and_custom_coefficients_match_reference_and_repeat() {
        // λ<1と全群の係数変更を含めても、絶対・相対許容誤差の和とGPU反復のビット一致の条件を満たす。
        let training = TrainingOptions {
            lr_linear: 0.02,
            weight_decay_linear: 0.003,
            lr_ft: 0.0007,
            weight_decay_ft: 0.0002,
            lr_head: 0.0004,
            weight_decay_head: 0.0003,
            lambda: 0.75,
        };
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            run(
                set,
                6,
                5,
                293,
                training,
                crate::nnue::model::HeadKind::DirectScrelu,
            )
            .unwrap();
        }
    }

    macro_rules! gpu_case {
        ($name:ident, $set:expr, $width:expr) => {
            #[test]
            #[ignore = "requires a CUDA GPU and NVRTC"]
            fn $name() {
                matrix($set, $width);
            }
        };
    }
    gpu_case!(gpu_p_width_2, InputFeatureSet::P, 2);
    gpu_case!(gpu_p_width_6, InputFeatureSet::P, 6);
    gpu_case!(gpu_p_width_8, InputFeatureSet::P, 8);
    gpu_case!(gpu_p_width_128, InputFeatureSet::P, 128);
    // H > block size and a partial warp of units exercise the strided kernels.
    gpu_case!(gpu_p_width_130, InputFeatureSet::P, 130);
    gpu_case!(gpu_halfkp_width_2, InputFeatureSet::HalfKpHmB8, 2);
    gpu_case!(gpu_halfkp_width_6, InputFeatureSet::HalfKpHmB8, 6);
    gpu_case!(gpu_halfkp_width_8, InputFeatureSet::HalfKpHmB8, 8);
    gpu_case!(gpu_halfkp_width_128, InputFeatureSet::HalfKpHmB8, 128);
    gpu_case!(gpu_halfkp_width_130, InputFeatureSet::HalfKpHmB8, 130);
}

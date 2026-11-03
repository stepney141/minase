use super::{
    Error, FIRST_LAYER_BIAS_MAX, FIRST_LAYER_SCALE, FIRST_LAYER_WEIGHT_MAX, Model, Parameter,
    Result, Shape,
};
use crate::nnue::data::InputFeatureSet;
use minase::eval::{
    features::PIECE_STATE_COUNT,
    nnue::{
        self, EncodeInput,
        feature_set::{HalfKpHmB8, P},
        head::PairwiseL16L32,
    },
};

/// 尺度と上限を検査済みの、MNUE版5に渡す整数配列。埋め草は含まない。
#[derive(Debug)]
pub struct Quantized {
    shape: Shape,
    /// 線形重み`[13680, 2]`。尺度8。
    pub linear: Vec<[i16; 2]>,
    /// 展開・畳み込み済みの第1層`[D, H]`。尺度255。
    pub weights: Vec<i16>,
    /// 第1層バイアス`[H]`。尺度255。
    pub biases: Vec<i32>,
    /// 直接出力o`[H]`。尺度4096。
    pub output_weights: Vec<i16>,
    /// 出力バイアスb。尺度127×4096。
    pub output_bias: i32,
    /// W1`[15, H]`。尺度256。
    pub hidden1_weights: Vec<i16>,
    /// b1`[15]`。尺度127×256。
    pub hidden1_biases: [i32; 15],
    /// W2`[32, 30]`。尺度256。
    pub hidden2_weights: Vec<i16>,
    /// b2`[32]`。尺度127×256。
    pub hidden2_biases: [i32; 32],
    /// 隠れ層の出力重みv`[32]`。尺度4096。
    pub hidden2_output: [i16; 32],
}
impl Quantized {
    /// 推論側のEncodeInputを使い、MNUE版5の全検査を経てバイト列を作る。
    pub fn encode(
        &self,
        piece_values: &[i32; PIECE_STATE_COUNT],
        rule_set: &str,
    ) -> Result<Vec<u8>> {
        let width = self.shape.width();
        let shared = self.shape.feature_set() == InputFeatureSet::P;
        let head = PairwiseL16L32::new(
            width,
            &self.output_weights,
            self.output_bias,
            &self.hidden1_weights,
            self.hidden1_biases,
            &self.hidden2_weights,
            self.hidden2_biases,
            self.hidden2_output,
            shared,
        )
        .map_err(Error::Mnue)?;
        let input = EncodeInput {
            width: self.shape.width() as u32,
            cp_per_logit: self.shape.cp_per_logit(),
            rule_set,
            piece_values,
            linear: &self.linear,
            weights: &self.weights,
            biases: &self.biases,
            head: &head,
        };
        match self.shape.feature_set() {
            InputFeatureSet::P => nnue::encode::<P, _>(&input),
            InputFeatureSet::HalfKpHmB8 => nnue::encode::<HalfKpHmB8, _>(&input),
        }
        .map_err(Error::Mnue)
    }
}
fn rounded(value: f32, scale: f64, low: i32, high: i32, parameter: Parameter) -> Result<i32> {
    let value = (f64::from(value) * scale).round_ties_even();
    if !value.is_finite() || value < f64::from(low) || value > f64::from(high) {
        return Err(Error::Quantization(parameter));
    }
    Ok(value as i32)
}
/// f32をf64へ広げて尺度を掛け、`round_ties_even`で偶数丸めする。
/// 線形は×8でi16、第1層は×255で±220、そのバイアスは×255で±510に収める。
/// 出力重みは×4,096で±8,192、出力バイアスは×127×4,096で±4,161,536に収める。
/// W1とW2は×256で±508のi16、b1とb2は×127×256で±260,096、vは×4,096で±8,192に収める。
/// 第1層は畳み込み済みの重みを使い、埋め草を除く。
/// 範囲外は切り詰めず[`Error::Quantization`]を返す。
pub fn quantize(model: &Model) -> Result<Quantized> {
    model.parameters().validate()?;
    let shape = model.shape();
    let linear = model
        .expanded_linear()
        .into_iter()
        .map(|row| {
            Ok([
                rounded(row[0], 8.0, -32768, 32767, Parameter::Linear)? as i16,
                rounded(row[1], 8.0, -32768, 32767, Parameter::Linear)? as i16,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let weights = model.expanded_first_layer()[..shape.feature_set().dimensions() * shape.width()]
        .iter()
        .map(|&v| {
            rounded(
                v,
                f64::from(FIRST_LAYER_SCALE),
                -FIRST_LAYER_WEIGHT_MAX,
                FIRST_LAYER_WEIGHT_MAX,
                Parameter::FirstLayer,
            )
            .map(|x| x as i16)
        })
        .collect::<Result<Vec<_>>>()?;
    let biases = model
        .expanded_biases()
        .iter()
        .map(|&v| {
            rounded(
                v,
                f64::from(FIRST_LAYER_SCALE),
                -FIRST_LAYER_BIAS_MAX,
                FIRST_LAYER_BIAS_MAX,
                Parameter::Biases,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let output_weights = model
        .expanded_output_weights()
        .iter()
        .map(|&v| rounded(v, 4096.0, -8192, 8192, Parameter::OutputWeights).map(|x| x as i16))
        .collect::<Result<Vec<_>>>()?;
    let output_bias = rounded(
        model.parameters()[Parameter::OutputBias][0],
        127.0 * 4096.0,
        -4_161_536,
        4_161_536,
        Parameter::OutputBias,
    )?;
    Ok(Quantized {
        shape,
        linear,
        weights,
        biases,
        output_weights,
        output_bias,
        hidden1_weights: model
            .expanded_hidden1_weights()
            .iter()
            .map(|&v| rounded(v, 256.0, -508, 508, Parameter::Hidden1Weights).map(|x| x as i16))
            .collect::<Result<_>>()?,
        hidden1_biases: quantized_array(
            &model.parameters()[Parameter::Hidden1Biases],
            127.0 * 256.0,
            260_096,
            Parameter::Hidden1Biases,
        )?,
        hidden2_weights: model.parameters()[Parameter::Hidden2Weights]
            .iter()
            .map(|&v| rounded(v, 256.0, -508, 508, Parameter::Hidden2Weights).map(|x| x as i16))
            .collect::<Result<_>>()?,
        hidden2_biases: quantized_array(
            &model.parameters()[Parameter::Hidden2Biases],
            127.0 * 256.0,
            260_096,
            Parameter::Hidden2Biases,
        )?,
        hidden2_output: quantized_array(
            &model.parameters()[Parameter::Hidden2Output],
            4096.0,
            8192,
            Parameter::Hidden2Output,
        )?
        .map(|x| x as i16),
    })
}

fn quantized_array<const N: usize>(
    values: &[f32],
    scale: f64,
    limit: i32,
    p: Parameter,
) -> Result<[i32; N]> {
    values
        .iter()
        .map(|&v| rounded(v, scale, -limit, limit, p))
        .collect::<Result<Vec<_>>>()?
        .try_into()
        .map_err(|_| Error::Invalid("quantized array shape"))
}

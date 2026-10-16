use super::{Error, Model, Parameter, Result, Shape};
use crate::nnue::data::InputFeatureSet;
use minase::eval::{
    features::PIECE_STATE_COUNT,
    nnue::{
        self, EncodeInput,
        feature_set::{HalfKpHmB8, P},
        head::DirectScrelu,
    },
};

/// 尺度と上限を検査済みの、MNUE版5に渡す整数配列。埋め草は含まない。
#[derive(Debug)]
pub struct Quantized {
    shape: Shape,
    /// 線形重み`[13680, 2]`。尺度8。
    pub linear: Vec<[i16; 2]>,
    /// 展開・畳み込み済みの第1層`[D, H]`。尺度127。
    pub weights: Vec<i16>,
    /// バイアス`[H]`。尺度127。
    pub biases: Vec<i32>,
    /// 出力重み`[2, H]`。尺度4096。
    pub output_weights: Vec<i16>,
    /// 出力バイアス。尺度127×4096。
    pub output_bias: i32,
}
impl Quantized {
    /// 推論側のEncodeInputを使い、MNUE版5の全検査を経てバイト列を作る。
    pub fn encode(
        &self,
        piece_values: &[i32; PIECE_STATE_COUNT],
        rule_set: &str,
    ) -> Result<Vec<u8>> {
        let head = DirectScrelu::new(
            self.shape.width(),
            &self.output_weights,
            self.output_bias,
            self.shape.feature_set() == InputFeatureSet::P,
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
/// 線形は×8でi16、第1層とバイアスは×127で±1,016、出力重みは×4,096で±8,192、出力バイアスは×127×4,096で±4,161,536に収める。
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
        .map(|&v| rounded(v, 127.0, -1016, 1016, Parameter::FirstLayer).map(|x| x as i16))
        .collect::<Result<Vec<_>>>()?;
    let biases = model
        .expanded_biases()
        .iter()
        .map(|&v| rounded(v, 127.0, -1016, 1016, Parameter::Biases))
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
    })
}

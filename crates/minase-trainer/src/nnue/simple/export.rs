use super::reference::forward_validated;
use super::{Error, Features, Parameter as P, Parameters, Result, SPEC, Scale};
use minase::eval::nnue::network::{SimpleNetwork, SimpleParameters};
use minase_core::{Color, Position};
fn quantized(
    name: &'static str,
    index: usize,
    value: f64,
    scale: f64,
    min: f64,
    max: f64,
) -> Result<f64> {
    let rounded = (value * scale).round();
    if !rounded.is_finite() || !(min..=max).contains(&rounded) {
        return Err(Error::Quantization {
            name,
            index,
            value: rounded,
        });
    }
    Ok(rounded)
}
/// 実の行と仮想行をf32で畳み込み、1回だけ量子化する。範囲外は切り詰めず拒否する。
pub fn quantize(w: &Parameters, scale: Scale) -> Result<SimpleParameters> {
    w.validate()?;
    let h = w.width();
    let i16q = |name, i, v| -> Result<i16> {
        Ok(quantized(name, i, v, 127.0, f64::from(i16::MIN), f64::from(i16::MAX))? as i16)
    };
    let i8q = |p: P| -> Result<Vec<i8>> {
        w[p].iter()
            .enumerate()
            .map(|(i, &v)| {
                Ok(quantized(
                    p.name(),
                    i,
                    f64::from(v),
                    64.0,
                    f64::from(i8::MIN),
                    f64::from(i8::MAX),
                )? as i8)
            })
            .collect()
    };
    let i32q = |p: P| -> Result<Vec<i32>> {
        w[p].iter()
            .enumerate()
            .map(|(i, &v)| {
                Ok(quantized(
                    p.name(),
                    i,
                    f64::from(v),
                    8128.0,
                    f64::from(i32::MIN),
                    f64::from(i32::MAX),
                )? as i32)
            })
            .collect()
    };
    Ok(SimpleParameters {
        width: h,
        fv_scale: scale.divisor(),
        ft_biases: w[P::FtBiases]
            .iter()
            .enumerate()
            .map(|(j, &v)| i16q("ft_biases", j, f64::from(v)))
            .collect::<Result<_>>()?,
        ft_weights: w[P::FtWeights]
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                i16q(
                    "ft_weights+ft_virtual",
                    i,
                    f64::from(v + w[P::FtVirtual][i % (SPEC.piece_inputs() * h)]),
                )
            })
            .collect::<Result<_>>()?,
        l1_weights: i8q(P::L1Weights)?,
        l1_biases: i32q(P::L1Biases)?,
        l2_weights: i8q(P::L2Weights)?,
        l2_biases: i32q(P::L2Biases)?,
        output_weights: i8q(P::L3Weights)?,
        output_bias: i32q(P::L3Bias)?[0],
    })
}
/// エンジンで累算値と後段の値域を検査してから符号化する。
pub fn export(w: &Parameters, scale: Scale) -> Result<Vec<u8>> {
    Ok(SimpleNetwork::new(quantize(w, scale)?)?.encode())
}
/// 標本で測った量子化誤差と、実数計算における活性化の飽和率。合否閾値を持たない。
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct QuantizationReport {
    /// 局面と視点の組の数。
    pub samples: usize,
    /// 実数評価と整数評価の平均絶対差、センチポーン。
    pub mean_absolute_cp: f64,
    /// 整数評価から実数評価を引いた差の平均、センチポーン。
    pub mean_signed_cp: f64,
    /// 絶対差の最大値、センチポーン。
    pub max_absolute_cp: f64,
    /// 第1層の2H成分のうち、0または1へ張り付いた割合。
    pub ft_saturation: f64,
    /// L1の32成分の飽和率。
    pub l1_saturation: f64,
    /// L2の出力32成分の飽和率。
    pub l2_saturation: f64,
}
/// 指定した標本で、書き出したネットのエンジン評価とK_o Yを比較する。
/// 飽和は活性化前の値が0以下または1以上の成分として数える。
pub fn quantization_report(
    w: &Parameters,
    scale: Scale,
    samples: &[(&Position, Color)],
) -> Result<QuantizationReport> {
    if samples.is_empty() {
        return Err(Error::Invalid("empty quantization sample"));
    }
    let network = SimpleNetwork::new(quantize(w, scale)?)?;
    let mut report = QuantizationReport {
        samples: samples.len(),
        mean_absolute_cp: 0.0,
        mean_signed_cp: 0.0,
        max_absolute_cp: 0.0,
        ft_saturation: 0.0,
        l1_saturation: 0.0,
        l2_saturation: 0.0,
    };
    let saturated = |x: f64| f64::from(x <= 0.0 || x >= 1.0);
    for &(position, side) in samples {
        let f = forward_validated(w, &Features::from_position(position, side)?);
        let integer = network.evaluate_for(position, side)?;
        let diff = f64::from(integer) - scale.output() * f.output;
        report.mean_absolute_cp += diff.abs();
        report.mean_signed_cp += diff;
        report.max_absolute_cp = report.max_absolute_cp.max(diff.abs());
        report.ft_saturation += f.a.iter().flatten().copied().map(saturated).sum::<f64>();
        report.l1_saturation += f.y.iter().copied().map(saturated).sum::<f64>();
        report.l2_saturation += f.u.iter().copied().map(saturated).sum::<f64>();
    }
    let count = samples.len() as f64;
    report.mean_absolute_cp /= count;
    report.mean_signed_cp /= count;
    report.ft_saturation /= count * (2 * w.width()) as f64;
    report.l1_saturation /= count * 32.0;
    report.l2_saturation /= count * 32.0;
    Ok(report)
}

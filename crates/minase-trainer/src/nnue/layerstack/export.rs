use super::model::{ALPHA, HALF, canonical, unit};
use super::reference::forward_validated;
use super::{Error, Features, H, Parameter as P, Parameters, Result, Scale};
use minase::eval::{
    nnue::input::FeatureSet,
    nnue::{LayerStackNetwork, LayerStackParameters, Network},
};
use minase_core::{Color, Position};

const FEATURE_COUNT: usize = FeatureSet::A.spec().dimensions();

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
/// 自由パラメータを展開し、共有L1は足してから1回だけ丸める。範囲外の値は拒否する。
/// 型への変換だけを担当し、累算値や鏡映の検査はexportでエンジンに委ねる。
pub fn quantize(w: &Parameters, scale: Scale) -> Result<LayerStackParameters> {
    w.validate()?;
    let i16q = |name, i, v| -> Result<i16> {
        Ok(quantized(name, i, v, 127.0, f64::from(i16::MIN), f64::from(i16::MAX))? as i16)
    };
    let i8q = |name, i, v| -> Result<i8> {
        Ok(quantized(name, i, v, 64.0, f64::from(i8::MIN), f64::from(i8::MAX))? as i8)
    };
    let i32q = |name, i, v| -> Result<i32> {
        Ok(quantized(name, i, v, 8128.0, f64::from(i32::MIN), f64::from(i32::MAX))? as i32)
    };
    let ft_biases = (0..H)
        .map(|j| i16q("ft_biases", j, f64::from(w[P::FtBiases][j % HALF])))
        .collect::<Result<_>>()?;
    let ft_weights = (0..FEATURE_COUNT * H)
        .map(|i| {
            let (f, j) = (i / H, i % H);
            i16q(
                "ft_weights",
                i,
                f64::from(w[P::FtWeights][canonical(f) * H + unit(f, j)]),
            )
        })
        .collect::<Result<_>>()?;
    let psqt_weights = (0..FEATURE_COUNT)
        .map(|f| i32q("psqt", f, f64::from(w[P::Psqt][canonical(f)])))
        .collect::<Result<_>>()?;
    let l1_weights = (0..16 * H)
        .map(|i| {
            i8q(
                "l1_weights+l1_shared_weights",
                i,
                f64::from(w[P::L1Weights][i]) + f64::from(w[P::L1SharedWeights][i]),
            )
        })
        .collect::<Result<_>>()?;
    let l1_biases = (0..16)
        .map(|i| {
            i32q(
                "l1_biases+l1_shared_biases",
                i,
                f64::from(w[P::L1Biases][i]) + f64::from(w[P::L1SharedBiases][i]),
            )
        })
        .collect::<Result<_>>()?;
    let dense_weights = |p: P| -> Result<Vec<i8>> {
        w[p].iter()
            .enumerate()
            .map(|(i, &v)| i8q(p.name(), i, f64::from(v)))
            .collect()
    };
    let dense_biases = |p: P| -> Result<Vec<i32>> {
        w[p].iter()
            .enumerate()
            .map(|(i, &v)| i32q(p.name(), i, f64::from(v)))
            .collect()
    };
    Ok(LayerStackParameters {
        width: H,
        fv_scale: scale.divisor(),
        ft_biases,
        ft_weights,
        psqt_bias: 0,
        psqt_weights,
        l1_weights,
        l1_biases,
        l2_weights: dense_weights(P::L2Weights)?,
        l2_biases: dense_biases(P::L2Biases)?,
        output_weights: dense_weights(P::L3Weights)?,
        output_bias: i32q("l3_bias", 0, f64::from(w[P::L3Bias][0]))?,
    })
}
/// エンジンの全検査を通してからtatara形式で符号化する。
pub fn export(w: &Parameters, scale: Scale) -> Result<Vec<u8>> {
    Ok(LayerStackNetwork::new(quantize(w, scale)?)?.encode())
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
    /// 第1層の積へ入る2×256成分のうち、0または1へ張り付いた割合。
    pub ft_saturation: f64,
    /// L1の2乗の枝の15成分の飽和率。
    pub l1_square_saturation: f64,
    /// L1のそのままの枝の15成分の飽和率。
    pub l1_linear_saturation: f64,
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
    let network = Network::decode(&export(w, scale)?)?;
    let mut report = QuantizationReport {
        samples: samples.len(),
        mean_absolute_cp: 0.0,
        mean_signed_cp: 0.0,
        max_absolute_cp: 0.0,
        ft_saturation: 0.0,
        l1_square_saturation: 0.0,
        l1_linear_saturation: 0.0,
        l2_saturation: 0.0,
    };
    let saturated = |x: f64| f64::from(x <= 0.0 || x >= 1.0);
    for &(position, side) in samples {
        let f = forward_validated(w, &Features::from_position(position, side));
        let integer = network
            .evaluate_for(position, side)
            .map_err(|_| Error::Invalid("A network evaluation"))?;
        let diff = f64::from(integer) - scale.output() * f.output;
        report.mean_absolute_cp += diff.abs();
        report.mean_signed_cp += diff;
        report.max_absolute_cp = report.max_absolute_cp.max(diff.abs());
        report.ft_saturation += f.a.iter().flatten().copied().map(saturated).sum::<f64>();
        report.l1_square_saturation += f.y[..15]
            .iter()
            .map(|&x| saturated(ALPHA * x * x))
            .sum::<f64>();
        report.l1_linear_saturation += f.y[..15].iter().copied().map(saturated).sum::<f64>();
        report.l2_saturation += f.u.iter().copied().map(saturated).sum::<f64>();
    }
    let count = samples.len() as f64;
    report.mean_absolute_cp /= count;
    report.mean_signed_cp /= count;
    report.ft_saturation /= count * (2 * H) as f64;
    report.l1_square_saturation /= count * 15.0;
    report.l1_linear_saturation /= count * 15.0;
    report.l2_saturation /= count * 32.0;
    Ok(report)
}

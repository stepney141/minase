use crate::nnue::model::{Model, Parameter};

/// 直接経路と隠れ層の前進結果。zとhは隠れ層がない場合は空。
pub(crate) struct HeadOutput {
    pub value: f32,
    pub z: Vec<f32>,
    pub h: Vec<f32>,
}

/// 各視点の内積を単位順にf32で加算し、2視点の和を返す。
/// P型は各視点のjとj+Uの活性化を先に足す。
fn dot(model: &Model, weights: &[f32], s: &[f32]) -> f32 {
    let shape = model.shape();
    let mut views = [0.0; 2];
    for v in 0..2 {
        for j in 0..shape.units() {
            let i = v * shape.width() + j;
            let activation = if shape.units() == shape.width() {
                s[i]
            } else {
                s[i] + s[i + shape.units()]
            };
            views[v] += activation * weights[v * shape.units() + j];
        }
    }
    views[0] + views[1]
}

/// SCReLU済みの2H成分から出力部を計算する。phiは序盤端点の係数。
/// D0=o·s+b、D2=phi×D0_open+(1−phi)×D0_end。
/// S1/S2は各端点へw2·clamp(W1s+b1,0,1)を加える。
/// W1の各行は手番側から相手側へ自由成分順に、w2は0〜15の順にf32で足す。
/// 各端点は(直接内積+w2内積)+バイアスの順とし、最後に位相で補間する。
pub(crate) fn head(model: &Model, s: &[f32], phi: f32) -> HeadOutput {
    let shape = model.shape();
    let p = model.parameters();
    let folded: Vec<_> = (0..2 * shape.units())
        .map(|i| {
            let j = i / shape.units() * shape.width() + i % shape.units();
            if shape.units() == shape.width() {
                s[j]
            } else {
                s[j] + s[j + shape.units()]
            }
        })
        .collect();
    let z: Vec<_> = p[Parameter::HiddenWeights]
        .chunks_exact(2 * shape.units())
        .zip(&p[Parameter::HiddenBiases])
        .map(|(w, &b)| w.iter().zip(&folded).fold(0.0, |sum, (&w, &x)| sum + w * x) + b)
        .collect();
    let h: Vec<_> = z.iter().map(|&x| x.clamp(0.0, 1.0)).collect();
    let endpoint = |o, w, b| {
        let mut hidden = 0.0;
        for (&activation, &weight) in h.iter().zip(&p[w]) {
            hidden += activation * weight;
        }
        dot(model, &p[o], s) + hidden + p[b][0]
    };
    let open = endpoint(
        Parameter::OutputWeights,
        Parameter::HiddenOutput,
        Parameter::OutputBias,
    );
    let value = if shape.head().phase2() {
        let end = endpoint(
            Parameter::OutputWeightsEnd,
            Parameter::HiddenOutputEnd,
            Parameter::OutputBiasEnd,
        );
        phi * open + (1.0 - phi) * end
    } else {
        open
    };
    HeadOutput { value, z, h }
}

/// 量子化検査と駒除去診断に共通の出力部。s、clamp、位相補間をf64で計算する。
pub(crate) fn head_f64(model: &Model, s: &[f64], phi: f64) -> f64 {
    let shape = model.shape();
    let p = model.parameters();
    let dot = |weights: &[f32]| {
        s.iter()
            .enumerate()
            .map(|(i, &x)| {
                x * f64::from(
                    weights[i / shape.width() * shape.units() + i % shape.width() % shape.units()],
                )
            })
            .sum::<f64>()
    };
    let h: Vec<_> = p[Parameter::HiddenWeights]
        .chunks_exact(2 * shape.units())
        .zip(&p[Parameter::HiddenBiases])
        .map(|(w, &b)| (dot(w) + f64::from(b)).clamp(0.0, 1.0))
        .collect();
    let endpoint = |o, w, b| {
        dot(&p[o])
            + h.iter()
                .zip(&p[w])
                .map(|(&x, &weight)| x * f64::from(weight))
                .sum::<f64>()
            + f64::from(p[b][0])
    };
    let open = endpoint(
        Parameter::OutputWeights,
        Parameter::HiddenOutput,
        Parameter::OutputBias,
    );
    if shape.head().phase2() {
        phi * open
            + (1.0 - phi)
                * endpoint(
                    Parameter::OutputWeightsEnd,
                    Parameter::HiddenOutputEnd,
                    Parameter::OutputBiasEnd,
                )
    } else {
        open
    }
}

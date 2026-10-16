use crate::nnue::model::{Model, Parameter};

/// 直接経路と隠れ層の前進結果。zとhは隠れ層がない場合は空。
pub(crate) struct HeadOutput {
    pub value: f32,
    pub z: Vec<f32>,
    pub h: Vec<f32>,
}

/// 32レーンの各レーンが添字lane, lane+32, ...をf32で足し、
/// 距離16、8、4、2、1の固定木で縮約する。末尾の空レーンは0とする。
/// CUDAのhead_warp_sumと同じ順序で、積和は融合しない。
fn warp_dot(weights: &[f32], activations: &[f32]) -> f32 {
    let mut lanes = [0.0_f32; 32];
    for (i, (&weight, &activation)) in weights.iter().zip(activations).enumerate() {
        lanes[i % 32] += weight * activation;
    }
    for distance in [16, 8, 4, 2, 1] {
        for lane in 0..distance {
            lanes[lane] += lanes[lane + distance];
        }
    }
    lanes[0]
}

/// SCReLU済みの2H成分から出力部を計算する。phiは序盤端点の係数。
/// D0=o·s+b、D2=phi×D0_open+(1−phi)×D0_end。
/// S1/S2は各端点へw2·clamp(W1s+b1,0,1)を加える。
/// 直接内積は各視点をwarp_dotで縮約して足す。W1は2視点を連結してwarp_dotで縮約する。
/// w2は0〜15の順にf32で足す。
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
        .map(|(w, &b)| warp_dot(w, &folded) + b)
        .collect();
    let h: Vec<_> = z.iter().map(|&x| x.clamp(0.0, 1.0)).collect();
    let endpoint = |o, w, b| {
        let mut hidden = 0.0;
        for (&activation, &weight) in h.iter().zip(&p[w]) {
            hidden += activation * weight;
        }
        (warp_dot(&p[o][..shape.units()], &folded[..shape.units()])
            + warp_dot(&p[o][shape.units()..], &folded[shape.units()..]))
            + hidden
            + p[b][0]
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

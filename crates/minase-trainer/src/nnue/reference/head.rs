use crate::nnue::model::{Model, Parameter};

/// 要素積を共有に従って畳み込んだ入力と、2段の隠れ層の前進結果。
pub(crate) struct HeadOutput {
    pub value: f32,
    pub f: Vec<f32>,
    pub z: [f32; 15],
    pub u: [f32; 30],
    pub a: [f32; 32],
    pub y: [f32; 32],
}

/// 32レーンが添字lane, lane+32, ...をf32で足し、距離16、8、4、2、1で縮約する。
/// CUDAのhead_warp_sumと同じ固定木を使い、積和は融合しない。
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

/// 1視点内の要素積iに対応する、第1層の2因子の添字。
pub(crate) fn factors(i: usize, width: usize) -> (usize, usize) {
    let q = width / 4;
    let left = i / q * 2 * q + i % q;
    (left, left + q)
}

/// 2視点の累算値から出力を計算する。P型の要素積jとj+H/4を先に足す。
/// W1は全入力をwarp_dotで縮約し、W2は列0〜29を昇順にf32で足してからバイアスを足す。
/// 直接内積は視点ごとに縮約し、vの内積は0〜31の順で足す。最後は(直接内積+v内積)+b。
pub(crate) fn head(model: &Model, acc: &[f32]) -> HeadOutput {
    let shape = model.shape();
    let h = shape.width();
    let units = shape.output_units();
    let p = model.parameters();
    let pair = |view: usize, i: usize| {
        let (left, right) = factors(i, h);
        acc[view * h + left].clamp(0.0, 1.0) * acc[view * h + right].clamp(0.0, 1.0)
    };
    let f: Vec<_> = (0..2 * units)
        .map(|i| {
            let value = pair(i / units, i % units);
            if units == h / 2 {
                value
            } else {
                value + pair(i / units, i % units + units)
            }
        })
        .collect();
    let z = std::array::from_fn(|k| {
        warp_dot(
            &p[Parameter::Hidden1Weights][k * 2 * units..(k + 1) * 2 * units],
            &f,
        ) + p[Parameter::Hidden1Biases][k]
    });
    let u = std::array::from_fn(|i| {
        if i < 15 {
            ((127.0 / 128.0 * z[i]) * z[i]).min(1.0)
        } else {
            z[i - 15].clamp(0.0, 1.0)
        }
    });
    let a = std::array::from_fn(|k| {
        let mut sum = 0.0;
        for (i, &value) in u.iter().enumerate() {
            sum += p[Parameter::Hidden2Weights][k * 30 + i] * value;
        }
        sum + p[Parameter::Hidden2Biases][k]
    });
    let y = a.map(|x| x.clamp(0.0, 1.0));
    let mut hidden = 0.0;
    for (k, &value) in y.iter().enumerate() {
        hidden += p[Parameter::Hidden2Output][k] * value;
    }
    let direct = warp_dot(&p[Parameter::OutputWeights][..units], &f[..units])
        + warp_dot(&p[Parameter::OutputWeights][units..], &f[units..]);
    HeadOutput {
        value: (direct + hidden) + p[Parameter::OutputBias][0],
        f,
        z,
        u,
        a,
        y,
    }
}

/// 量子化検査と駒除去診断用。要素積と2段の隠れ層を実数の式どおりf64で計算する。
/// 整数推論の丸めは使わず、2乗の枝の係数127/128は含める。
pub(crate) fn head_f64(model: &Model, acc: &[f64]) -> f64 {
    let shape = model.shape();
    let h = shape.width();
    let units = shape.output_units();
    let p = model.parameters();
    let s: Vec<_> = (0..h)
        .map(|i| {
            let (left, right) = factors(i % (h / 2), h);
            let view = i / (h / 2) * h;
            acc[view + left].clamp(0.0, 1.0) * acc[view + right].clamp(0.0, 1.0)
        })
        .collect();
    let dot = |weights: &[f32]| {
        s.iter()
            .enumerate()
            .map(|(i, &value)| {
                value * f64::from(weights[i / (h / 2) * units + i % (h / 2) % units])
            })
            .sum::<f64>()
    };
    let z: [f64; 15] = std::array::from_fn(|k| {
        dot(&p[Parameter::Hidden1Weights][k * 2 * units..(k + 1) * 2 * units])
            + f64::from(p[Parameter::Hidden1Biases][k])
    });
    let u: [f64; 30] = std::array::from_fn(|i| {
        if i < 15 {
            (127.0 / 128.0 * z[i] * z[i]).min(1.0)
        } else {
            z[i - 15].clamp(0.0, 1.0)
        }
    });
    let mut hidden = 0.0;
    for k in 0..32 {
        let a = u
            .iter()
            .enumerate()
            .map(|(i, &value)| f64::from(p[Parameter::Hidden2Weights][k * 30 + i]) * value)
            .sum::<f64>()
            + f64::from(p[Parameter::Hidden2Biases][k]);
        hidden += f64::from(p[Parameter::Hidden2Output][k]) * a.clamp(0.0, 1.0);
    }
    (dot(&p[Parameter::OutputWeights]) + hidden) + f64::from(p[Parameter::OutputBias][0])
}

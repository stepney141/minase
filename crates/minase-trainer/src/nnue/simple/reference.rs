use super::{Error, Parameter as P, Parameters, Result, SPEC, Scale};
use crate::nnue::data::{BatchSlot, FEATURE_COLUMNS, SENTINEL};
use minase::eval::nnue::input::{FeatureSet, HalfKpVm, Indexer, for_each_active_row};
use minase_core::{Color, Position};

/// 手番側、相手側の順に並べた実の行番号。仮想行番号は行番号mod基底次元である。
#[derive(Clone, Debug)]
pub struct Features {
    views: [Vec<usize>; 2],
}
impl Features {
    /// 各視点の発火数と行番号をエンジンの仕様で検査する。
    pub fn new(views: [Vec<usize>; 2]) -> Result<Self> {
        if views
            .iter()
            .any(|v| v.len() > SPEC.max_active() || v.iter().any(|&r| r >= SPEC.dimensions()))
        {
            return Err(Error::Invalid("HalfKP_vm features"));
        }
        Ok(Self { views })
    }
    /// 両視点の文脈を検査し、エンジンの番号付けで行を列挙する。
    pub fn from_position(position: &Position, side: Color) -> Result<Self> {
        let mut views = [Vec::new(), Vec::new()];
        for (v, side) in [side, side.opposite()].into_iter().enumerate() {
            let context = HalfKpVm::context(position, side)?;
            for_each_active_row::<HalfKpVm>(context, side, position, |r| views[v].push(r));
        }
        Self::new(views)
    }
    /// HalfKP_vmのバッチを検査し、番兵を除く。
    pub fn from_batch(batch: &BatchSlot) -> Result<Vec<Self>> {
        if batch.feature_set() != FeatureSet::HalfKpVm {
            return Err(Error::Invalid("Simple requires HalfKP_vm input"));
        }
        batch
            .features()
            .as_chunks::<{ 2 * FEATURE_COLUMNS }>()
            .0
            .iter()
            .map(|rows| {
                let view = |xs: &[u32]| {
                    xs.iter()
                        .filter(|&&r| r != SENTINEL)
                        .map(|&r| r as usize)
                        .collect()
                };
                Self::new([
                    view(&rows[..FEATURE_COLUMNS]),
                    view(&rows[FEATURE_COLUMNS..]),
                ])
            })
            .collect()
    }
    /// 両視点の行番号を返す。
    pub fn views(&self) -> &[Vec<usize>; 2] {
        &self.views
    }
}
/// 単精度で計算した中間値を、比較と統計のため倍精度で保持する。
#[derive(Clone, Debug)]
pub struct Forward {
    /// 両視点の活性化前の累算値。
    pub a: [Vec<f64>; 2],
    /// C(a_us)とC(a_them)の連結。
    pub x: Vec<f64>,
    /// L1の活性化前の値。
    pub y: [f64; 32],
    /// L1の活性化後の値。
    pub h1: [f64; 32],
    /// L2の活性化前の値。
    pub u: [f64; 32],
    /// L2の活性化後の値。
    pub h2: [f64; 32],
    /// 出力Y。
    pub output: f64,
}
fn c(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
pub(super) fn derivative(x: f32) -> f32 {
    if x > 0.0 && x < 1.0 { 1.0 } else { 0.0 }
}
fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}
/// CUDA実装も距離16,8,4,2,1の木と各laneの昇順の積和を使う。
fn sum32(mut lanes: [f32; 32]) -> f32 {
    for distance in [16, 8, 4, 2, 1] {
        for lane in 0..distance {
            lanes[lane] += lanes[lane + distance];
        }
    }
    lanes[0]
}
fn dot(w: &[f32], x: &[f32]) -> f32 {
    sum32(std::array::from_fn(|lane| {
        (lane..x.len())
            .step_by(32)
            .fold(0.0, |s, j| s + w[j] * x[j])
    }))
}
/// パラメータを検査して単精度の順伝播を行う。
pub fn forward(w: &Parameters, features: &Features) -> Result<Forward> {
    w.validate()?;
    Ok(forward_validated(w, features))
}
pub(super) fn forward_validated(w: &Parameters, features: &Features) -> Forward {
    let h = w.width();
    let mut a = [w[P::FtBiases].to_vec(), w[P::FtBiases].to_vec()];
    for (v, rows) in features.views.iter().enumerate() {
        for &r in rows {
            for (j, value) in a[v].iter_mut().enumerate() {
                *value +=
                    w[P::FtWeights][r * h + j] + w[P::FtVirtual][(r % SPEC.piece_inputs()) * h + j];
            }
        }
    }
    let x: Vec<_> = a.iter().flatten().copied().map(c).collect();
    let y = std::array::from_fn(|k| {
        w[P::L1Biases][k] + dot(&w[P::L1Weights][k * 2 * h..(k + 1) * 2 * h], &x)
    });
    let h1 = y.map(c);
    let u = std::array::from_fn(|k| {
        w[P::L2Biases][k] + dot(&w[P::L2Weights][k * 32..(k + 1) * 32], &h1)
    });
    let h2 = u.map(c);
    let output = w[P::L3Bias][0] + dot(&w[P::L3Weights], &h2);
    Forward {
        a: a.map(|v| v.into_iter().map(f64::from).collect()),
        x: x.into_iter().map(f64::from).collect(),
        y: y.map(f64::from),
        h1: h1.map(f64::from),
        u: u.map(f64::from),
        h2: h2.map(f64::from),
        output: f64::from(output),
    }
}
fn check_batch(features: &[Features], targets: &[f32]) -> Result<()> {
    if features.is_empty()
        || features.len() != targets.len()
        || targets
            .iter()
            .any(|r| !r.is_finite() || !(0.0..=1.0).contains(r))
    {
        return Err(Error::Invalid("empty batch, target length or target range"));
    }
    Ok(())
}
/// K_o/K_tを掛けた出力のsigmoidと教師勝率の平均二乗誤差。
pub fn loss(w: &Parameters, scale: Scale, features: &[Features], targets: &[f32]) -> Result<f64> {
    w.validate()?;
    check_batch(features, targets)?;
    Ok(features
        .iter()
        .zip(targets)
        .map(|(f, &t)| {
            (sigmoid(scale.ratio() * forward_validated(w, f).output) - f64::from(t)).powi(2)
        })
        .sum::<f64>()
        / targets.len() as f64)
}
/// 各局面の積はf32、局面方向の和はf64。活性化の両境界の微分は0。
/// WとVには同じ出現のδを加算する。後段の勾配は局面数倍の配列を持たない。
pub fn backward(
    w: &Parameters,
    scale: Scale,
    features: &[Features],
    targets: &[f32],
) -> Result<(f64, Parameters)> {
    w.validate()?;
    check_batch(features, targets)?;
    let h = w.width();
    let mut gradients: [Vec<f64>; 9] = P::ALL.map(|p| vec![0.0; p.count(h)]);
    let mut total = 0.0;
    for (features, &target) in features.iter().zip(targets) {
        let f = forward_validated(w, features);
        let sigma = sigmoid(scale.ratio() * f.output);
        total += (sigma - f64::from(target)).powi(2);
        let gy = (2.0 / targets.len() as f64
            * (sigma - f64::from(target))
            * sigma
            * (1.0 - sigma)
            * scale.ratio()) as f32;
        let mut add = |p: P, i: usize, x: f32| gradients[p as usize][i] += f64::from(x);
        add(P::L3Bias, 0, gy);
        let du: [f32; 32] =
            std::array::from_fn(|k| gy * w[P::L3Weights][k] * derivative(f.u[k] as f32));
        for (k, &du) in du.iter().enumerate() {
            add(P::L3Weights, k, gy * f.h2[k] as f32);
            add(P::L2Biases, k, du);
            for j in 0..32 {
                add(P::L2Weights, k * 32 + j, du * f.h1[j] as f32);
            }
        }
        let dy: [f32; 32] = std::array::from_fn(|j| {
            sum32(std::array::from_fn(|k| du[k] * w[P::L2Weights][k * 32 + j]))
                * derivative(f.y[j] as f32)
        });
        let mut dx = vec![0.0_f32; 2 * h];
        // L1の逆伝播はLayerStackと同じく出力kの昇順で足す。
        for (k, &dy) in dy.iter().enumerate() {
            add(P::L1Biases, k, dy);
            for (j, dx) in dx.iter_mut().enumerate() {
                add(P::L1Weights, k * 2 * h + j, dy * f.x[j] as f32);
                *dx += dy * w[P::L1Weights][k * 2 * h + j];
            }
        }
        for v in 0..2 {
            for j in 0..h {
                let da = dx[v * h + j] * derivative(f.a[v][j] as f32);
                add(P::FtBiases, j, da);
                for &r in &features.views[v] {
                    add(P::FtWeights, r * h + j, da);
                    add(P::FtVirtual, (r % SPEC.piece_inputs()) * h + j, da);
                }
            }
        }
    }
    let mut result = Parameters::zeros(h)?;
    for p in P::ALL {
        for (dst, &g) in result[p].iter_mut().zip(&gradients[p as usize]) {
            *dst = g as f32;
        }
    }
    result.validate()?;
    Ok((total / targets.len() as f64, result))
}
/// 共通の教師混合式で教師を作り、1バッチ更新する。
pub fn train_batch(
    w: &mut Parameters,
    optimizer: &mut super::AdamW,
    scale: Scale,
    batch: &BatchSlot,
    lambda: f64,
    multiplier: f64,
) -> Result<f64> {
    let features = Features::from_batch(batch)?;
    let targets = crate::data::build_targets_from_arrays(
        batch.teacher_scores(),
        &vec![0; batch.len()],
        batch.outcomes(),
        &[scale.teacher()],
        &[lambda],
    )
    .map_err(crate::nnue::data::Error::from)?;
    let (loss, gradients) = backward(w, scale, &features, &targets)?;
    optimizer.update(w, &gradients, multiplier)?;
    Ok(loss)
}

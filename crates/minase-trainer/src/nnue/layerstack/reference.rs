use super::model::{HALF, canonical, unit};

use super::{Error, H, Parameter as P, Parameters, Result, Scale};
use crate::nnue::data::{BatchSlot, InputFeatureSet, SENTINEL, decode_p_feature};
use minase::eval::features::{FEATURE_COUNT, active_features_for};
use minase_core::{Color, Position};

const ALPHA: f32 = 127.0 / 128.0;

/// 手番側、相手側の順に並べた、検査済みのP型基底特徴。
#[derive(Clone, Debug)]
pub struct Features {
    views: [Vec<usize>; 2],
}
impl Features {
    /// 各視点145特徴以下、各番号13680未満を要求する。重複は和に含める。
    pub fn new(views: [Vec<usize>; 2]) -> Result<Self> {
        if views
            .iter()
            .any(|v| v.len() > 145 || v.iter().any(|&f| f >= FEATURE_COUNT))
        {
            return Err(Error::Invalid("P features"));
        }
        Ok(Self { views })
    }
    /// 指定した陣営を手番側として盤面と先獅子から特徴を構築する。
    pub fn from_position(position: &Position, side: Color) -> Self {
        Self {
            views: [side, side.opposite()].map(|view| {
                let mut rows = Vec::new();
                active_features_for(view, position, |f| rows.push(f));
                rows
            }),
        }
    }
    /// 既存のP型バッチの圧縮番号を基底番号へ戻す。
    pub fn from_batch(batch: &BatchSlot) -> Result<Vec<Self>> {
        if batch.feature_set() != InputFeatureSet::P {
            return Err(Error::Invalid("LayerStack requires P input"));
        }
        let (rows, remainder) = batch.features().as_chunks::<290>();
        if !remainder.is_empty() || rows.len() != batch.len() {
            return Err(Error::Invalid("P batch shape"));
        }
        rows.iter()
            .map(|rows| {
                let view = |slice: &[u32]| -> Result<Vec<usize>> {
                    slice
                        .iter()
                        .filter(|&&f| f != SENTINEL)
                        .map(|&f| Ok(decode_p_feature(f)? as usize))
                        .collect()
                };
                Self::new([view(&rows[..145])?, view(&rows[145..])?])
            })
            .collect()
    }
    /// 両視点の特徴番号を返す。
    pub fn views(&self) -> &[Vec<usize>; 2] {
        &self.views
    }
}

/// GPUと同じ順序で単精度計算した1局面の順伝播。
/// 統計と誤差比較に使うため、結果を丸めずf64へ拡張して格納する。
#[derive(Clone, Debug)]
pub struct Forward {
    /// 活性化前の第1層の累算値。
    pub a: [[f64; H]; 2],
    /// 両視点の半幅要素積を連結した値。
    pub p: [f64; H],
    /// 共有項を足した第1後段層。
    pub y: [f64; 16],
    /// 2乗の枝、そのままの枝の順に連結した値。
    pub z: [f64; 30],
    /// 活性化前の第2後段層。
    pub u: [f64; 32],
    /// 出力層とy[15]の和。
    pub n: f64,
    /// 両視点のPSQTの和。
    pub psqt: [f64; 2],
    /// PSQT差の半分を加えた最終出力Y。
    pub output: f64,
}
fn c(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}
pub(super) fn crelu_grad(x: f32) -> f32 {
    if x > 0.0 && x < 1.0 { 1.0 } else { 0.0 }
}
pub(super) fn square_grad(x: f32) -> f32 {
    2.0 * ALPHA * x * crelu_grad(ALPHA * x * x)
}
fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// 有限なパラメータを検査して、GPUと同じ単精度の演算順で1局面を計算する。
pub fn forward(params: &Parameters, features: &Features) -> Result<Forward> {
    params.validate()?;
    Ok(forward_validated(params, features))
}
// CUDAのwarp_sumと同じ距離16、8、4、2、1の木。
fn sum32(mut lanes: [f32; 32]) -> f32 {
    for distance in [16, 8, 4, 2, 1] {
        for lane in 0..distance {
            lanes[lane] += lanes[lane + distance];
        }
    }
    lanes[0]
}

pub(super) fn forward_validated(w: &Parameters, features: &Features) -> Forward {
    let mut a = [[0.0_f32; H]; 2];
    let mut psqt = [0.0_f32; 2];
    for (v, rows) in features.views.iter().enumerate() {
        for (j, value) in a[v].iter_mut().enumerate() {
            *value = w[P::FtBiases][j % HALF];
        }
        let (mut sum, mut correction) = (0.0_f32, 0.0_f32);
        for &f in rows {
            let row = canonical(f);
            // ls_forwardと同じ補償和。主和、補正とも単精度で保持する。
            let x = w[P::Psqt][row];
            let next = sum + x;
            correction += if sum.abs() >= x.abs() {
                (sum - next) + x
            } else {
                (x - next) + sum
            };
            sum = next;
            for (j, value) in a[v].iter_mut().enumerate() {
                *value += w[P::FtWeights][row * H + unit(f, j)];
            }
        }
        psqt[v] = sum + correction;
    }
    let p: [f32; H] =
        std::array::from_fn(|j| ALPHA * c(a[j / HALF][j % HALF]) * c(a[j / HALF][j % HALF + HALF]));
    let y: [f32; 16] = std::array::from_fn(|k| {
        let lanes = std::array::from_fn(|lane| {
            (lane..H).step_by(32).fold(0.0_f32, |s, j| {
                s + (w[P::L1Weights][k * H + j] + w[P::L1SharedWeights][k * H + j]) * p[j]
            })
        });
        w[P::L1Biases][k] + w[P::L1SharedBiases][k] + sum32(lanes)
    });
    let z: [f32; 30] = std::array::from_fn(|i| {
        if i < 15 {
            c(ALPHA * y[i] * y[i])
        } else {
            c(y[i - 15])
        }
    });
    let u: [f32; 32] = std::array::from_fn(|k| {
        let lanes = std::array::from_fn(|j| {
            if j < 30 {
                w[P::L2Weights][k * 30 + j] * z[j]
            } else {
                0.0
            }
        });
        w[P::L2Biases][k] + sum32(lanes)
    });
    let n = w[P::L3Bias][0] + y[15] + sum32(std::array::from_fn(|j| w[P::L3Weights][j] * c(u[j])));
    Forward {
        a: a.map(|v| v.map(f64::from)),
        p: p.map(f64::from),
        y: y.map(f64::from),
        z: z.map(f64::from),
        u: u.map(f64::from),
        n: f64::from(n),
        psqt: psqt.map(f64::from),
        output: f64::from(n + (psqt[0] - psqt[1]) * 0.5),
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
/// 平均二乗誤差。教師値は既存のbuild_targetsと同じ混合式であらかじめ構築する。
pub fn loss(w: &Parameters, scale: Scale, features: &[Features], targets: &[f32]) -> Result<f64> {
    w.validate()?;
    check_batch(features, targets)?;
    Ok(features
        .iter()
        .zip(targets)
        .map(|(f, r)| {
            (sigmoid(scale.ratio() * forward_validated(w, f).output) - f64::from(*r)).powi(2)
        })
        .sum::<f64>()
        / features.len() as f64)
}

/// 局面ごとの積とδはGPUと同じ単精度の演算順で計算する。
/// 局面方向の和は倍精度で累算し、最後にf32へ丸める。
/// 後段の重みの勾配はバッチ全体で1組だけ保持し、局面数倍の配列は確保しない。
/// CReLUの境界の微分は0であり、鏡映共有の勾配は平均せず加算する。
pub fn backward(
    w: &Parameters,
    scale: Scale,
    features: &[Features],
    targets: &[f32],
) -> Result<(f64, Parameters)> {
    w.validate()?;
    check_batch(features, targets)?;
    let mut gradients: [Vec<f64>; 11] = P::ALL.map(|p| vec![0.0; p.count()]);
    let mut total_loss = 0.0;
    for (features, &target) in features.iter().zip(targets) {
        let f = forward_validated(w, features);
        let sigma = sigmoid(scale.ratio() * f.output);
        total_loss += (sigma - f64::from(target)).powi(2);
        // sigmoidと尺度の計算はCUDAと同じf64で行い、g_Yをf32へ丸める。
        let gy = (2.0 / targets.len() as f64
            * (sigma - f64::from(target))
            * sigma
            * (1.0 - sigma)
            * scale.ratio()) as f32;
        let mut add = |p: P, i: usize, value: f32| {
            gradients[p as usize][i] += f64::from(value);
        };
        add(P::L3Bias, 0, gy);
        let du: [f32; 32] =
            std::array::from_fn(|k| gy * w[P::L3Weights][k] * crelu_grad(f.u[k] as f32));
        for (k, &du) in du.iter().enumerate() {
            add(P::L3Weights, k, gy * c(f.u[k] as f32));
            add(P::L2Biases, k, du);
            for i in 0..30 {
                add(P::L2Weights, k * 30 + i, du * f.z[i] as f32);
            }
        }
        let dz: [f32; 30] = std::array::from_fn(|i| {
            sum32(std::array::from_fn(|k| du[k] * w[P::L2Weights][k * 30 + i]))
        });
        let dy: [f32; 16] = std::array::from_fn(|k| {
            if k == 15 {
                gy
            } else {
                dz[k] * square_grad(f.y[k] as f32) + dz[k + 15] * crelu_grad(f.y[k] as f32)
            }
        });
        let mut dp = [0.0_f32; H];
        for (k, &dy) in dy.iter().enumerate() {
            add(P::L1Biases, k, dy);
            add(P::L1SharedBiases, k, dy);
            for (i, dp) in dp.iter_mut().enumerate() {
                add(P::L1Weights, k * H + i, dy * f.p[i] as f32);
                add(P::L1SharedWeights, k * H + i, dy * f.p[i] as f32);
                *dp += dy * (w[P::L1Weights][k * H + i] + w[P::L1SharedWeights][k * H + i]);
            }
        }
        for v in 0..2 {
            let da: [f32; H] = std::array::from_fn(|j| {
                dp[v * HALF + j % HALF]
                    * ALPHA
                    * c(f.a[v][(j + HALF) % H] as f32)
                    * crelu_grad(f.a[v][j] as f32)
            });
            // 半幅の共有成分もf32同士で先に足さず、各項をf64へ広げてから足す。
            for j in 0..HALF {
                gradients[P::FtBiases as usize][j] += f64::from(da[j]) + f64::from(da[j + HALF]);
            }
            for &base in &features.views[v] {
                let row = canonical(base);
                gradients[P::Psqt as usize][row] +=
                    f64::from(if v == 0 { gy * 0.5 } else { gy * -0.5 });
                for j in 0..H {
                    gradients[P::FtWeights as usize][row * H + j] += f64::from(da[unit(base, j)]);
                }
            }
        }
    }

    let mut result = Parameters::zeros();
    for p in P::ALL {
        for (out, &g) in result[p].iter_mut().zip(&gradients[p as usize]) {
            *out = g as f32;
        }
    }
    result.validate()?;
    Ok((total_loss / targets.len() as f64, result))
}

/// 既存の教師混合処理を使い、P型バッチを1回更新する。予定から受け取る倍率を適用する。
pub fn train_batch(
    w: &mut Parameters,
    optimizer: &mut super::AdamW,
    scale: Scale,
    batch: &BatchSlot,
    lambda: f64,
    multiplier: f64,
) -> Result<f64> {
    let features = Features::from_batch(batch)?;
    // 分類ごとに同じK_tとλを使うため、共通の教師混合処理へ分類0として渡す。
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

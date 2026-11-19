//! 駒除去診断用の倍精度順伝播。学習と同じLayerStackの式を丸めずに計算する。
use super::model::{ALPHA, HALF, canonical, unit};
use super::{Features, H, Parameter as P, Parameters};

#[derive(Clone)]
pub(crate) struct Accumulator {
    a: [[f64; H]; 2],
    psqt: [f64; 2],
}
impl Accumulator {
    /// 呼び出し元で検査済みの重みと両視点の特徴から全再計算する。
    pub(crate) fn new(w: &Parameters, features: &Features) -> Self {
        let mut acc = Self {
            a: [std::array::from_fn(|j| f64::from(w[P::FtBiases][j % HALF])); 2],
            psqt: [0.0; 2],
        };
        for (v, rows) in features.views().iter().enumerate() {
            for &f in rows {
                acc.add(w, v, f, 1.0);
            }
        }
        acc
    }
    fn add(&mut self, w: &Parameters, view: usize, feature: usize, sign: f64) {
        let row = canonical(feature);
        self.psqt[view] += sign * f64::from(w[P::Psqt][row]);
        for j in 0..H {
            self.a[view][j] += sign * f64::from(w[P::FtWeights][row * H + unit(feature, j)]);
        }
    }
    /// 指定した駒の両視点の行だけを除き、先獅子の特徴は残す。
    pub(crate) fn without(&self, w: &Parameters, rows: [usize; 2]) -> Self {
        let mut after = self.clone();
        for (v, row) in rows.into_iter().enumerate() {
            after.add(w, v, row, -1.0);
        }
        after
    }
    /// Y = L3(CReLU(L2(z))) + y[15] + (PSQT_us − PSQT_them)/2。
    pub(crate) fn output(&self, w: &Parameters) -> f64 {
        let c = |x: f64| x.clamp(0.0, 1.0);
        let p: [f64; H] = std::array::from_fn(|j| {
            ALPHA * c(self.a[j / HALF][j % HALF]) * c(self.a[j / HALF][j % HALF + HALF])
        });
        let y: [f64; 16] = std::array::from_fn(|k| {
            f64::from(w[P::L1Biases][k])
                + f64::from(w[P::L1SharedBiases][k])
                + p.iter()
                    .enumerate()
                    .map(|(j, p)| {
                        (f64::from(w[P::L1Weights][k * H + j])
                            + f64::from(w[P::L1SharedWeights][k * H + j]))
                            * p
                    })
                    .sum::<f64>()
        });
        let z: [f64; 30] = std::array::from_fn(|j| {
            if j < 15 {
                c(ALPHA * y[j] * y[j])
            } else {
                c(y[j - 15])
            }
        });
        let dense = (0..32)
            .map(|k| {
                let u = f64::from(w[P::L2Biases][k])
                    + z.iter()
                        .enumerate()
                        .map(|(j, z)| f64::from(w[P::L2Weights][k * 30 + j]) * z)
                        .sum::<f64>();
                f64::from(w[P::L3Weights][k]) * c(u)
            })
            .sum::<f64>();
        f64::from(w[P::L3Bias][0]) + y[15] + dense + (self.psqt[0] - self.psqt[1]) * 0.5
    }
}

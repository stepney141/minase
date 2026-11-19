//! 駒除去の差分を検査する倍精度の累算値と後段。
use super::{Features, Parameter as P, Parameters, SPEC};
#[derive(Clone)]
pub(crate) struct Accumulator {
    a: [Vec<f64>; 2],
}
impl Accumulator {
    pub(crate) fn new(w: &Parameters, features: &Features) -> Self {
        let bias: Vec<_> = w[P::FtBiases].iter().copied().map(f64::from).collect();
        let mut result = Self {
            a: [bias.clone(), bias],
        };
        for (v, rows) in features.views().iter().enumerate() {
            for &r in rows {
                result.add(w, v, r, 1.0);
            }
        }
        result
    }
    fn add(&mut self, w: &Parameters, view: usize, row: usize, sign: f64) {
        for (j, a) in self.a[view].iter_mut().enumerate() {
            *a += sign
                * (f64::from(w[P::FtWeights][row * w.width() + j])
                    + f64::from(w[P::FtVirtual][(row % SPEC.piece_inputs()) * w.width() + j]));
        }
    }
    pub(crate) fn without(&self, w: &Parameters, rows: [usize; 2]) -> Self {
        let mut result = self.clone();
        for (v, r) in rows.into_iter().enumerate() {
            result.add(w, v, r, -1.0);
        }
        result
    }
    pub(crate) fn output(&self, w: &Parameters) -> f64 {
        let c = |x: f64| x.clamp(0.0, 1.0);
        let x: Vec<_> = self.a.iter().flatten().copied().map(c).collect();
        let dense = |weights: P, bias: P, input: &[f64]| -> Vec<f64> {
            (0..32)
                .map(|k| {
                    c(f64::from(w[bias][k])
                        + input
                            .iter()
                            .enumerate()
                            .map(|(j, x)| f64::from(w[weights][k * input.len() + j]) * x)
                            .sum::<f64>())
                })
                .collect()
        };
        let h1 = dense(P::L1Weights, P::L1Biases, &x);
        let h2 = dense(P::L2Weights, P::L2Biases, &h1);
        f64::from(w[P::L3Bias][0])
            + h2.iter()
                .zip(&w[P::L3Weights])
                .map(|(x, &w)| x * f64::from(w))
                .sum::<f64>()
    }
}

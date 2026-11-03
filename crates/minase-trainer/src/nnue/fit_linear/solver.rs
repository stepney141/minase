//! 重みつきロジスティック回帰を、固定順のニュートン法で解く。

use super::{Error, Result};

/// ロジットを`offset + Σ terms[i] × theta[i]`とする1局面。
pub(super) struct Row {
    /// パラメータ番号とロジットへの係数の疎な列。
    pub terms: Vec<(usize, f64)>,
    /// 固定されたロジット。
    pub offset: f64,
    /// 手番側の結果。勝ち1、引き分け0.5、負け0。
    pub target: f64,
    /// 対局ごとの総和を1にそろえる局面の重み。
    pub weight: f64,
}

impl Row {
    fn logit(&self, theta: &[f64]) -> f64 {
        self.offset + self.terms.iter().map(|&(i, x)| theta[i] * x).sum::<f64>()
    }
}

/// ロジットから二値交差エントロピーを求め、指数の桁あふれと大きな値同士の相殺を避ける。
pub(super) fn bce(logit: f64, target: f64) -> f64 {
    let linear = if logit >= 0.0 {
        (1.0 - target) * logit
    } else {
        -target * logit
    };
    linear + (-logit.abs()).exp().ln_1p()
}

fn objective(rows: &[Row], theta: &[f64], precision: f64) -> f64 {
    let terms = rows
        .iter()
        .map(|row| row.weight * bce(row.logit(theta), row.target))
        .chain(theta.iter().map(|x| 0.5 * precision * x * x));
    // 多数の局面の加算誤差で、収束直前のArmijo判定が反転しないよう補償する。
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for term in terms {
        let adjusted = term - compensation;
        let next = sum + adjusted;
        compensation = (next - sum) - adjusted;
        sum = next;
    }
    sum
}

/// 下三角に入った対称行列をコレスキー分解し、`H step = gradient`を解く。
fn cholesky_solve(
    mut matrix: Vec<f64>,
    mut gradient: Vec<f64>,
    stage: &'static str,
) -> Result<Vec<f64>> {
    let n = gradient.len();
    for i in 0..n {
        for j in 0..=i {
            let mut value = matrix[i * n + j];
            for k in 0..j {
                value -= matrix[i * n + k] * matrix[j * n + k];
            }
            matrix[i * n + j] = if i == j {
                if !value.is_finite() || value <= 0.0 {
                    return Err(Error::NotPositiveDefinite {
                        stage,
                        pivot: i,
                        value,
                    });
                }
                value.sqrt()
            } else {
                value / matrix[j * n + j]
            };
        }
    }
    for i in 0..n {
        for j in 0..i {
            gradient[i] -= matrix[i * n + j] * gradient[j];
        }
        gradient[i] /= matrix[i * n + i];
    }
    for i in (0..n).rev() {
        for j in i + 1..n {
            gradient[i] -= matrix[j * n + i] * gradient[j];
        }
        gradient[i] /= matrix[i * n + i];
    }
    Ok(gradient)
}

/// 0から開始し、逆分散`precision`の正則化を加える。0なら最尤推定。
pub(super) fn newton(
    rows: &[Row],
    n: usize,
    precision: f64,
    stage: &'static str,
) -> Result<Vec<f64>> {
    let mut theta = vec![0.0; n];
    for _ in 0..100 {
        let loss = objective(rows, &theta, precision);
        let mut gradient: Vec<_> = theta.iter().map(|x| precision * x).collect();
        let mut hessian = vec![0.0; n * n];
        for i in 0..n {
            hessian[i * n + i] = precision;
        }
        for row in rows {
            let logit = row.logit(&theta);
            let exponential = (-logit.abs()).exp();
            let denominator = 1.0 + exponential;
            let probability = if logit >= 0.0 {
                1.0 / denominator
            } else {
                exponential / denominator
            };
            let residual = row.weight * (probability - row.target);
            let curvature = row.weight * exponential / (denominator * denominator);
            for &(i, x) in &row.terms {
                gradient[i] += residual * x;
                for &(j, y) in &row.terms {
                    if j <= i {
                        hessian[i * n + j] += curvature * x * y;
                    }
                }
            }
        }
        if !loss.is_finite() || gradient.iter().chain(&hessian).any(|x| !x.is_finite()) {
            return Err(Error::NonFinite { stage });
        }
        let step = cholesky_solve(hessian, gradient.clone(), stage)?;
        if step.iter().any(|x| !x.is_finite()) {
            return Err(Error::NonFinite { stage });
        }
        if step.iter().map(|x| x.abs()).fold(0.0, f64::max) < 1e-6 {
            return Ok(theta);
        }
        let decrease: f64 = gradient.iter().zip(&step).map(|(g, d)| g * d).sum();
        if !decrease.is_finite() || decrease <= 0.0 {
            return Err(Error::LineSearch { stage });
        }
        let mut alpha = 1.0;
        let mut accepted = None;
        for _ in 0..60 {
            let candidate: Vec<_> = theta
                .iter()
                .zip(&step)
                .map(|(x, d)| x - alpha * d)
                .collect();
            let candidate_loss = objective(rows, &candidate, precision);
            if candidate_loss.is_finite() && candidate_loss <= loss - 1e-4 * alpha * decrease {
                accepted = Some(candidate);
                break;
            }
            alpha *= 0.5;
        }
        theta = accepted.ok_or(Error::LineSearch { stage })?;
    }
    Err(Error::NonConvergence { stage })
}

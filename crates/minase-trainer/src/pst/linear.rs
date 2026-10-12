//! PSTの固定順の和、位相補間と駒除去差分。

use super::batch::SENTINEL;

pub(super) fn phase(numerator: u8) -> [f32; 2] {
    let phi = f32::from(numerator) / 90.0;
    [phi, 1.0 - phi]
}

pub(super) fn row(weights: &[f32], feature: u32) -> [f32; 2] {
    if feature == SENTINEL {
        [0.0; 2]
    } else {
        let i = 2 * feature as usize;
        [weights[i], weights[i + 1]]
    }
}

// 左右の升を先に対にし、鏡映しても加算順による丸めを変えない。
pub(super) fn feature_sum<T>(mut value: impl FnMut(usize) -> T) -> T
where
    T: Copy + Default + std::ops::Add<Output = T> + std::ops::AddAssign,
{
    let mut result = T::default();
    for rank in 0..12 {
        for file in 0..6 {
            result += value(rank * 12 + file) + value(rank * 12 + 11 - file);
        }
    }
    result + value(144)
}

pub(super) fn sum_mirrored_f64(weights: &[f32], columns: &[u32]) -> [f64; 2] {
    std::array::from_fn(|endpoint| {
        feature_sum(|col| f64::from(row(weights, columns[col])[endpoint]))
    })
}

pub(super) fn blend(sums: [f32; 2], numerator: u8) -> f32 {
    let u = phase(numerator);
    u[0] * sums[0] + u[1] * sums[1]
}

// pst.removal._numeratorsと同じ順序で、丸め前の評価差を求める。
pub(super) fn removal_delta(sums: [f64; 2], removed: [f64; 2], numerators: [u8; 2]) -> f64 {
    let [q, after_q] = numerators.map(f64::from);
    let before = q * sums[0] + (90.0 - q) * sums[1];
    let after = after_q * (sums[0] - removed[0]) + (90.0 - after_q) * (sums[1] - removed[1]);
    (after - before) / 90.0
}

pub(super) fn eligible(row: u32, reference: i32) -> bool {
    row != SENTINEL
        && (row as usize) < 94 * 72
        && ![11, 21].contains(&(row as usize / 72 % 47))
        && reference != 0
}

pub(super) fn wrong_direction(row: u32, reference: i32) -> bool {
    (row < 47 * 72 && reference > 0) || ((47 * 72..94 * 72).contains(&row) && reference < 0)
}

// 戻り値の微分は-1、0、1。PSTは整数係数を合算するまで浮動小数点化しない。
pub(super) fn penalty(delta: f64, sign: i32, magnitude: f64, wrong: bool) -> (f64, i32) {
    let signed = f64::from(sign) * delta;
    let lower = magnitude.min(0.25);
    let penalty = (lower - signed).max(0.0)
        + if wrong {
            (signed - magnitude).max(0.0)
        } else {
            0.0
        };
    let derivative = if signed < lower {
        -sign
    } else if wrong && signed > magnitude {
        sign
    } else {
        0
    };
    (penalty, derivative)
}

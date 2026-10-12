//! NNUEとPSTに共通する線形和、位相補間、駒除去の計算。

use crate::data::SENTINEL;

pub(crate) fn phase_numerator(count: u8, removed: bool) -> u8 {
    count.saturating_sub(if removed { 3 } else { 2 }).min(90)
}

pub(crate) fn phase(count: u8, removed: bool) -> [f32; 2] {
    let phi = f32::from(phase_numerator(count, removed)) / 90.0;
    [phi, 1.0 - phi]
}

pub(crate) fn row(weights: &[f32], feature: u32) -> [f32; 2] {
    if feature == SENTINEL {
        [0.0; 2]
    } else {
        let i = 2 * feature as usize;
        [weights[i], weights[i + 1]]
    }
}

pub(crate) fn sum_f64(weights: &[f32], columns: &[u32]) -> [f64; 2] {
    let mut sums = [0.0; 2];
    for &feature in columns {
        for (sum, value) in sums.iter_mut().zip(row(weights, feature)) {
            *sum += f64::from(value);
        }
    }
    sums
}

// NNUEの既存の縮約順。左右の升を先に対にする。
pub(crate) fn feature_sum<T>(mut value: impl FnMut(usize) -> T) -> T
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

pub(crate) fn blend(sums: [f32; 2], count: u8) -> f32 {
    let u = phase(count, false);
    u[0] * sums[0] + u[1] * sums[1]
}

// pst.removal._numeratorsと同じ順序で、丸め前の評価差を求める。
pub(crate) fn removal_delta(sums: [f64; 2], removed: [f64; 2], count: u8) -> f64 {
    let q = f64::from(phase_numerator(count, false));
    let after_q = f64::from(phase_numerator(count, true));
    let before = q * sums[0] + (90.0 - q) * sums[1];
    let after = after_q * (sums[0] - removed[0]) + (90.0 - after_q) * (sums[1] - removed[1]);
    (after - before) / 90.0
}

pub(crate) fn eligible(row: u32, reference: i32) -> bool {
    row != SENTINEL
        && (row as usize) < 94 * 72
        && ![11, 21].contains(&(row as usize / 72 % 47))
        && reference != 0
}

pub(crate) fn wrong_direction(row: u32, reference: i32) -> bool {
    (row < 47 * 72 && reference > 0) || ((47 * 72..94 * 72).contains(&row) && reference < 0)
}

// 戻り値の微分は-1、0、1。PSTは整数係数を合算するまで浮動小数点化しない。
pub(crate) fn penalty(delta: f64, sign: i32, magnitude: f64, wrong: bool) -> (f64, i32) {
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

//! PSTの固定順の和、盤上総駒数による補間、および駒除去差分。
//! 前進は`sum_mirrored_f64`のf64の和をf32へ丸め、`blend`で補間する。

use super::batch::SENTINEL;

/// [`super::Batch::phase_numerators`]の分子qを90で割ったf32の係数`(φ,1−φ)`を返す。
/// 理由は`docs/adr/interpolate-the-linear-path-by-total-piece-count.md`にある。
pub(crate) fn phase(numerator: u8) -> [f32; 2] {
    let phi = f32::from(numerator) / 90.0;
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

/// 各段で筋fと11−fを先に対にして加え、段の順に累算し、最後に先獅子の列144を足す。
/// 左右鏡映しても同じ加算順となり、丸めが変わらない。
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

/// [`feature_sum`]の順序で両端点をf64で加算する。
pub(crate) fn sum_mirrored_f64(weights: &[f32], columns: &[u32]) -> [f64; 2] {
    std::array::from_fn(|endpoint| {
        feature_sum(|col| f64::from(row(weights, columns[col])[endpoint]))
    })
}

/// 端点の和から`E = φ×S_mg + (1−φ)×S_eg`をf32で返す。
/// 単位はセンチポーン、視点は手番側であり、φは[`phase`]で求める。
pub(crate) fn blend(sums: [f32; 2], numerator: u8) -> f32 {
    let u = phase(numerator);
    u[0] * sums[0] + u[1] * sums[1]
}

/// 1枚を除く評価差をセンチポーンで求め、整数への丸めは行わない。
/// 除去前後の分子qとq′、端点和S、除く行wに対し、`Δ = (q′(S_mg−w_mg)+(90−q′)(S_eg−w_eg)−q×S_mg−(90−q)×S_eg)/90`とする。
pub(crate) fn removal_delta(sums: [f64; 2], removed: [f64; 2], numerators: [u8; 2]) -> f64 {
    let [q, after_q] = numerators.map(f64::from);
    let before = q * sums[0] + (90.0 - q) * sums[1];
    let after = after_q * (sums[0] - removed[0]) + (90.0 - after_q) * (sums[1] - removed[1]);
    (after - before) / 90.0
}

/// 駒の行`row < 94×72`のうち、王将の状態11、太子の状態21、および整数参照差分0を除いたものを適格とする。
/// 空升の番兵と先獅子の列は適格としない。
pub(crate) fn eligible(row: u32, reference: i32) -> bool {
    row != SENTINEL
        && (row as usize) < 94 * 72
        && ![11, 21].contains(&(row as usize / 72 % 47))
        && reference != 0
}

/// 自駒の行`row < 47×72`で参照差分が正、または相手駒で負なら逆向きとする。
pub(crate) fn wrong_direction(row: u32, reference: i32) -> bool {
    (row < 47 * 72 && reference > 0) || ((47 * 72..94 * 72).contains(&row) && reference < 0)
}

/// `s = sign(ref)×Δ`、`ℓ = min(|ref_pre|,0.25)`として、`max(0,ℓ−s) + [逆向き]×max(0,s−|ref_pre|)`を返す。
/// 併せて返すΔに関する微分は−1、0、+1であり、PSTは整数係数を合算してから倍率を掛ける。
/// 理由は`docs/adr/penalize-wrong-signed-piece-removal-differences.md`にある。
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

/// 各列rの除去係数を、序中盤は`(q′−q)Σd_j − q′d_r`、終盤は`(q−q′)Σd_j − (90−q′)d_r`で求める。
/// dは[`penalty`]の整数微分であり、鏡映対が同じ行に写る場合も整数で合算してからscaleを掛ける。
/// 各行の出現数に応じた二値交差エントロピーの微分`g×u`も加える。
pub(crate) fn gradient_rows(
    columns: &[u32],
    numerators: [u8; 2],
    derivatives: &[i32; 145],
    g: f64,
    u: [f64; 2],
    scale: f64,
) -> Vec<(u32, [f64; 2])> {
    let [q, after_q] = numerators.map(i32::from);
    let derivative_sum: i32 = derivatives.iter().sum();
    let common = (after_q - q) * derivative_sum;
    // 各除去で、自身は(-q, -(90-q))、残存行は(q'-q, q-q')。
    // 全行に残存係数を足して自身から(q',90-q')を引くと同じ整数係数になる。
    // 鏡映対が同じ行へ写る場合も整数で合算してから倍率を掛け、f32差分で相殺しない。
    let mut rows = std::collections::BTreeMap::<u32, [i32; 3]>::new();
    for (col, &row) in columns.iter().enumerate() {
        if row == SENTINEL {
            continue;
        }
        let entry = rows.entry(row).or_default();
        entry[0] += 1;
        entry[1] += common - after_q * derivatives[col];
        entry[2] += -common - (90 - after_q) * derivatives[col];
    }
    rows.into_iter()
        .map(|(row, counts)| {
            (
                row,
                [
                    f64::from(counts[0]) * g * u[0] + f64::from(counts[1]) * scale,
                    f64::from(counts[0]) * g * u[1] + f64::from(counts[2]) * scale,
                ],
            )
        })
        .collect()
}

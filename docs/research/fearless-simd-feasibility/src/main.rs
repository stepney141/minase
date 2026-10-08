#![forbid(unsafe_code)]
use fearless_simd::*;
use fearless_simd_macros::simd;

const RANK: usize = 32;

/// after = before - removed + added, 32 lanes, i16 rows widened to i32.
#[simd]
fn replace_sums<S: Simd>(simd: S, before: &[i32; RANK], after: &mut [i32; RANK], removed: &[i16; RANK], added: &[i16; RANK]) {
    let n16 = S::i16s::LEN;
    let n32 = S::i32s::LEN;
    let mut i = 0;
    while i < RANK {
        let r = S::i16s::from_slice(simd, &removed[i..i + n16]);
        let a = S::i16s::from_slice(simd, &added[i..i + n16]);
        let (r_lo, r_hi) = r.widen();
        let (a_lo, a_hi) = a.widen();
        let b_lo = S::i32s::from_slice(simd, &before[i..i + n32]);
        let b_hi = S::i32s::from_slice(simd, &before[i + n32..i + 2 * n32]);
        (b_lo - r_lo + a_lo).store_slice(&mut after[i..i + n32]);
        (b_hi - r_hi + a_hi).store_slice(&mut after[i + n32..i + 2 * n32]);
        i += n16;
    }
}

/// sum over lanes of sign[f] * a[f] * a[f] in i64.
#[simd]
fn signed_square_sum<S: Simd>(simd: S, sums: &[i32; RANK], signs: &[i32; RANK]) -> i64 {
    let n32 = S::i32s::LEN;
    let mut total = S::i64s::splat(simd, 0);
    let mut i = 0;
    while i < RANK {
        let a = S::i32s::from_slice(simd, &sums[i..i + n32]);
        let d = S::i32s::from_slice(simd, &signs[i..i + n32]);
        let (a_lo, a_hi) = a.widen();
        let (ad_lo, ad_hi) = (a * d).widen();
        total = total + a_lo * ad_lo + a_hi * ad_hi;
        i += n32;
    }
    total.reduce_sum()
}

fn main() {
    let level = Level::new();
    println!("level = {level:?}");
    let before: [i32; RANK] = std::array::from_fn(|i| i as i32 * 1000);
    let mut after = [0i32; RANK];
    let removed: [i16; RANK] = std::array::from_fn(|i| i as i16);
    let added: [i16; RANK] = std::array::from_fn(|i| -(i as i16) * 3);
    dispatch!(level, simd => replace_sums(simd, &before, &mut after, &removed, &added));
    let expected: Vec<i32> = (0..RANK).map(|i| before[i] - removed[i] as i32 + added[i] as i32).collect();
    assert_eq!(&after[..], &expected[..]);
    let signs: [i32; RANK] = std::array::from_fn(|i| if i % 3 == 0 { -1 } else { 1 });
    let sums: [i32; RANK] = std::array::from_fn(|i| (i as i32 - 16) * 148_480);
    let got = dispatch!(level, simd => signed_square_sum(simd, &sums, &signs));
    let want: i64 = (0..RANK).map(|i| i64::from(signs[i]) * i64::from(sums[i]) * i64::from(sums[i])).sum();
    assert_eq!(got, want);
    println!("ok {got}");
}

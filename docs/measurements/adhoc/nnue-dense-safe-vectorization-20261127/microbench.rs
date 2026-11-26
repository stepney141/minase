#![forbid(unsafe_code)]
use std::hint::black_box;
use std::time::Instant;

const W: usize = 256;
const H: usize = W / 2;

struct Net {
    l1_b: Vec<i32>,
    l1_w: Vec<i8>,
    l2_b: Vec<i32>,
    l2_w: Vec<i8>,
    out_b: i32,
    out_w: Vec<i8>,
    // i16 copies for safe vectorization
    l1_w16: Vec<i16>,   // [16][W]
    l1_wt16: Vec<i16>,  // [W/2][16][2]
    l2_w16: Vec<i16>,   // [32][32], inputs 30,31 zero
    out_w16: [i16; 32],
    l2_wt16: Vec<i16>,  // [30][32] input-major
    l1_wtp: Vec<i16>,   // [W][16] input-major
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn i8(&mut self, lim: i64) -> i8 {
        ((self.next() % (2 * lim as u64 + 1)) as i64 - lim) as i8
    }
}

impl Net {
    fn random() -> Self {
        let mut r = Rng(0x9e3779b97f4a7c15);
        let l1_b: Vec<i32> = (0..16).map(|_| (r.next() % 20000) as i32 - 10000).collect();
        let l1_w: Vec<i8> = (0..16 * W).map(|_| r.i8(127)).collect();
        let l2_b: Vec<i32> = (0..32).map(|_| (r.next() % 20000) as i32 - 10000).collect();
        let l2_w: Vec<i8> = (0..32 * 30).map(|_| r.i8(127)).collect();
        let out_w: Vec<i8> = (0..32).map(|_| r.i8(127)).collect();
        let l1_w16 = l1_w.iter().map(|&w| i16::from(w)).collect();
        let mut l1_wt16 = vec![0i16; 16 * W];
        for j in 0..W / 2 {
            for o in 0..16 {
                for k in 0..2 {
                    l1_wt16[j * 32 + o * 2 + k] = i16::from(l1_w[o * W + 2 * j + k]);
                }
            }
        }
        let mut l2_w16 = vec![0i16; 32 * 32];
        for o in 0..32 {
            for j in 0..30 {
                l2_w16[o * 32 + j] = i16::from(l2_w[o * 30 + j]);
            }
        }
        let out_w16 = std::array::from_fn(|k| i16::from(out_w[k]));
        let mut l2_wt16 = vec![0i16; 30 * 32];
        for o in 0..32 {
            for j in 0..30 {
                l2_wt16[j * 32 + o] = i16::from(l2_w[o * 30 + j]);
            }
        }
        let mut l1_wtp = vec![0i16; W * 16];
        for o in 0..16 {
            for j in 0..W {
                l1_wtp[j * 16 + o] = i16::from(l1_w[o * W + j]);
            }
        }
        Net { l1_b, l1_w, l2_b, l2_w, out_b: 1234, out_w, l1_w16, l1_wt16, l2_w16, out_w16, l2_wt16, l1_wtp }
    }
}

fn pair(a: i16, b: i16) -> u8 {
    ((i32::from(a).clamp(0, 127) * i32::from(b).clamp(0, 127)) >> 7) as u8
}
fn square_activation(y: i32) -> u8 {
    ((i64::from(y) * i64::from(y)) >> 19).clamp(0, 127) as u8
}
fn activation(y: i32) -> u8 {
    (y >> 6).clamp(0, 127) as u8
}
fn affine(bias: i32, weights: &[i8], input: &[u8]) -> i32 {
    bias + weights
        .iter()
        .zip(input)
        .map(|(&w, &x)| i32::from(w) * i32::from(x))
        .sum::<i32>()
}

/// Current engine code (runtime width, row-major i8 weights).
#[inline(never)]
fn dense0(d: &Net, width: usize, rows: [&[i16]; 2]) -> i32 {
    let mut storage = [0; 1024];
    let input = &mut storage[..width];
    for (part, row) in rows.into_iter().enumerate() {
        for i in 0..width / 2 {
            input[part * width / 2 + i] = pair(row[i], row[i + width / 2]);
        }
    }
    let mut z = [0; 30];
    for i in 0..15 {
        let y = affine(d.l1_b[i], &d.l1_w[i * width..(i + 1) * width], input);
        z[i] = square_activation(y);
        z[15 + i] = activation(y);
    }
    let skip = affine(d.l1_b[15], &d.l1_w[15 * width..16 * width], input);
    let h: [u8; 32] = std::array::from_fn(|k| {
        activation(affine(d.l2_b[k], &d.l2_w[k * 30..(k + 1) * 30], &z))
    });
    affine(d.out_b, &d.out_w, &h) + skip
}

/// Pair stage writing i16 so the products below are i16 x i16.
fn pairs16(rows: [&[i16]; 2]) -> [i16; W] {
    let mut input = [0i16; W];
    for (part, row) in rows.into_iter().enumerate() {
        let row: &[i16; W] = row.try_into().unwrap();
        let (lo, hi) = row.split_at(H);
        for ((o, &a), &b) in input[part * H..(part + 1) * H].iter_mut().zip(lo).zip(hi) {
            *o = (a.clamp(0, 127) * b.clamp(0, 127)) >> 7;
        }
    }
    input
}

fn dot16(w: &[i16; W], x: &[i16; W]) -> i32 {
    w.iter().zip(x).map(|(&w, &x)| i32::from(w) * i32::from(x)).sum()
}

fn tail16(d: &Net, y: [i32; 16]) -> i32 {
    let mut z = [0i16; 32];
    for i in 0..15 {
        z[i] = i16::from(square_activation(y[i]));
        z[15 + i] = i16::from(activation(y[i]));
    }
    let mut h = [0i16; 32];
    for (o, hv) in h.iter_mut().enumerate() {
        let w: &[i16; 32] = d.l2_w16[o * 32..(o + 1) * 32].try_into().unwrap();
        let s = d.l2_b[o] + w.iter().zip(&z).map(|(&w, &x)| i32::from(w) * i32::from(x)).sum::<i32>();
        *hv = i16::from(activation(s));
    }
    d.out_b + d.out_w16.iter().zip(&h).map(|(&w, &x)| i32::from(w) * i32::from(x)).sum::<i32>() + y[15]
}

/// Safe: const width, i16 inputs and widened i16 weights, row-major dot products.
#[inline(never)]
fn dense_s1(d: &Net, rows: [&[i16]; 2]) -> i32 {
    let input = pairs16(rows);
    let y: [i32; 16] = std::array::from_fn(|o| {
        let w: &[i16; W] = d.l1_w16[o * W..(o + 1) * W].try_into().unwrap();
        d.l1_b[o] + dot16(w, &input)
    });
    tail16(d, y)
}

/// Tail with input-major L2: two i16 products summed in i16 lanes (|sum| <= 2*127*128), then widened.
fn tail16t(d: &Net, y: [i32; 16]) -> i32 {
    let mut z = [0i16; 30];
    for i in 0..15 {
        z[i] = i16::from(square_activation(y[i]));
        z[15 + i] = i16::from(activation(y[i]));
    }
    let mut acc: [i32; 32] = d.l2_b[..32].try_into().unwrap();
    for (z2, w) in z.chunks_exact(2).zip(d.l2_wt16.chunks_exact(64)) {
        let (w0, w1) = w.split_at(32);
        let w0: &[i16; 32] = w0.try_into().unwrap();
        let w1: &[i16; 32] = w1.try_into().unwrap();
        for o in 0..32 {
            acc[o] += i32::from(z2[0] * w0[o] + z2[1] * w1[o]);
        }
    }
    let h: [i16; 32] = acc.map(|s| i16::from(activation(s)));
    d.out_b + d.out_w16.iter().zip(&h).map(|(&w, &x)| i32::from(w) * i32::from(x)).sum::<i32>() + y[15]
}

/// Safe: row-major pmaddwd L1 and input-major L2.
#[inline(never)]
fn dense_s3(d: &Net, rows: [&[i16]; 2]) -> i32 {
    let input = pairs16(rows);
    let y: [i32; 16] = std::array::from_fn(|o| {
        let w: &[i16; W] = d.l1_w16[o * W..(o + 1) * W].try_into().unwrap();
        d.l1_b[o] + dot16(w, &input)
    });
    tail16t(d, y)
}

/// Safe: input-major L1 (16 outputs in i16 lanes, pairs summed in i16) and input-major L2.
#[inline(never)]
fn dense_s4(d: &Net, rows: [&[i16]; 2]) -> i32 {
    let input = pairs16(rows);
    let mut y: [i32; 16] = d.l1_b[..16].try_into().unwrap();
    for (x2, w) in input.chunks_exact(2).zip(d.l1_wtp.chunks_exact(32)) {
        let (w0, w1) = w.split_at(16);
        let w0: &[i16; 16] = w0.try_into().unwrap();
        let w1: &[i16; 16] = w1.try_into().unwrap();
        for o in 0..16 {
            y[o] += i32::from(x2[0] * w0[o] + x2[1] * w1[o]);
        }
    }
    tail16t(d, y)
}

/// Safe: transposed pairs layout, 16 outputs kept in lanes (no horizontal sums).
#[inline(never)]
fn dense_s2(d: &Net, rows: [&[i16]; 2]) -> i32 {
    let input = pairs16(rows);
    let mut y: [i32; 16] = d.l1_b[..16].try_into().unwrap();
    for (x2, w) in input.chunks_exact(2).zip(d.l1_wt16.chunks_exact(32)) {
        let w: &[i16; 32] = w.try_into().unwrap();
        for o in 0..16 {
            y[o] += i32::from(x2[0]) * i32::from(w[2 * o]) + i32::from(x2[1]) * i32::from(w[2 * o + 1]);
        }
    }
    tail16(d, y)
}

fn main() {
    let net = Net::random();
    let mut r = Rng(12345);
    let n = 100_000;
    let all: Vec<i16> = (0..n * 2 * W).map(|_| (r.next() % 400) as i16 - 120).collect();
    let samples: Vec<&[i16]> = all.chunks_exact(2 * W).collect();
    let width = black_box(W);
    for s in &samples {
        let rows = [&s[..W], &s[W..]];
        let r0 = dense0(&net, width, rows);
        assert_eq!(r0, dense_s1(&net, rows));
        assert_eq!(r0, dense_s2(&net, rows));
        assert_eq!(r0, dense_s3(&net, rows));
        assert_eq!(r0, dense_s4(&net, rows));
    }
    println!("all variants agree");
    let variants: [(&str, &dyn Fn([&[i16]; 2]) -> i32); 5] = [
        ("current", &|r| dense0(&net, width, r)),
        ("safe-i16-rowmajor", &|r| dense_s1(&net, r)),
        ("safe-i16-transposed", &|r| dense_s2(&net, r)),
        ("safe-rowmajor+L2-inmajor", &|r| dense_s3(&net, r)),
        ("safe-inmajor-both", &|r| dense_s4(&net, r)),
    ];
    for round in 0..3 {
        for (name, f) in &variants {
            let t = Instant::now();
            let mut acc = 0i64;
            for _ in 0..5 {
                for s in &samples {
                    acc += i64::from(f(black_box([&s[..W], &s[W..]])));
                }
            }
            let ns = t.elapsed().as_nanos() as f64 / (5 * samples.len()) as f64;
            if round == 2 {
                println!("{name:24} {ns:7.1} ns/call  (chk {acc})");
            }
        }
    }
}

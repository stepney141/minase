//! 要素積、幅15と幅32の隠れ層、および直接内積で両視点の累算値を整数出力へ変換する。
//! 第1層の尺度は255で、各視点のH成分を4等分し、隣り合う区画の要素積を取る。
//! `c = clamp(acc, 0, 255)`をu16で持ち、`s = (c₁×c₂) >> 9`をi16へ格納する。
//! 積の上限は65,025、sの範囲は0〜127であり、手番側H/2個、相手側H/2個の順に並べる。
//!
//! 幅15の層は`z = W1·s + b1`をi32で求め、
//! `u[k] = min(127, (i64(z[k])²) >> 23)`と`u[15+k] = clamp(z[k] >> 8, 0, 127)`を連結する。
//! 負のzも2乗し、幅32の層は`y = clamp((W2·u + b2) >> 8, 0, 127)`とする。
//! 実数のz=1は整数の32,512にあたり、2乗の枝は126、切り詰めの枝は127となる。
//! 最終出力はi32の`o·s + v·y + b`であり、センチポーンへの換算は呼び手が行う。
//! zと第2隠れ層の和の右シフトは算術シフト、要素積の右シフトは論理シフトである。
//!
//! oとvは尺度4,096のi16で絶対値8,192以下、W1とW2は尺度256のi16で絶対値508以下とする。
//! b1とb2は尺度127×256のi32で絶対値260,096以下、bは尺度127×4,096のi32で絶対値4,161,536以下とする。
//! P型ではoとW1の各行を、各視点内の成分jとj+H/4で共有する。後続の層には共有を課さない。

use super::Error;
use super::format::{read_i16, read_i32, validate_range};

/// 出力区画の復号と、両視点の非線形累算値からの読み出し。
pub trait Head: Sized {
    /// 構造記述文字列での名前。
    const NAME: &'static str;
    /// 幅Hに対応する出力区画のバイト数。
    fn byte_len(width: usize) -> usize;
    /// 幅、区画長、値域を検査し、`shared_mirror`なら出力部の鏡映共有も検査する。
    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error>;
    /// 検査済みの整数配列をリトルエンディアンで符号化する。
    fn to_bytes(&self) -> Vec<u8>;
    /// 各視点の幅Hの累算値から、尺度変換前の出力を計算する。
    fn output(&self, us: &[i16], them: &[i16]) -> i32;
}

/// `pairwise-l16-l32`の検査済み出力区画。幅Hは4以上512以下の4の倍数とする。
/// バイト列は`o: i16[H]`、`b: i32`、`W1: i16[15][H]`、`b1: i32[15]`、
/// `W2: i16[32][30]`、`b2: i32[32]`、`v: i16[32]`の順で、行列は行優先である。
/// H=256で10,368バイトとなる。
pub struct PairwiseL16L32 {
    output_weights: Box<[i16]>,
    output_bias: i32,
    hidden1_weights: Box<[i16]>,
    hidden1_biases: [i32; 15],
    hidden2_weights: Box<[i16]>,
    hidden2_biases: [i32; 32],
    hidden2_output: [i16; 32],
    shared_mirror_weights: Option<SharedMirrorWeights>,
}

/// 各視点の鏡映対から片側だけを取り出した、半幅の推論用重み。
struct SharedMirrorWeights {
    output_weights: Box<[i16]>,
    hidden1_weights: Box<[i16]>,
}

impl PairwiseL16L32 {
    /// 幅、配列長、全値域、鏡映共有の順に検査する。
    /// W1とW2は行優先で、要素数はそれぞれ15Hと32×30である。
    /// `shared_mirror`ではoとW1の各行の各視点内で、jとj+H/4の一致を要求する。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        width: usize,
        output_weights: &[i16],
        output_bias: i32,
        hidden1_weights: &[i16],
        hidden1_biases: [i32; 15],
        hidden2_weights: &[i16],
        hidden2_biases: [i32; 32],
        hidden2_output: [i16; 32],
        shared_mirror: bool,
    ) -> Result<Self, Error> {
        validate_width(width)?;
        validate_length("output_weights", output_weights.len(), width)?;
        validate_length("hidden1_weights", hidden1_weights.len(), 15 * width)?;
        validate_length("hidden2_weights", hidden2_weights.len(), 32 * 30)?;
        validate_range(
            "output_weights",
            output_weights.iter().copied().map(i32::from),
            8_192,
        )?;
        validate_range("output_bias", [output_bias], 4_161_536)?;
        validate_range(
            "hidden1_weights",
            hidden1_weights.iter().copied().map(i32::from),
            508,
        )?;
        validate_range("hidden1_biases", hidden1_biases, 260_096)?;
        validate_range(
            "hidden2_weights",
            hidden2_weights.iter().copied().map(i32::from),
            508,
        )?;
        validate_range("hidden2_biases", hidden2_biases, 260_096)?;
        validate_range(
            "hidden2_output",
            hidden2_output.into_iter().map(i32::from),
            8_192,
        )?;
        if shared_mirror {
            validate_mirror("output_weights", output_weights, width)?;
            validate_mirror("hidden1_weights", hidden1_weights, width)?;
        }
        Ok(Self {
            output_weights: output_weights.into(),
            output_bias,
            hidden1_weights: hidden1_weights.into(),
            hidden1_biases,
            hidden2_weights: hidden2_weights.into(),
            hidden2_biases,
            hidden2_output,
            shared_mirror_weights: shared_mirror.then(|| SharedMirrorWeights {
                output_weights: output_weights
                    .chunks_exact(width / 2)
                    .flat_map(|view| view[..width / 4].iter().copied())
                    .collect(),
                hidden1_weights: hidden1_weights
                    .chunks_exact(width / 2)
                    .flat_map(|view| view[..width / 4].iter().copied())
                    .collect(),
            }),
        })
    }
}

impl Head for PairwiseL16L32 {
    const NAME: &'static str = "pairwise-l16-l32";

    fn byte_len(width: usize) -> usize {
        2 * width + 4 + 15 * width * 2 + 15 * 4 + 32 * 30 * 2 + 32 * 4 + 32 * 2
    }

    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error> {
        validate_width(width)?;
        let expected = Self::byte_len(width);
        if bytes.len() != expected {
            return Err(Error::InvalidLength {
                expected,
                actual: bytes.len(),
            });
        }
        let output_weights: Vec<i16> = (0..width).map(|i| read_i16(bytes, 2 * i)).collect();
        let output_bias = read_i32(bytes, 2 * width);
        let mut offset = 2 * width + 4;
        let hidden1_weights: Vec<i16> = (0..15 * width)
            .map(|i| read_i16(bytes, offset + 2 * i))
            .collect();
        offset += 15 * width * 2;
        let hidden1_biases = std::array::from_fn(|k| read_i32(bytes, offset + 4 * k));
        offset += 15 * 4;
        let hidden2_weights: Vec<i16> = (0..32 * 30)
            .map(|i| read_i16(bytes, offset + 2 * i))
            .collect();
        offset += 32 * 30 * 2;
        let hidden2_biases = std::array::from_fn(|k| read_i32(bytes, offset + 4 * k));
        offset += 32 * 4;
        let hidden2_output = std::array::from_fn(|k| read_i16(bytes, offset + 2 * k));
        Self::new(
            width,
            &output_weights,
            output_bias,
            &hidden1_weights,
            hidden1_biases,
            &hidden2_weights,
            hidden2_biases,
            hidden2_output,
            shared_mirror,
        )
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::byte_len(self.output_weights.len()));
        for value in &self.output_weights {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.output_bias.to_le_bytes());
        for value in &self.hidden1_weights {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.hidden1_biases {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in &self.hidden2_weights {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.hidden2_biases {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.hidden2_output {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// H=512でも`|z| ≤ 512×127×508 + 260,096`と
    /// `|out| ≤ (512+32)×127×8,192 + 4,161,536`はi32に収まる。
    fn output(&self, us: &[i16], them: &[i16]) -> i32 {
        let width = self.output_weights.len();
        assert_eq!(us.len(), width);
        assert_eq!(them.len(), width);
        let mut buffer = [0_i16; 512];
        let (input, output_weights, hidden1_weights) =
            if let Some(weights) = &self.shared_mirror_weights {
                let f = &mut buffer[..width / 2];
                pairwise_folded(us, &mut f[..width / 4]);
                pairwise_folded(them, &mut f[width / 4..]);
                (
                    &*f,
                    weights.output_weights.as_ref(),
                    weights.hidden1_weights.as_ref(),
                )
            } else {
                let s = &mut buffer[..width];
                pairwise(us, &mut s[..width / 2]);
                pairwise(them, &mut s[width / 2..]);
                (
                    &*s,
                    self.output_weights.as_ref(),
                    self.hidden1_weights.as_ref(),
                )
            };

        let mut u = [0_u8; 30];
        for (k, row) in hidden1_weights.chunks_exact(input.len()).enumerate() {
            let mut z = self.hidden1_biases[k];
            for (&w, &a) in row.iter().zip(input) {
                z += i32::from(w) * i32::from(a);
            }
            u[k] = ((i64::from(z) * i64::from(z)) >> 23).min(127) as u8;
            u[15 + k] = (z >> 8).clamp(0, 127) as u8;
        }
        let mut y = [0_u8; 32];
        for (k, row) in self.hidden2_weights.as_chunks::<30>().0.iter().enumerate() {
            let mut z = self.hidden2_biases[k];
            for (&w, &a) in row.iter().zip(&u) {
                z += i32::from(w) * i32::from(i16::from(a));
            }
            y[k] = (z >> 8).clamp(0, 127) as u8;
        }
        let mut out = self.output_bias;
        for (&w, &a) in output_weights.iter().zip(input) {
            out += i32::from(w) * i32::from(a);
        }
        for (&w, &a) in self.hidden2_output.iter().zip(&y) {
            out += i32::from(w) * i32::from(i16::from(a));
        }
        out
    }
}

/// 1視点の[A, B, Aの鏡映, Bの鏡映]を、2つの要素積へ縮める。
fn pairwise(acc: &[i16], output: &mut [i16]) {
    let quarter = acc.len() / 4;
    for (half, values) in acc
        .chunks_exact(2 * quarter)
        .zip(output.chunks_exact_mut(quarter))
    {
        let (left, right) = half.split_at(quarter);
        for ((value, &a), &b) in values.iter_mut().zip(left).zip(right) {
            *value = pairwise_product(a, b);
        }
    }
}

/// 鏡映共有する2つの要素積を直接足し、各行が再利用するi16入力へ格納する。
/// 個々の積は0〜127なので和は0〜254であり、共有重みとの積和は展開前と等しい。
fn pairwise_folded(acc: &[i16], output: &mut [i16]) {
    let quarter = acc.len() / 4;
    let (left, right) = acc.split_at(2 * quarter);
    let (a, b) = left.split_at(quarter);
    let (c, d) = right.split_at(quarter);
    for ((((value, &a), &b), &c), &d) in output.iter_mut().zip(a).zip(b).zip(c).zip(d) {
        *value = pairwise_product(a, b) + pairwise_product(c, d);
    }
}

/// 0〜255へ切り詰めた2値の積をu16の論理シフトで縮める。
/// 積は最大65,025でu16に収まり、9ビットのシフト後は0〜127なのでi16でも同じ値を保つ。
fn pairwise_product(a: i16, b: i16) -> i16 {
    let a = a.clamp(0, 255) as u16;
    let b = b.clamp(0, 255) as u16;
    ((a * b) >> 9) as i16
}

fn validate_width(width: usize) -> Result<(), Error> {
    if !(4..=512).contains(&width) || !width.is_multiple_of(4) {
        return Err(Error::InvalidWidth {
            actual: width as u32,
        });
    }
    Ok(())
}

fn validate_length(parameter: &'static str, actual: usize, expected: usize) -> Result<(), Error> {
    if actual != expected {
        return Err(Error::InvalidParameterLength {
            parameter,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_mirror<T: PartialEq>(
    parameter: &'static str,
    weights: &[T],
    width: usize,
) -> Result<(), Error> {
    let half = width / 2;
    for (block, row) in weights.chunks_exact(half).enumerate() {
        for j in 0..width / 4 {
            if row[j] != row[j + width / 4] {
                return Err(Error::MirrorMismatch {
                    parameter,
                    index: block * half + j,
                });
            }
        }
    }
    Ok(())
}

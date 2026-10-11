//! 両視点の非線形累算値を整数の出力へ変換する。

use super::Error;
use super::format::{read_i16, read_i32, validate_range};

/// 出力区画の復号と、両視点の非線形累算値からの読み出し。
pub trait Head: Sized {
    /// 構造記述文字列での名前。
    const NAME: &'static str;
    /// 幅Hに対応する出力区画のバイト数。
    fn byte_len(width: usize) -> usize;
    /// 出力区画の長さ、範囲および必要な鏡映共有を検証して復号する。
    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error>;
    /// 手番側と相手側の非線形累算値から、尺度変換前の出力を計算する。
    fn output(&self, us: &[i32], them: &[i32]) -> i32;
}

/// SCReLU活性化後の両視点と出力重みの直接内積。
pub struct DirectScrelu {
    pub(super) weights: Box<[i16]>,
    bias: i32,
}

impl Head for DirectScrelu {
    const NAME: &'static str = "direct-screlu";

    fn byte_len(width: usize) -> usize {
        4 * width + 4
    }

    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error> {
        if !(2..=512).contains(&width) || !width.is_multiple_of(2) {
            return Err(Error::InvalidWidth {
                actual: width as u32,
            });
        }
        if bytes.len() != Self::byte_len(width) {
            return Err(Error::InvalidLength {
                expected: Self::byte_len(width),
                actual: bytes.len(),
            });
        }
        let weights: Box<[i16]> = bytes[..width * 4]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| read_i16(b, 0))
            .collect();
        let bias = read_i32(bytes, width * 4);
        validate_range(
            "output_weights",
            weights.iter().copied().map(i32::from),
            8_192,
        )?;
        validate_range("output_bias", [bias], 4_161_536)?;
        if shared_mirror {
            for perspective in 0..2 {
                for j in 0..width {
                    if weights[perspective * width + j]
                        != weights[perspective * width + (j + width / 2) % width]
                    {
                        return Err(Error::MirrorMismatch {
                            parameter: "output_weights",
                            index: perspective * width + j,
                        });
                    }
                }
            }
        }
        Ok(Self { weights, bias })
    }

    fn output(&self, us: &[i32], them: &[i32]) -> i32 {
        let width = self.weights.len() / 2;
        let mut out = self.bias;
        for (&value, &weight) in us.iter().zip(&self.weights[..width]) {
            let c = value.clamp(0, 127);
            let a = (c * c) >> 7;
            out += i32::from(weight) * a;
        }
        for (&value, &weight) in them.iter().zip(&self.weights[width..]) {
            let c = value.clamp(0, 127);
            let a = (c * c) >> 7;
            out += i32::from(weight) * a;
        }
        out
    }
}

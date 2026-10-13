//! 両視点の非線形累算値を整数の出力へ変換する。

use super::Error;
use super::format::{read_i16, read_i32, validate_range};

/// 出力区画の復号と、両視点の非線形累算値からの読み出し。
/// 新しい出力部はこのトレイトの実装と構造記述の`head=`の名前を追加して定義し、入力特徴集合と第1層は変更しない。
pub trait Head: Sized {
    /// 構造記述文字列での名前。
    const NAME: &'static str;
    /// 幅Hに対応する出力区画のバイト数。
    fn byte_len(width: usize) -> usize;
    /// 出力区画の長さと範囲を検証し、`shared_mirror`ならjとj+H/2の共有も検査する。
    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error>;
    /// 手番側と相手側の非線形累算値から、尺度変換前の出力を計算する。
    fn output(&self, us: &[i32], them: &[i32]) -> i32;
}

/// SCReLU活性化後の両視点と出力重みの直接内積。
/// 出力重みは手番側視点のH個に相手側視点のH個が続くi16で、尺度4,096、絶対値8,192以下である。
/// 出力バイアスはi32で、尺度127×4,096、絶対値4,161,536以下である。
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

    /// 各累算値を`c = clamp(acc, 0, 127)`、`a = (c×c) >> 7`で活性化する。
    /// 127で割る代わりに128で切り捨てる近似で、評価1回あたり2H回の除算を省く。
    /// aは0〜126で、出力の絶対値は`2H×126×8,192 + 4,161,536`以下となり、H=512でもi32に収まる。
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

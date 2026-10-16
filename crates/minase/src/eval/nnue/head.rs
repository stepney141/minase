//! 両視点の非線形累算値を整数の出力へ変換する。
//! 隠れ層の重みW1は尺度64で絶対値127以下、バイアスb1は尺度127×64で65,024以下とする。
//! 隠れ層の出力重みw2は尺度4,096で絶対値8,192以下とする。
//! P型では、W1の各行も各視点内の前半と後半で共有する。b1とw2には共有を課さない。

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
    /// 検査済みの整数配列をリトルエンディアンで符号化する。
    fn to_bytes(&self) -> Vec<u8>;
    /// 手番側と相手側の非線形累算値から、尺度変換前の出力を計算する。
    /// 呼び手は線形経路と同じ係数qを渡し、0..=90を保証する。
    fn output(&self, us: &[i32], them: &[i32], q: i32) -> i32;
}

/// SCReLU活性化後の両視点と出力重みの直接内積。
/// 出力重みは手番側視点のH個に相手側視点のH個が続くi16で、尺度4,096、絶対値8,192以下である。
/// 出力バイアスはi32で、尺度127×4,096、絶対値4,161,536以下である。
pub struct DirectScrelu {
    weights: Box<[i16]>,
    bias: i32,
}

impl Head for DirectScrelu {
    const NAME: &'static str = "direct-screlu";

    fn byte_len(width: usize) -> usize {
        4 * width + 4
    }

    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error> {
        validate_bytes::<Self>(bytes, width)?;
        let weights: Box<[i16]> = bytes[..width * 4]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| read_i16(b, 0))
            .collect();
        let bias = read_i32(bytes, width * 4);
        Self::new(width, &weights, bias, shared_mirror)
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::byte_len(self.weights.len() / 2));
        for value in &self.weights {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&self.bias.to_le_bytes());
        bytes
    }

    /// 各累算値を`c = clamp(acc, 0, 127)`、`a = (c×c) >> 7`で活性化する。
    /// 127で割る代わりに128で切り捨てる近似で、評価1回あたり2H回の除算を省く。
    /// aは0〜126で、出力の絶対値は`2H×126×8,192 + 4,161,536`以下となり、H=512でもi32に収まる。
    fn output(&self, us: &[i32], them: &[i32], _q: i32) -> i32 {
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

impl DirectScrelu {
    /// 幅、配列長、値域、鏡映共有の順に検査する。
    pub fn new(
        width: usize,
        weights: &[i16],
        bias: i32,
        shared_mirror: bool,
    ) -> Result<Self, Error> {
        validate_width(width)?;
        validate_length("output_weights", weights.len(), 2 * width)?;
        validate_range(
            "output_weights",
            weights.iter().copied().map(i32::from),
            8_192,
        )?;
        validate_range("output_bias", [bias], 4_161_536)?;
        if shared_mirror {
            validate_mirror("output_weights", weights, width)?;
        }
        Ok(Self {
            weights: weights.into(),
            bias,
        })
    }

    fn dot(&self, activated: &[u8]) -> i32 {
        let mut out = self.bias;
        for (&w, &a) in self.weights.iter().zip(activated) {
            out += i32::from(w) * i32::from(i16::from(a));
        }
        out
    }
}

fn validate_width(width: usize) -> Result<(), Error> {
    if !(2..=512).contains(&width) || !width.is_multiple_of(2) {
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

fn validate_bytes<H: Head>(bytes: &[u8], width: usize) -> Result<(), Error> {
    validate_width(width)?;
    if bytes.len() != H::byte_len(width) {
        return Err(Error::InvalidLength {
            expected: H::byte_len(width),
            actual: bytes.len(),
        });
    }
    Ok(())
}

fn validate_mirror<T: PartialEq>(
    parameter: &'static str,
    weights: &[T],
    width: usize,
) -> Result<(), Error> {
    for (row_index, row) in weights.chunks_exact(width).enumerate() {
        for j in 0..width / 2 {
            if row[j] != row[j + width / 2] {
                return Err(Error::MirrorMismatch {
                    parameter,
                    index: row_index * width + j,
                });
            }
        }
    }
    Ok(())
}

fn activate<'a>(us: &[i32], them: &[i32], buffer: &'a mut [u8; 1024]) -> &'a [u8] {
    let values = &mut buffer[..us.len() + them.len()];
    for (a, &value) in values[..us.len()].iter_mut().zip(us) {
        let c = value.clamp(0, 127);
        *a = ((c * c) >> 7) as u8;
    }
    for (a, &value) in values[us.len()..].iter_mut().zip(them) {
        let c = value.clamp(0, 127);
        *a = ((c * c) >> 7) as u8;
    }
    values
}

fn interpolate(open: i32, end: i32, q: i32) -> i32 {
    debug_assert!((0..=90).contains(&q));
    (i64::from(q) * i64::from(open) + i64::from(90 - q) * i64::from(end)).div_euclid(90) as i32
}

/// 尺度64の重みと尺度127×64のバイアスを持つ、幅16の隠れ層。
struct Hidden16 {
    weights: Box<[i8]>,
    biases: [i32; 16],
}

impl Hidden16 {
    fn new(width: usize, weights: &[i8], biases: [i32; 16]) -> Result<Self, Error> {
        validate_length("hidden_weights", weights.len(), 16 * 2 * width)?;
        validate_range(
            "hidden_weights",
            weights.iter().copied().map(i32::from),
            127,
        )?;
        validate_range("hidden_biases", biases, 65_024)?;
        Ok(Self {
            weights: weights.into(),
            biases,
        })
    }

    fn output(&self, activated: &[u8]) -> [u8; 16] {
        let mut output = [0; 16];
        for (k, value) in output.iter_mut().enumerate() {
            let row = &self.weights[k * activated.len()..(k + 1) * activated.len()];
            let mut z = self.biases[k];
            for (&w, &a) in row.iter().zip(activated) {
                z += i32::from(i16::from(w)) * i32::from(i16::from(a));
            }
            *value = (z >> 6).clamp(0, 127) as u8;
        }
        output
    }

    fn append_bytes(&self, bytes: &mut Vec<u8>) {
        bytes.extend(self.weights.iter().map(|&w| w as u8));
        for bias in self.biases {
            bytes.extend_from_slice(&bias.to_le_bytes());
        }
    }
}

/// `direct-screlu-phase2`の検査済み出力区画。端点は序盤、終盤の順に並ぶ。
pub struct DirectScreluPhase2 {
    endpoints: [DirectScrelu; 2],
}
impl DirectScreluPhase2 {
    /// 整数配列の長さ、全値域、鏡映共有の順に検査する。
    /// 出力重みの尺度は4,096であり、両端点の値域は直接出力部と同じである。
    pub fn new(
        width: usize,
        output_weights: [&[i16]; 2],
        output_biases: [i32; 2],
        shared_mirror: bool,
    ) -> Result<Self, Error> {
        validate_width(width)?;
        for weights in output_weights {
            validate_length("output_weights", weights.len(), 2 * width)?;
        }
        let endpoints = [
            DirectScrelu::new(width, output_weights[0], output_biases[0], false)?,
            DirectScrelu::new(width, output_weights[1], output_biases[1], false)?,
        ];
        if shared_mirror {
            for endpoint in &endpoints {
                validate_mirror("output_weights", &endpoint.weights, width)?;
            }
        }
        Ok(Self { endpoints })
    }
}
impl Head for DirectScreluPhase2 {
    const NAME: &'static str = "direct-screlu-phase2";
    fn byte_len(width: usize) -> usize {
        2 * (4 * width + 4)
    }
    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error> {
        validate_bytes::<Self>(bytes, width)?;
        let weights: [Vec<i16>; 2] = std::array::from_fn(|e| {
            (0..2 * width)
                .map(|i| read_i16(bytes, e * (4 * width + 4) + 2 * i))
                .collect()
        });
        let biases = std::array::from_fn(|e| read_i32(bytes, e * (4 * width + 4) + 4 * width));
        Self::new(
            width,
            weights.each_ref().map(Vec::as_slice),
            biases,
            shared_mirror,
        )
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for endpoint in &self.endpoints {
            bytes.extend(endpoint.to_bytes());
        }
        bytes
    }
    fn output(&self, us: &[i32], them: &[i32], q: i32) -> i32 {
        let mut buffer = [0; 1024];
        let activated = activate(us, them, &mut buffer);
        interpolate(
            self.endpoints[0].dot(activated),
            self.endpoints[1].dot(activated),
            q,
        )
    }
}

/// `screlu-hidden16`の検査済み出力区画。直接内積に幅16の隠れ層の寄与を加算する。
pub struct ScreluHidden16 {
    endpoints: [DirectScrelu; 1],
    hidden: Hidden16,
    hidden_output: [[i16; 16]; 1],
}
impl ScreluHidden16 {
    /// 整数配列の長さ、全値域、鏡映共有の順に検査する。
    /// 出力重みの尺度は4,096、隠れ層の重みの尺度は64である。
    pub fn new(
        width: usize,
        output_weights: [&[i16]; 1],
        output_biases: [i32; 1],
        hidden_weights: &[i8],
        hidden_biases: [i32; 16],
        hidden_output: [[i16; 16]; 1],
        shared_mirror: bool,
    ) -> Result<Self, Error> {
        validate_width(width)?;
        for weights in output_weights {
            validate_length("output_weights", weights.len(), 2 * width)?;
        }
        validate_length("hidden_weights", hidden_weights.len(), 32 * width)?;
        let endpoints = [DirectScrelu::new(
            width,
            output_weights[0],
            output_biases[0],
            false,
        )?];
        let hidden = Hidden16::new(width, hidden_weights, hidden_biases)?;
        validate_range(
            "hidden_output",
            hidden_output.iter().flatten().copied().map(i32::from),
            8_192,
        )?;
        if shared_mirror {
            for endpoint in &endpoints {
                validate_mirror("output_weights", &endpoint.weights, width)?;
            }
            validate_mirror("hidden_weights", &hidden.weights, width)?;
        }
        Ok(Self {
            endpoints,
            hidden,
            hidden_output,
        })
    }
}
impl Head for ScreluHidden16 {
    const NAME: &'static str = "screlu-hidden16";
    fn byte_len(width: usize) -> usize {
        4 * width + 4 + 32 * width + 64 + 32
    }
    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error> {
        validate_bytes::<Self>(bytes, width)?;
        let weights: [Vec<i16>; 1] = std::array::from_fn(|e| {
            (0..2 * width)
                .map(|i| read_i16(bytes, e * (4 * width + 4) + 2 * i))
                .collect()
        });
        let biases = std::array::from_fn(|e| read_i32(bytes, e * (4 * width + 4) + 4 * width));
        let offset = 4 * width + 4;
        let hidden_weights: Vec<i8> = bytes[offset..offset + 32 * width]
            .iter()
            .map(|&b| b as i8)
            .collect();
        let offset = offset + 32 * width;
        let hidden_biases = std::array::from_fn(|k| read_i32(bytes, offset + 4 * k));
        let hidden_output = std::array::from_fn(|e| {
            std::array::from_fn(|k| read_i16(bytes, offset + 64 + e * 32 + 2 * k))
        });
        Self::new(
            width,
            weights.each_ref().map(Vec::as_slice),
            biases,
            &hidden_weights,
            hidden_biases,
            hidden_output,
            shared_mirror,
        )
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for endpoint in &self.endpoints {
            bytes.extend(endpoint.to_bytes());
        }
        self.hidden.append_bytes(&mut bytes);
        for row in self.hidden_output {
            for value in row {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }
    fn output(&self, us: &[i32], them: &[i32], _q: i32) -> i32 {
        let mut buffer = [0; 1024];
        let activated = activate(us, them, &mut buffer);
        let h = self.hidden.output(activated);
        let mut out = self.endpoints[0].dot(activated);
        for (&w, &a) in self.hidden_output[0].iter().zip(&h) {
            out += i32::from(w) * i32::from(i16::from(a));
        }
        out
    }
}

/// `screlu-hidden16-phase2`の検査済み出力区画。端点は序盤、終盤の順に並ぶ。
pub struct ScreluHidden16Phase2 {
    endpoints: [DirectScrelu; 2],
    hidden: Hidden16,
    hidden_output: [[i16; 16]; 2],
}
impl ScreluHidden16Phase2 {
    /// 整数配列の長さ、全値域、鏡映共有の順に検査する。
    /// 出力重みの尺度は4,096、隠れ層の重みの尺度は64である。
    pub fn new(
        width: usize,
        output_weights: [&[i16]; 2],
        output_biases: [i32; 2],
        hidden_weights: &[i8],
        hidden_biases: [i32; 16],
        hidden_output: [[i16; 16]; 2],
        shared_mirror: bool,
    ) -> Result<Self, Error> {
        validate_width(width)?;
        for weights in output_weights {
            validate_length("output_weights", weights.len(), 2 * width)?;
        }
        validate_length("hidden_weights", hidden_weights.len(), 32 * width)?;
        let endpoints = [
            DirectScrelu::new(width, output_weights[0], output_biases[0], false)?,
            DirectScrelu::new(width, output_weights[1], output_biases[1], false)?,
        ];
        let hidden = Hidden16::new(width, hidden_weights, hidden_biases)?;
        validate_range(
            "hidden_output",
            hidden_output.iter().flatten().copied().map(i32::from),
            8_192,
        )?;
        if shared_mirror {
            for endpoint in &endpoints {
                validate_mirror("output_weights", &endpoint.weights, width)?;
            }
            validate_mirror("hidden_weights", &hidden.weights, width)?;
        }
        Ok(Self {
            endpoints,
            hidden,
            hidden_output,
        })
    }
}
impl Head for ScreluHidden16Phase2 {
    const NAME: &'static str = "screlu-hidden16-phase2";
    fn byte_len(width: usize) -> usize {
        2 * (4 * width + 4) + 32 * width + 64 + 2 * 32
    }
    fn decode(bytes: &[u8], width: usize, shared_mirror: bool) -> Result<Self, Error> {
        validate_bytes::<Self>(bytes, width)?;
        let weights: [Vec<i16>; 2] = std::array::from_fn(|e| {
            (0..2 * width)
                .map(|i| read_i16(bytes, e * (4 * width + 4) + 2 * i))
                .collect()
        });
        let biases = std::array::from_fn(|e| read_i32(bytes, e * (4 * width + 4) + 4 * width));
        let offset = 2 * (4 * width + 4);
        let hidden_weights: Vec<i8> = bytes[offset..offset + 32 * width]
            .iter()
            .map(|&b| b as i8)
            .collect();
        let offset = offset + 32 * width;
        let hidden_biases = std::array::from_fn(|k| read_i32(bytes, offset + 4 * k));
        let hidden_output = std::array::from_fn(|e| {
            std::array::from_fn(|k| read_i16(bytes, offset + 64 + e * 32 + 2 * k))
        });
        Self::new(
            width,
            weights.each_ref().map(Vec::as_slice),
            biases,
            &hidden_weights,
            hidden_biases,
            hidden_output,
            shared_mirror,
        )
    }
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for endpoint in &self.endpoints {
            bytes.extend(endpoint.to_bytes());
        }
        self.hidden.append_bytes(&mut bytes);
        for row in self.hidden_output {
            for value in row {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }
    fn output(&self, us: &[i32], them: &[i32], q: i32) -> i32 {
        let mut buffer = [0; 1024];
        let activated = activate(us, them, &mut buffer);
        let h = self.hidden.output(activated);
        let mut outputs = [0; 2];
        for (e, out) in outputs.iter_mut().enumerate() {
            *out = self.endpoints[e].dot(activated);
            for (&w, &a) in self.hidden_output[e].iter().zip(&h) {
                *out += i32::from(w) * i32::from(i16::from(a));
            }
        }
        interpolate(outputs[0], outputs[1], q)
    }
}

/// 直接出力の区画から、全位相で同じ評価を返すテスト用区画を作る。
#[cfg(test)]
pub(crate) fn from_direct<H: Head>(
    bytes: &[u8],
    width: usize,
    shared_mirror: bool,
) -> Result<H, Error> {
    let direct = DirectScrelu::decode(bytes, width, shared_mirror)?;
    let mut bytes = direct.to_bytes();
    match H::NAME {
        DirectScrelu::NAME => {}
        DirectScreluPhase2::NAME => bytes.extend(direct.to_bytes()),
        ScreluHidden16::NAME => bytes.resize(H::byte_len(width), 0),
        ScreluHidden16Phase2::NAME => {
            bytes.extend(direct.to_bytes());
            bytes.resize(H::byte_len(width), 0);
        }
        _ => {
            return Err(Error::UnexpectedArchitecture {
                actual: H::NAME.into(),
            });
        }
    }
    H::decode(&bytes, width, shared_mirror)
}

use super::{Error, Result};
use minase::eval::{nnue::input::FeatureSet, piece_values::piece_value_of_state};
const FEATURE_COUNT: usize = FeatureSet::A.spec().dimensions();
use std::ops::{Index, IndexMut};

/// A型LayerStackの第1層の幅。
pub const H: usize = 256;
/// 左右鏡映を共有した6,840行と、常に0の埋め草1行。
pub const ROWS: usize = super::input::CANONICAL_ROWS;
pub(super) const HALF: usize = H / 2;
pub(super) const ALPHA: f64 = 127.0 / 128.0;

/// 教師の尺度K_tと出力の除数m。K_oは8128/mである。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    teacher: f64,
    divisor: i32,
}
impl Scale {
    /// K_tとround(8128/K_t)の定義域を検査する。
    pub fn new(teacher: f64) -> Result<Self> {
        if !teacher.is_finite() || teacher <= 0.0 {
            return Err(Error::TeacherScale(teacher));
        }
        let divisor = (8128.0 / teacher).round();
        if !(1.0..=f64::from(i32::MAX)).contains(&divisor) {
            return Err(Error::OutputScale(divisor));
        }
        Ok(Self {
            teacher,
            divisor: divisor as i32,
        })
    }
    /// 教師の1ロジットあたりのセンチポーン。
    pub fn teacher(self) -> f64 {
        self.teacher
    }
    /// 整数出力をセンチポーンに換算する除数。
    pub fn divisor(self) -> i32 {
        self.divisor
    }
    /// 実数出力の1単位あたりのセンチポーン。
    pub fn output(self) -> f64 {
        8128.0 / f64::from(self.divisor)
    }
    pub(super) fn ratio(self) -> f64 {
        self.output() / self.teacher
    }
}

/// 行優先の自由パラメータ。宣言順が初期化の乱数消費順と保存順を定める。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Parameter {
    /// 正準行の第1層の重み[6841,256]。
    FtWeights,
    /// 前後半で共有するバイアス[128]。
    FtBiases,
    /// 正準行の駒升評価[6841]。
    Psqt,
    /// 区分0の第1後段層の重み[16,256]。
    L1Weights,
    /// 区分0の第1後段層のバイアス[16]。
    L1Biases,
    /// 共有する第1後段層の重み[16,256]。
    L1SharedWeights,
    /// 共有する第1後段層のバイアス[16]。
    L1SharedBiases,
    /// 第2後段層の重み[32,30]。
    L2Weights,
    /// 第2後段層のバイアス[32]。
    L2Biases,
    /// 出力層の重み[32]。
    L3Weights,
    /// 出力層のバイアス[1]。
    L3Bias,
}
impl Parameter {
    /// 宣言順の全配列。
    pub const ALL: [Self; 11] = [
        Self::FtWeights,
        Self::FtBiases,
        Self::Psqt,
        Self::L1Weights,
        Self::L1Biases,
        Self::L1SharedWeights,
        Self::L1SharedBiases,
        Self::L2Weights,
        Self::L2Biases,
        Self::L3Weights,
        Self::L3Bias,
    ];
    /// 保存名およびエラーに表示する名前。
    pub fn name(self) -> &'static str {
        match self {
            Self::FtWeights => "ft_weights",
            Self::FtBiases => "ft_biases",
            Self::Psqt => "psqt",
            Self::L1Weights => "l1_weights",
            Self::L1Biases => "l1_biases",
            Self::L1SharedWeights => "l1_shared_weights",
            Self::L1SharedBiases => "l1_shared_biases",
            Self::L2Weights => "l2_weights",
            Self::L2Biases => "l2_biases",
            Self::L3Weights => "l3_weights",
            Self::L3Bias => "l3_bias",
        }
    }
    /// 詰め物の行を含む形状。
    pub fn dimensions(self) -> &'static [usize] {
        match self {
            Self::FtWeights => &[ROWS, H],
            Self::FtBiases => &[HALF],
            Self::Psqt => &[ROWS],
            Self::L1Weights | Self::L1SharedWeights => &[16, H],
            Self::L1Biases | Self::L1SharedBiases => &[16],
            Self::L2Weights => &[32, 30],
            Self::L2Biases | Self::L3Weights => &[32],
            Self::L3Bias => &[1],
        }
    }
    pub(super) fn count(self) -> usize {
        self.dimensions().iter().product()
    }
    pub(super) fn active_len(self) -> usize {
        match self {
            Self::FtWeights => (ROWS - 1) * H,
            Self::Psqt => ROWS - 1,
            _ => self.count(),
        }
    }
    /// 更新後の制限。第1層を±224/127に制限すると整数は絶対値224以下となり、
    /// 最大145特徴とバイアスの累算値は145×224＋224＝32,704でi16に収まる。
    /// L1と共有L1の重みは各±127/128に制限し、和を64倍して丸めた値を±127以内にする。
    /// tataraは各配列を±127/64に制限し、書き出し時にも和を8ビットへ切り詰めるが、
    /// minaseは書き出し時に切り詰めないため、学習時の重みの制限を半分にする。
    /// 両バイアスは±127/64のままとする。CPU参照とGPUはこの制限を共有する。
    pub(super) fn limit(self) -> Option<f32> {
        match self {
            Self::FtWeights | Self::FtBiases => Some(224.0 / 127.0),
            Self::L1Weights | Self::L1SharedWeights => Some(127.0 / 128.0),
            Self::Psqt | Self::L3Bias => None,
            _ => Some(127.0 / 64.0),
        }
    }
}

/// 固定形状のf32配列。パラメータ、勾配、および両モーメントで同じ配置を使う。
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    arrays: [Vec<f32>; 11],
}
impl Parameters {
    /// 全成分を0として確保する。
    pub fn zeros() -> Self {
        Self {
            arrays: Parameter::ALL.map(|p| vec![0.0; p.count()]),
        }
    }
    /// SplitMix64の上位53ビットを[0,1)へ写し、宣言順、行優先で重みの乱数を消費する。
    /// バイアス、PSQT、および埋め草は乱数を消費しない。第1層は±sqrt(1/13680)、
    /// 後段の重みは±0.01とし、PSQTは王駒と先獅子を除く符号付きv0駒価値/K_oとする。
    pub fn initialize(seed: u64, scale: Scale) -> Self {
        let mut result = Self::zeros();
        let mut rng = Uniform(seed);
        for p in Parameter::ALL {
            let half_width = match p {
                Parameter::FtWeights => (1.0 / FEATURE_COUNT as f64).sqrt(),
                Parameter::L1Weights
                | Parameter::L1SharedWeights
                | Parameter::L2Weights
                | Parameter::L3Weights => 0.01,
                _ => continue,
            };
            for value in &mut result[p][..p.active_len()] {
                *value = (rng.next() * half_width) as f32;
            }
        }
        for (row, value) in result[Parameter::Psqt][..94 * 72].iter_mut().enumerate() {
            let state = row / 72 % 47;
            if ![11, 21].contains(&state) {
                let sign = if row / (47 * 72) == 0 { 1.0 } else { -1.0 };
                *value = (sign * f64::from(piece_value_of_state(state)) / scale.output()) as f32;
            }
        }
        result
    }
    /// 有限性と埋め草が0であることを検査する。値域制限は最適化器と書き出しが担う。
    pub fn validate(&self) -> Result<()> {
        for p in Parameter::ALL {
            for (i, v) in self[p].iter().enumerate() {
                if !v.is_finite() {
                    return Err(Error::NonFinite(p.name(), i));
                }
            }
            if self[p][p.active_len()..].iter().any(|v| v.to_bits() != 0) {
                return Err(Error::Invalid("nonzero padding"));
            }
        }
        Ok(())
    }
}
impl Index<Parameter> for Parameters {
    type Output = [f32];
    fn index(&self, p: Parameter) -> &[f32] {
        &self.arrays[p as usize]
    }
}
impl IndexMut<Parameter> for Parameters {
    fn index_mut(&mut self, p: Parameter) -> &mut [f32] {
        &mut self.arrays[p as usize]
    }
}

pub(super) fn canonical(f: usize) -> usize {
    f / 144 * 72 + f % 144 / 12 * 6 + (f % 12).min(11 - f % 12)
}
pub(super) fn unit(f: usize, j: usize) -> usize {
    (j + HALF * usize::from(f % 12 >= 6)) % H
}

pub(super) struct Uniform(pub u64);
impl Uniform {
    pub(super) fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / 9_007_199_254_740_992.0 * 2.0 - 1.0
    }
}

use super::{Error, Result};
use minase::eval::nnue::input::{FeatureSet, FeatureSetSpec};
use std::ops::{Index, IndexMut};
/// CLIで使う第1層の幅。
pub const H: usize = 256;
/// エンジンと共有するHalfKP_vmの仕様。
pub const SPEC: FeatureSetSpec = FeatureSet::HalfKpVm.spec();
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

/// 初期化と保存の順序を定める自由パラメータ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Parameter {
    /// 実の第1層の重み[D,H]。
    FtWeights,
    /// 区分を共有する仮想行[K,H]。
    FtVirtual,
    /// 第1層のバイアス[H]。
    FtBiases,
    /// L1の重み[32,2H]。
    L1Weights,
    /// L1のバイアス[32]。
    L1Biases,
    /// L2の重み[32,32]。
    L2Weights,
    /// L2のバイアス[32]。
    L2Biases,
    /// 出力の重み[32]。
    L3Weights,
    /// 出力のバイアス[1]。
    L3Bias,
}
impl Parameter {
    /// 宣言順の全配列。
    pub const ALL: [Self; 9] = [
        Self::FtWeights,
        Self::FtVirtual,
        Self::FtBiases,
        Self::L1Weights,
        Self::L1Biases,
        Self::L2Weights,
        Self::L2Biases,
        Self::L3Weights,
        Self::L3Bias,
    ];
    /// NPZの配列名。
    pub fn name(self) -> &'static str {
        match self {
            Self::FtWeights => "ft_weights",
            Self::FtVirtual => "ft_virtual",
            Self::FtBiases => "ft_biases",
            Self::L1Weights => "l1_weights",
            Self::L1Biases => "l1_biases",
            Self::L2Weights => "l2_weights",
            Self::L2Biases => "l2_biases",
            Self::L3Weights => "l3_weights",
            Self::L3Bias => "l3_bias",
        }
    }
    /// 実行時の幅から求める行優先の形状。
    pub fn dimensions(self, width: usize) -> Vec<usize> {
        match self {
            Self::FtWeights => vec![SPEC.dimensions(), width],
            Self::FtVirtual => vec![
                if SPEC.factorized() {
                    SPEC.piece_inputs()
                } else {
                    0
                },
                width,
            ],
            Self::FtBiases => vec![width],
            Self::L1Weights => vec![32, 2 * width],
            Self::L2Weights => vec![32, 32],
            Self::L3Bias => vec![1],
            _ => vec![32],
        }
    }
    pub(super) fn count(self, width: usize) -> usize {
        self.dimensions(width).iter().product()
    }
    /// WとVを各±112/127、bを±224/127に制限する。
    /// 畳み込んだ整数の絶対値は224以下となり、最大145特徴の累算値は
    /// 145×224＋224＝32,704≤32,767でi16に収まる。
    pub(super) fn limit(self) -> Option<f32> {
        match self {
            Self::FtWeights | Self::FtVirtual => Some(112.0 / 127.0),
            Self::FtBiases => Some(224.0 / 127.0),
            Self::L3Bias => None,
            _ => Some(127.0 / 64.0),
        }
    }
}
/// 同じ幅の重み、勾配、モーメントに使う配列群。
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    width: usize,
    arrays: [Vec<f32>; 9],
}
impl Parameters {
    /// エンジンの幅の契約を検査して全成分を0で確保する。
    pub fn zeros(width: usize) -> Result<Self> {
        check_width(width)?;
        Ok(Self {
            width,
            arrays: Parameter::ALL.map(|p| vec![0.0; p.count(width)]),
        })
    }
    /// 第1層の幅を返す。
    pub fn width(&self) -> usize {
        self.width
    }
    /// 全ての重みとバイアスを宣言順、行優先で±0.01の一様乱数とする。
    /// Vは0のままとし乱数を消費しないため、初期書き出しは特徴分解なしと一致する。
    pub fn initialize(seed: u64, width: usize) -> Result<Self> {
        let mut result = Self::zeros(width)?;
        let mut rng = Uniform(seed);
        for p in Parameter::ALL {
            if p != Parameter::FtVirtual {
                for v in &mut result[p] {
                    *v = (rng.next() * 0.01) as f32;
                }
            }
        }
        Ok(result)
    }
    /// 配列の有限性を検査する。形状は構築後に変更できない。
    pub fn validate(&self) -> Result<()> {
        for p in Parameter::ALL {
            for (i, v) in self[p].iter().enumerate() {
                if !v.is_finite() {
                    return Err(Error::NonFinite(p.name(), i));
                }
            }
        }
        Ok(())
    }
}
pub(super) fn check_width(width: usize) -> Result<()> {
    if !(2..=1024).contains(&width) || !width.is_multiple_of(2) {
        return Err(Error::Invalid("Simple width must be even and in 2..=1024"));
    }
    Ok(())
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

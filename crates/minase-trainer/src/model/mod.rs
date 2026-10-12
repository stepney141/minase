//! f32パラメータ、鏡映共有、初期化、保存範囲への射影、および保存形式。
//!
//! 配列は行優先。`Parameter`の順序はチェックポイントの保存順を定める。

mod checkpoint;
pub(crate) mod linear;
mod npz;
mod quantize;

pub use checkpoint::{BestEpoch, Checkpoint, Progress};
pub use npz::{encode_pst_npz, load_initial};
pub use quantize::{Quantized, quantize};

use crate::data::{CANONICAL_ROWS, InputFeatureSet, Observations};
use minase::eval::features::{FEATURE_COUNT, mirror_feature};
use std::{
    fmt,
    ops::{Index, IndexMut},
};

/// モデルと参照計算の失敗。
#[derive(Debug)]
pub enum Error {
    /// ファイルの入出力に失敗した。
    Io(std::io::Error),
    /// サポートしていない、または壊れた保存形式。
    Format(&'static str),
    /// 幅、配列の形、マスク、または計算引数の違反。
    Invalid(&'static str),
    /// NaNまたは無限大のパラメータ。
    NonFinite(Parameter),
    /// 量子化後の値が保存範囲を超えた。
    Quantization(Parameter),
    /// MNUEの検査に失敗した。
    Mnue(minase::eval::nnue::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Format(s) => write!(f, "unsupported or invalid model format: {s}"),
            Self::Invalid(s) => write!(f, "invalid model input: {s}"),
            Self::NonFinite(p) => write!(f, "non-finite parameter: {}", p.name()),
            Self::Quantization(p) => write!(f, "quantization overflow: {}", p.name()),
            Self::Mnue(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Mnue(e) => Some(e),
            _ => None,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
/// モデルと参照計算の結果。
pub type Result<T> = std::result::Result<T, Error>;

/// パラメータ配列の種類。P型の仮想行は長さ0。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Parameter {
    /// 線形正準行[6841, 2]、センチポーン。
    Linear,
    /// P型[6841, H]、HalfKP[109441, H]。
    FirstLayer,
    /// HalfKPの仮想行[13681, H]。
    Virtual,
    /// P型[H/2]、HalfKP[H]。
    Biases,
    /// P型[2, H/2]、HalfKP[2, H]。
    OutputWeights,
    /// ロジット単位の出力バイアス[1]。
    OutputBias,
}
impl Parameter {
    /// 保存順の全パラメータ。
    pub const ALL: [Self; 6] = [
        Self::Linear,
        Self::FirstLayer,
        Self::Virtual,
        Self::Biases,
        Self::OutputWeights,
        Self::OutputBias,
    ];
    /// 検査メッセージに使うパラメータ名。
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear.weight",
            Self::FirstLayer => "ft.embedding.weight",
            Self::Virtual => "ft.virtual.weight",
            Self::Biases => "head.biases",
            Self::OutputWeights => "head.output_weights",
            Self::OutputBias => "head.output_bias",
        }
    }
}

/// 検査済みの入力特徴集合、幅、出力用K。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    feature_set: InputFeatureSet,
    width: usize,
    k: f32,
}
impl Shape {
    /// 幅は2..=512の偶数、Kは有限かつ0より大きく100000以下。
    pub fn new(feature_set: InputFeatureSet, width: usize, k: f32) -> Result<Self> {
        if !(2..=512).contains(&width) || !width.is_multiple_of(2) {
            return Err(Error::Invalid("width must be even and in 2..512"));
        }
        if !k.is_finite() || k <= 0.0 || k > 100_000.0 {
            return Err(Error::Invalid("K must be in (0, 100000]"));
        }
        Ok(Self {
            feature_set,
            width,
            k,
        })
    }
    /// 入力特徴集合。
    pub fn feature_set(self) -> InputFeatureSet {
        self.feature_set
    }
    /// 展開後の幅H。
    pub fn width(self) -> usize {
        self.width
    }
    /// 出力用K。
    pub fn k(self) -> f32 {
        self.k
    }
    /// バイアスと視点ごとの出力重みの自由パラメータ数。
    pub fn units(self) -> usize {
        if self.feature_set == InputFeatureSet::P {
            self.width / 2
        } else {
            self.width
        }
    }
    /// 第1層の行数。埋め草を含む。
    pub fn rows(self) -> usize {
        if self.feature_set == InputFeatureSet::P {
            CANONICAL_ROWS
        } else {
            self.feature_set.dimensions() + 1
        }
    }
    /// パラメータ配列の要素数。
    pub fn len(self, p: Parameter) -> usize {
        match p {
            Parameter::Linear => CANONICAL_ROWS * 2,
            Parameter::FirstLayer => self.rows() * self.width,
            Parameter::Virtual => {
                if self.feature_set == InputFeatureSet::P {
                    0
                } else {
                    (FEATURE_COUNT + 1) * self.width
                }
            }
            Parameter::Biases => self.units(),
            Parameter::OutputWeights => 2 * self.units(),
            Parameter::OutputBias => 1,
        }
    }
    /// MNUE版5と同じ構造記述文字列。
    pub fn architecture(self) -> String {
        let name = match self.feature_set {
            InputFeatureSet::P => "P",
            InputFeatureSet::HalfKpHmB8 => "HalfKP-hm-b8",
        };
        format!(
            "features={name};ft={};linear=tapered2;head=direct-screlu",
            self.width
        )
    }
}

/// 形を変えずに読み書きできる全パラメータ。勾配とAdamWの状態も同じ配置。
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    shape: Shape,
    values: [Vec<f32>; 6],
}
impl Parameters {
    /// 全要素を0で確保する。
    pub fn zeros(shape: Shape) -> Self {
        Self {
            shape,
            values: Parameter::ALL.map(|p| vec![0.0; shape.len(p)]),
        }
    }
    /// 配列の形。
    pub fn shape(&self) -> Shape {
        self.shape
    }
    /// 全要素が有限であることを検査する。
    pub fn validate(&self) -> Result<()> {
        for p in Parameter::ALL {
            if self[p].iter().any(|v| !v.is_finite()) {
                return Err(Error::NonFinite(p));
            }
        }
        Ok(())
    }
}
impl Index<Parameter> for Parameters {
    type Output = [f32];
    fn index(&self, p: Parameter) -> &[f32] {
        &self.values[p as usize]
    }
}
impl IndexMut<Parameter> for Parameters {
    fn index_mut(&mut self, p: Parameter) -> &mut [f32] {
        &mut self.values[p as usize]
    }
}

/// 線形・第1層・仮想行の観測マスク。末尾の埋め草は常にfalse。
#[derive(Clone, Debug, PartialEq)]
pub struct Masks {
    canonical: Vec<bool>,
    first_layer: Vec<bool>,
    virtual_rows: Vec<bool>,
}
impl Masks {
    /// フェーズ1aの観測回数から作る。
    pub fn from_observations(set: InputFeatureSet, observations: &Observations) -> Result<Self> {
        Self::new(
            set,
            observations.canonical_mask(),
            observations.input_mask(),
        )
    }
    /// 正準行[6841]と入力特徴[D+1]から作り、仮想行は8区分の論理和にする。
    pub fn new(set: InputFeatureSet, canonical: Vec<bool>, input: Vec<bool>) -> Result<Self> {
        if canonical.len() != CANONICAL_ROWS
            || canonical[CANONICAL_ROWS - 1]
            || input.len() != set.dimensions() + 1
            || input[set.dimensions()]
        {
            return Err(Error::Invalid("observation mask shape or padding"));
        }
        let (first_layer, virtual_rows) = match set {
            InputFeatureSet::P => (canonical.clone(), Vec::new()),
            InputFeatureSet::HalfKpHmB8 => {
                let mut virtual_rows = vec![false; FEATURE_COUNT + 1];
                for (i, &seen) in input[..set.dimensions()].iter().enumerate() {
                    virtual_rows[i % FEATURE_COUNT] |= seen;
                }
                (input, virtual_rows)
            }
        };
        Ok(Self {
            canonical,
            first_layer,
            virtual_rows,
        })
    }
    /// 線形とP型第1層の正準マスク。
    pub fn canonical(&self) -> &[bool] {
        &self.canonical
    }
    /// 第1層の行に対応するマスク。
    pub fn first_layer(&self) -> &[bool] {
        &self.first_layer
    }
    /// 仮想行のマスク。P型は空。
    pub fn virtual_rows(&self) -> &[bool] {
        &self.virtual_rows
    }
}

/// パラメータと、未観測行を復元するための不変の初期値。
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    parameters: Parameters,
    masks: Masks,
    initial_linear: Vec<f32>,
    initial_virtual: Vec<f32>,
}
impl Model {
    /// J75の全基底特徴の2端点に鏡映対称性を要求し、観測行を決定的に初期化する。
    /// init_stdは第1層の正規分布の標準偏差で、有限かつ非負でなければならない。
    pub fn initialize(
        shape: Shape,
        initial: &[[f32; 2]],
        masks: Masks,
        seed: u64,
        init_std: f32,
    ) -> Result<Self> {
        if !init_std.is_finite() || init_std < 0.0 {
            return Err(Error::Invalid(
                "initial standard deviation must be finite and nonnegative",
            ));
        }
        validate_initial(initial)?;
        if masks.first_layer.len() != shape.rows()
            || masks.virtual_rows.len() * shape.width != shape.len(Parameter::Virtual)
        {
            return Err(Error::Invalid("mask feature set"));
        }
        let mut parameters = Parameters::zeros(shape);
        for (base, endpoints) in initial.iter().enumerate() {
            if base % 12 < 6 {
                let row = canonical(base);
                parameters[Parameter::Linear][2 * row..2 * row + 2].copy_from_slice(endpoints);
            }
        }
        let mut random = Normal::new(seed, init_std);
        let mut shared = vec![0.0; CANONICAL_ROWS * shape.width];
        for (row, &observed) in masks.canonical.iter().enumerate() {
            if observed {
                for value in &mut shared[row * shape.width..(row + 1) * shape.width] {
                    *value = random.next();
                }
            }
        }
        if shape.feature_set == InputFeatureSet::P {
            parameters[Parameter::FirstLayer].copy_from_slice(&shared);
        } else {
            for base in 0..FEATURE_COUNT {
                for j in 0..shape.width {
                    parameters[Parameter::Virtual][base * shape.width + j] =
                        shared[canonical(base) * shape.width
                            + rotated_unit(base % 12 >= 6, j, shape.width)];
                }
            }
        }
        parameters.validate()?;
        Ok(Self {
            initial_linear: parameters[Parameter::Linear].to_vec(),
            initial_virtual: parameters[Parameter::Virtual].to_vec(),
            parameters,
            masks,
        })
    }
    /// モデルの形。
    pub fn shape(&self) -> Shape {
        self.parameters.shape
    }
    /// 全パラメータ。
    pub fn parameters(&self) -> &Parameters {
        &self.parameters
    }
    /// 指定したパラメータの値を編集する。配列の長さやモデルの形は変更できない。
    pub fn parameter_mut(&mut self, parameter: Parameter) -> &mut [f32] {
        &mut self.parameters[parameter]
    }
    /// 観測マスク。
    pub fn masks(&self) -> &Masks {
        &self.masks
    }
    /// 未観測の線形を戻す値[6841, 2]。
    pub fn initial_linear(&self) -> &[f32] {
        &self.initial_linear
    }
    /// 未観測の仮想行を戻す値。P型は空。
    pub fn initial_virtual(&self) -> &[f32] {
        &self.initial_virtual
    }
    /// バイアスを[H]に展開する。
    pub fn expanded_biases(&self) -> Vec<f32> {
        (0..self.shape().width)
            .map(|j| self.parameters[Parameter::Biases][j % self.shape().units()])
            .collect()
    }
    /// 出力重みを[2, H]に展開する。
    pub fn expanded_output_weights(&self) -> Vec<f32> {
        let s = self.shape();
        (0..2 * s.width)
            .map(|i| {
                self.parameters[Parameter::OutputWeights]
                    [i / s.width * s.units() + i % s.width % s.units()]
            })
            .collect()
    }
    /// 全基底特徴の線形[13680, 2]を展開する。
    pub fn expanded_linear(&self) -> Vec<[f32; 2]> {
        (0..FEATURE_COUNT)
            .map(|base| {
                let i = 2 * canonical(base);
                [
                    self.parameters[Parameter::Linear][i],
                    self.parameters[Parameter::Linear][i + 1],
                ]
            })
            .collect()
    }
    /// P型を回転付きで展開し、HalfKPはW+Vを畳み込む。埋め草を含む[D+1, H]。
    pub fn expanded_first_layer(&self) -> Vec<f32> {
        let s = self.shape();
        let mut result = vec![0.0; (s.feature_set.dimensions() + 1) * s.width];
        for row in 0..s.feature_set.dimensions() {
            for j in 0..s.width {
                result[row * s.width + j] = match s.feature_set {
                    InputFeatureSet::P => {
                        self.parameters[Parameter::FirstLayer]
                            [canonical(row) * s.width + rotated_unit(row % 12 >= 6, j, s.width)]
                    }
                    InputFeatureSet::HalfKpHmB8 => {
                        self.parameters[Parameter::FirstLayer][row * s.width + j]
                            + self.parameters[Parameter::Virtual][row % FEATURE_COUNT * s.width + j]
                    }
                };
            }
        }
        result
    }
    /// Python NNUE.constrainと同じ順で切り詰めた後、未観測行を復元する。
    pub fn constrain(&mut self) -> Result<()> {
        self.parameters.validate()?;
        let s = self.shape();
        linear::constrain_linear(&mut self.parameters[Parameter::Linear])?;
        for value in &mut self.parameters[Parameter::Virtual] {
            *value = value.clamp(-8.0, 8.0);
        }
        for i in 0..s.len(Parameter::FirstLayer) {
            let v = if s.feature_set == InputFeatureSet::P {
                0.0
            } else {
                let base = if i / s.width == s.feature_set.dimensions() {
                    FEATURE_COUNT
                } else {
                    i / s.width % FEATURE_COUNT
                };
                self.parameters[Parameter::Virtual][base * s.width + i % s.width]
            };
            self.parameters[Parameter::FirstLayer][i] =
                self.parameters[Parameter::FirstLayer][i].clamp(-8.0 - v, 8.0 - v);
        }
        for p in [
            Parameter::Biases,
            Parameter::OutputWeights,
            Parameter::OutputBias,
        ] {
            let limit = if p == Parameter::OutputWeights {
                1.98
            } else {
                8.0
            };
            for value in &mut self.parameters[p] {
                *value = value.clamp(-limit, limit);
            }
        }
        self.fix_unobserved();
        Ok(())
    }
    /// 未観測の線形・仮想行は初期値、第1層とすべての埋め草は0に戻す。
    pub fn fix_unobserved(&mut self) {
        let width = self.shape().width;
        for (row, &seen) in self.masks.canonical.iter().enumerate() {
            if !seen {
                self.parameters[Parameter::Linear][2 * row..2 * row + 2]
                    .copy_from_slice(&self.initial_linear[2 * row..2 * row + 2]);
            }
        }
        for (row, &seen) in self.masks.first_layer.iter().enumerate() {
            if !seen {
                self.parameters[Parameter::FirstLayer][row * width..(row + 1) * width].fill(0.0);
            }
        }
        for (row, &seen) in self.masks.virtual_rows.iter().enumerate() {
            if !seen {
                self.parameters[Parameter::Virtual][row * width..(row + 1) * width]
                    .copy_from_slice(&self.initial_virtual[row * width..(row + 1) * width]);
            }
        }
    }
}

fn validate_initial(initial: &[[f32; 2]]) -> Result<()> {
    if initial.len() != FEATURE_COUNT
        || initial
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || !(-4096.0..=4095.875).contains(v))
    {
        return Err(Error::Invalid(
            "initial PST must be finite [13680, 2] in -4096..4095.875",
        ));
    }
    if initial
        .iter()
        .enumerate()
        .any(|(base, &value)| value != initial[mirror_feature(base)])
    {
        return Err(Error::Invalid("initial PST mirror sharing violated"));
    }
    Ok(())
}

pub(crate) fn canonical(base: usize) -> usize {
    crate::data::canonical_row(base as u32) as usize
}
pub(crate) fn rotated_unit(rotated: bool, j: usize, width: usize) -> usize {
    (j + usize::from(rotated) * (width / 2)) % width
}

// SplitMix64の上位53ビットを(0,1)へ写す。Box–Mullerのcos、sinの順に消費する。
struct Normal {
    state: u64,
    spare: Option<f32>,
    std: f64,
}
impl Normal {
    fn new(seed: u64, std: f32) -> Self {
        Self {
            state: seed,
            spare: None,
            std: f64::from(std),
        }
    }
    fn uniform(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        // 分母を表現可能な2^53+2とし、対数への0と端点1を避ける。
        ((z >> 11) as f64 + 1.0) / (9_007_199_254_740_992.0 + 2.0)
    }
    fn next(&mut self) -> f32 {
        if let Some(value) = self.spare.take() {
            return value;
        }
        let radius = (-2.0 * self.uniform().ln()).sqrt() * self.std;
        let angle = std::f64::consts::TAU * self.uniform();
        self.spare = Some((radius * angle.sin()) as f32);
        (radius * angle.cos()) as f32
    }
}

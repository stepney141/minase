//! NNUEのf32パラメータ、鏡映共有、初期化、および保存範囲への制限。
//! 配列は行優先で、[`Parameter`]の順序がチェックポイントの保存順を定める。
//! Linearはセンチポーン単位の`[6841,2]`で、`canonical(base) = base/144×72 + (base%144)/12×6 + min(筋,11−筋)`へ写す。
//! P型の第1層は`[6841,H]`で、筋6以上の特徴はH/2だけ回転して読む。
//! HalfKPは第1層`[109441,H]`とVirtual`[13681,H]`を持ち、推論時は両者を足す。
//! P型のバイアスはH/2、視点ごとの出力重みはH/4個の独立な値を持つ。
//! 各配列の最後の埋め草行は常に0である。
//! 理由は`docs/adr/share-p-type-mirror-weights-by-half-width-rotation.md`にある。

mod checkpoint;
mod quantize;

pub use checkpoint::{Checkpoint, Progress};
pub use quantize::{Quantized, quantize};

use crate::nnue::data::{CANONICAL_ROWS, InputFeatureSet, Observations};
use minase::eval::features::{FEATURE_COUNT, mirror_feature};
use std::{
    fmt,
    ops::{Index, IndexMut},
};

/// 第1層の整数化の尺度。
pub(crate) const FIRST_LAYER_SCALE: f32 = 255.0;
/// 保存形式における第1層の重みの絶対値の上限。
pub(crate) const FIRST_LAYER_WEIGHT_MAX: i32 = 220;
/// 保存形式における第1層のバイアスの絶対値の上限。
pub(crate) const FIRST_LAYER_BIAS_MAX: i32 = 510;
/// 尺度255で保存範囲に収まる、第1層と仮想行の実数重みの上限。
pub(crate) const FIRST_LAYER_WEIGHT_LIMIT: f32 = FIRST_LAYER_WEIGHT_MAX as f32 / FIRST_LAYER_SCALE;
/// 尺度255で保存範囲に収まる、第1層の実数バイアスの上限。
pub(crate) const FIRST_LAYER_BIAS_LIMIT: f32 = FIRST_LAYER_BIAS_MAX as f32 / FIRST_LAYER_SCALE;

/// モデルと参照計算の失敗。
#[derive(Debug)]
pub enum Error {
    /// 再開条件の不一致。
    CheckpointMismatch(crate::nn::checkpoint::MismatchField),
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
            Self::CheckpointMismatch(field) => {
                write!(f, "invalid model input: checkpoint {field} mismatch")
            }
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
impl From<crate::nn::Error> for Error {
    fn from(error: crate::nn::Error) -> Self {
        match error {
            crate::nn::Error::CheckpointMismatch(field) => Self::CheckpointMismatch(field),
            crate::nn::Error::Io(error) => Self::Io(error),
            crate::nn::Error::Invalid(reason) => Self::Invalid(reason),
            crate::nn::Error::Format(reason) => Self::Format(reason),
        }
    }
}
/// モデルと参照計算の結果。
pub type Result<T> = std::result::Result<T, Error>;

/// MNUEの構造記述と一致する出力部。
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[repr(u64)]
pub enum HeadKind {
    /// 要素積、幅15と幅32の隠れ層、および直接内積。
    #[value(name = "pairwise-l16-l32")]
    #[serde(rename = "pairwise-l16-l32")]
    PairwiseL16L32,
}
impl HeadKind {
    /// 対応する全出力部。
    pub const ALL: [Self; 1] = [Self::PairwiseL16L32];
    /// MNUEとCLIで使う名前。
    pub fn name(self) -> &'static str {
        match self {
            Self::PairwiseL16L32 => "pairwise-l16-l32",
        }
    }
}

/// パラメータ配列の種類。P型の仮想行は長さ0。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Parameter {
    /// 線形正準行`[6841, 2]`、センチポーン。
    Linear,
    /// P型`[6841, H]`、HalfKP`[109441, H]`。
    FirstLayer,
    /// HalfKPの仮想行`[13681, H]`。
    Virtual,
    /// P型`[H/2]`、HalfKP`[H]`。
    Biases,
    /// 直接出力o。P型`[2, H/4]`、HalfKP`[2, H/2]`。
    OutputWeights,
    /// ロジット単位の出力バイアスb`[1]`。
    OutputBias,
    /// W1。P型`[15, 2, H/4]`、HalfKP`[15, 2, H/2]`。
    Hidden1Weights,
    /// b1`[15]`。
    Hidden1Biases,
    /// W2`[32, 30]`。
    Hidden2Weights,
    /// b2`[32]`。
    Hidden2Biases,
    /// 隠れ層の出力重みv`[32]`。
    Hidden2Output,
}
impl Parameter {
    /// 保存順の全パラメータ。
    pub const ALL: [Self; 11] = [
        Self::Linear,
        Self::FirstLayer,
        Self::Virtual,
        Self::Biases,
        Self::OutputWeights,
        Self::OutputBias,
        Self::Hidden1Weights,
        Self::Hidden1Biases,
        Self::Hidden2Weights,
        Self::Hidden2Biases,
        Self::Hidden2Output,
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
            Self::Hidden1Weights => "head.hidden1_weights",
            Self::Hidden1Biases => "head.hidden1_biases",
            Self::Hidden2Weights => "head.hidden2_weights",
            Self::Hidden2Biases => "head.hidden2_biases",
            Self::Hidden2Output => "head.hidden2_output",
        }
    }
}

/// 検査済みの入力特徴集合、出力部、幅、勝率尺度（1ロジットあたりのcp）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    feature_set: InputFeatureSet,
    width: usize,
    cp_per_logit: f32,
    head: HeadKind,
}
impl Shape {
    /// 幅は4..=512の4の倍数、勝率尺度は有限かつ0より大きく100000以下。
    pub fn new(
        feature_set: InputFeatureSet,
        width: usize,
        cp_per_logit: f32,
        head: HeadKind,
    ) -> Result<Self> {
        if !(4..=512).contains(&width) || !width.is_multiple_of(4) {
            return Err(Error::Invalid(
                "width must be a multiple of 4 and in 4..512",
            ));
        }
        if !cp_per_logit.is_finite() || cp_per_logit <= 0.0 || cp_per_logit > 100_000.0 {
            return Err(Error::Invalid("cp_per_logit must be in (0, 100000]"));
        }
        Ok(Self {
            feature_set,
            width,
            cp_per_logit,
            head,
        })
    }
    /// 出力部の種類。
    pub fn head(self) -> HeadKind {
        self.head
    }
    /// 入力特徴集合。
    pub fn feature_set(self) -> InputFeatureSet {
        self.feature_set
    }
    /// 展開後の幅H。
    pub fn width(self) -> usize {
        self.width
    }
    /// 勝率尺度。
    pub fn cp_per_logit(self) -> f32 {
        self.cp_per_logit
    }
    /// 第1層のバイアスの自由パラメータ数。
    pub fn bias_units(self) -> usize {
        if self.feature_set == InputFeatureSet::P {
            self.width / 2
        } else {
            self.width
        }
    }
    /// 視点ごとの出力重みの自由パラメータ数。
    pub fn output_units(self) -> usize {
        self.bias_units() / 2
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
            Parameter::Biases => self.bias_units(),
            Parameter::OutputWeights => 2 * self.output_units(),
            Parameter::OutputBias => 1,
            Parameter::Hidden1Weights => 15 * 2 * self.output_units(),
            Parameter::Hidden1Biases => 15,
            Parameter::Hidden2Weights => 32 * 30,
            Parameter::Hidden2Biases | Parameter::Hidden2Output => 32,
        }
    }
    /// MNUE版5と同じ構造記述文字列。
    pub fn architecture(self) -> String {
        let name = match self.feature_set {
            InputFeatureSet::P => "P",
            InputFeatureSet::HalfKpHmB8 => "HalfKP-hm-b8",
        };
        format!(
            "features={name};ft={};linear=tapered2;head={}",
            self.width,
            self.head.name()
        )
    }
}

/// 形を変えずに読み書きできる全パラメータ。勾配とAdamWの状態も同じ配置。
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    shape: Shape,
    values: [Vec<f32>; 11],
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
    /// 訓練集合の[`Observations`]から正準行と両視点の入力行のマスクを作る。
    /// 鏡映込みの手番側の回数の左半分を正準マスクとし、両視点を鏡映なしで数えた入力マスクを[`Self::new`]へ渡す。
    pub fn from_observations(set: InputFeatureSet, observations: &Observations) -> Result<Self> {
        Self::new(
            set,
            observations.canonical_mask(),
            observations.input_mask(),
        )
    }
    /// 正準行`[6841]`と入力特徴`[D+1]`から作り、仮想行は8区分の論理和にする。
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
    /// 全基底特徴の2端点に鏡映対称性を要求し、観測済みの正準行だけを行番号順に初期化する。
    /// SplitMix64の上位53ビットkを`(k+1)/(2^53+2)`へ写し、Box–Muller法でcos、sinの順に正規乱数を使う。
    /// P型では第1層へ置き、HalfKPではVirtual`[base]`へ正準行を回転して置いて特徴行を0にするため、同じ初期値とシードでは両型の初期評価が一致する。
    /// 続く同じ乱数列からW1、b1を一様分布±1/√H、W2、b2を一様分布±1/√30の順に引く。
    /// 直接重みo、隠れ層の出力重みv、出力バイアスbは0とする。
    /// `init_std`は有限かつ非負の標準偏差で、学習時は両型とも基底特徴数D = `FEATURE_COUNT`に対してsqrt(1/(3D))を使う。
    /// 選び方は`docs/lessons/nnue-first-layer-init-saturation.md`にある。
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
        for (p, inputs) in [
            (Parameter::Hidden1Weights, shape.width),
            (Parameter::Hidden1Biases, shape.width),
            (Parameter::Hidden2Weights, 30),
            (Parameter::Hidden2Biases, 30),
        ] {
            let bound = 1.0 / (inputs as f64).sqrt();
            for value in &mut parameters[p] {
                *value = ((2.0 * random.uniform() - 1.0) * bound) as f32;
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
    /// 未観測の線形を戻す値`[6841, 2]`。
    pub fn initial_linear(&self) -> &[f32] {
        &self.initial_linear
    }
    /// 未観測の仮想行を戻す値。P型は空。
    pub fn initial_virtual(&self) -> &[f32] {
        &self.initial_virtual
    }
    /// バイアスを`[H]`に展開する。
    pub fn expanded_biases(&self) -> Vec<f32> {
        (0..self.shape().width)
            .map(|j| self.parameters[Parameter::Biases][j % self.shape().bias_units()])
            .collect()
    }
    /// 出力重みoを`[2, H/2]`に展開する。
    pub fn expanded_output_weights(&self) -> Vec<f32> {
        self.expanded_head_weights(Parameter::OutputWeights)
    }
    /// W1を`[15, H]`へ展開する。P型は各視点のjとj+H/4を共有する。
    pub fn expanded_hidden1_weights(&self) -> Vec<f32> {
        self.expanded_head_weights(Parameter::Hidden1Weights)
    }
    fn expanded_head_weights(&self, p: Parameter) -> Vec<f32> {
        let s = self.shape();
        let half = s.width / 2;
        let units = s.output_units();
        (0..self.parameters[p].len() / units * half)
            .map(|i| self.parameters[p][i / half * units + i % half % units])
            .collect()
    }
    /// 観測済みの展開後の第1層重みのうち、絶対値が切り詰め上限に達した割合。
    /// HalfKPはW+Vで判定し、未観測の特徴行と埋め草を分母から除く。
    /// 観測済みの行がなければ、分母を定義できないためエラーを返す。
    pub fn first_layer_at_limit(&self) -> Result<f64> {
        let s = self.shape();
        let mut count = 0_usize;
        let mut total = 0_usize;
        for (row, &seen) in self.masks.first_layer.iter().enumerate() {
            if !seen {
                continue;
            }
            for j in 0..s.width {
                let mut value = self.parameters[Parameter::FirstLayer][row * s.width + j];
                if s.feature_set == InputFeatureSet::HalfKpHmB8 {
                    value += self.parameters[Parameter::Virtual][row % FEATURE_COUNT * s.width + j];
                }
                count += usize::from(value.abs() >= FIRST_LAYER_WEIGHT_LIMIT);
                total += 1;
            }
        }
        // P型の各正準行は同じ個数の展開行を持つので、比率は展開前でも等しい。
        if total == 0 {
            return Err(Error::Invalid(
                "first-layer limit fraction has no observed rows",
            ));
        }
        Ok(count as f64 / total as f64)
    }
    /// 全基底特徴の線形`[13680, 2]`を展開する。
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
    /// P型を回転付きで展開し、HalfKPはW+Vを畳み込む。埋め草を含む`[D+1, H]`。
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
    /// 線形、仮想行、第1層、出力部の順に保存範囲へ制限する。
    /// 線形は[−4096,4095.875]、仮想行は±L、第1層はP型なら±L、HalfKPなら[−L−V,L−V]とする。
    /// L=220/255であり、第1層のバイアスは±2とする。
    /// W1とW2は±127/64、隠れ層と出力のバイアスは±8、oとvは±1.98とする。
    /// 最後に[`Self::fix_unobserved`]で未観測行を復元する。
    pub fn constrain(&mut self) -> Result<()> {
        self.parameters.validate()?;
        let s = self.shape();
        for value in &mut self.parameters[Parameter::Linear] {
            *value = value.clamp(-4096.0, 4095.875);
        }
        for value in &mut self.parameters[Parameter::Virtual] {
            *value = value.clamp(-FIRST_LAYER_WEIGHT_LIMIT, FIRST_LAYER_WEIGHT_LIMIT);
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
            self.parameters[Parameter::FirstLayer][i] = self.parameters[Parameter::FirstLayer][i]
                .clamp(-FIRST_LAYER_WEIGHT_LIMIT - v, FIRST_LAYER_WEIGHT_LIMIT - v);
        }
        for p in [
            Parameter::Biases,
            Parameter::OutputWeights,
            Parameter::OutputBias,
            Parameter::Hidden1Weights,
            Parameter::Hidden1Biases,
            Parameter::Hidden2Weights,
            Parameter::Hidden2Biases,
            Parameter::Hidden2Output,
        ] {
            let limit = match p {
                Parameter::Biases => FIRST_LAYER_BIAS_LIMIT,
                Parameter::OutputWeights | Parameter::Hidden2Output => 1.98,
                Parameter::Hidden1Weights | Parameter::Hidden2Weights => 127.0 / 64.0,
                _ => 8.0,
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
    base / 144 * 72 + base % 144 / 12 * 6 + (base % 12).min(11 - base % 12)
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

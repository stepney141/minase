//! tatara形式の量子化ネットと整数の順伝播。
//!
//! A型はLayerStack、HalfKP_vmはSimpleと組み合わせる。第1層の幅Hは2〜1,024の
//! 偶数であり、量子化済みの整数を検査してから非公開のフィールドに保持する。
//! 第1層は尺度127、後段の重みは64、後段のバイアスとPSQTは8,128を尺度とする。
//! 評価値は整数の出力を正の`fv_scale`で0方向に除算し、±28,999に制限する。
//!
//! ファイルはリトルエンディアンで、版、ネットのハッシュ、構造記述のバイト長、
//! UTF-8の構造記述の順に始まる。LayerStackの版は0x7AF32F21、Simpleは0x7AF32F16。
//! LayerStackだけ区分数1を続け、その後に第1層のハッシュとバイアス・行優先の重みを置く。
//! LayerStackの第1層は符号付きLEB128、Simpleは非圧縮のi16である。
//! LayerStackはさらにPSQTの0バイアスと13,680個のi32重みを置く。
//! 最後は後段のハッシュ、L1、L2、出力の順で、各層にi32バイアスとi8重みを置く。
//! ファイル中の重みの各行は入力幅を32の倍数に切り上げて0で埋める。
//! 構造記述の長さは1〜16,384バイトであり、再構築した記述との不一致、非0の詰め物、
//! 途中切れ、末尾の余り、値域と鏡映共有の違反を拒否する。

mod evaluator;
mod layer_stack;
mod leb128;
mod simple;
pub(crate) use evaluator::SearchEvaluator;
pub use layer_stack::{LayerStackNetwork, LayerStackParameters};
pub use simple::{SimpleNetwork, SimpleParameters};

#[cfg(feature = "test-util")]
pub mod test_util;
#[cfg(test)]
mod tests;

use super::EVALUATION_LIMIT;
use super::input::{AnchorError, FeatureSet, Indexer};
use super::transformer::FeatureTransformer;
use minase_core::{Color, Position};
use std::fmt;

/// ネットの形式または量子化済みパラメータが不正である原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatError {
    /// 指定区画に必要なバイトがない。
    Truncated {
        /// 区画の名前。
        section: &'static str,
    },
    /// ネット末尾に未使用バイトがある。
    TrailingBytes {
        /// 余りのバイト数。
        count: usize,
    },
    /// この構造では受理できない版の番号である。
    Version {
        /// 読み取った番号。
        actual: u32,
    },
    /// 構造記述が1〜16,384バイトでない。
    ArchLength {
        /// 読み取った長さ。
        actual: u32,
    },
    /// 構造記述がUTF-8でない。
    ArchUtf8,
    /// 特徴集合の名前が構造と対応しない。
    FeatureName {
        /// 読み取った名前。
        actual: String,
    },
    /// 幅が2〜1,024の偶数でない、または整数として読めない。
    Width {
        /// 入力された値。
        value: String,
    },
    /// 除数が1〜i32::MAXでない、または整数として読めない。
    FvScale {
        /// 入力された値。
        value: String,
    },
    /// 構造記述が正規形と一致しない。
    Architecture {
        /// 期待する記述または文法。
        expected: String,
        /// 読み取った記述。
        actual: String,
    },
    /// ハッシュが構造から計算した値と一致しない。
    Hash {
        /// network_hash、ft_hash、fc_hashのいずれか。
        section: &'static str,
        /// 構造から計算した値。
        expected: u32,
        /// 読み取った値。
        actual: u32,
    },
    /// LayerStackの区分数が1でない。
    Buckets {
        /// 読み取った数。
        actual: u32,
    },
    /// LEB128区画の識別子が一致しない。
    LebMagic {
        /// 区画の名前。
        section: &'static str,
    },
    /// 圧縮バイト数が値の数以上、3倍以下でない。
    LebSize {
        /// 区画の名前。
        section: &'static str,
        /// 圧縮バイト数。
        actual: usize,
        /// 要求する値の数。
        count: usize,
    },
    /// LEB128の値が10バイト以内のi64として表せない。
    LebOverflow {
        /// 区画の名前。
        section: &'static str,
    },
    /// 復号したLEB128の値がi16で表せない。
    LebRange {
        /// 区画の名前。
        section: &'static str,
        /// 復号値。
        value: i64,
    },
    /// LEB128区画が要求した個数を満たす前に終わった。
    LebCount {
        /// 区画の名前。
        section: &'static str,
        /// 要求する個数。
        expected: usize,
        /// 復号済みの個数。
        actual: usize,
    },
    /// LEB128区画で要求した個数の後にバイトが残った。
    LebTrailing {
        /// 区画の名前。
        section: &'static str,
        /// 余ったバイト数。
        count: usize,
    },
    /// PSQTのバイアスが0でない。
    PsqtBias {
        /// 入力値。
        value: i32,
    },
    /// 後段の詰め物が0でない。
    Padding {
        /// 層の名前。
        layer: &'static str,
        /// 行番号。
        row: usize,
        /// 詰め物を含む列番号。
        column: usize,
    },
    /// 第1層の累算値の絶対値上限が32,767を超えた。
    AccumulatorBound {
        /// 王駒の区分番号。
        bucket: usize,
        /// 成分番号。
        component: usize,
        /// 計算した上限。
        bound: i32,
    },
    /// 後段のバイアスの絶対値が2^24を超えた。
    BiasRange {
        /// 層の名前。
        layer: &'static str,
        /// 入力値。
        value: i32,
    },
    /// PSQTの重みの絶対値が2^22を超えた。
    PsqtRange {
        /// 行番号。
        row: usize,
        /// 入力値。
        value: i32,
    },
    /// 第1層の重みが鏡映先の半幅回転と一致しない。
    MirrorWeight {
        /// 行番号。
        row: usize,
        /// 成分番号。
        component: usize,
    },
    /// 第1層のバイアスの前半と後半が一致しない。
    MirrorBias {
        /// 前半の成分番号。
        component: usize,
    },
    /// PSQTの鏡映対の重みが一致しない。
    MirrorPsqt {
        /// 行番号。
        row: usize,
    },
    /// パラメータの配列長が構造と一致しない。
    ParameterLength {
        /// 欄の名前。
        field: &'static str,
        /// 必要な個数。
        expected: usize,
        /// 入力の個数。
        actual: usize,
    },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { section } => write!(f, "truncated {section}"),
            Self::TrailingBytes { count } => write!(f, "{count} trailing bytes"),
            Self::Version { actual } => write!(f, "unsupported version {actual:#010x}"),
            Self::ArchLength { actual } => {
                write!(f, "architecture length outside 1..=16384: {actual}")
            }
            Self::ArchUtf8 => write!(f, "architecture is not UTF-8"),
            Self::FeatureName { actual } => {
                write!(f, "unsupported feature set for this architecture: {actual}")
            }
            Self::Width { value } => write!(f, "width must be even and in 2..=1024: {value}"),
            Self::FvScale { value } => write!(f, "fv_scale must be in 1..=i32::MAX: {value}"),
            Self::Architecture { expected, actual } => write!(
                f,
                "architecture mismatch: expected {expected}, got {actual}"
            ),
            Self::Hash {
                section,
                expected,
                actual,
            } => write!(
                f,
                "{section}: expected {expected:#010x}, got {actual:#010x}"
            ),
            Self::Buckets { actual } => write!(f, "expected one bucket, got {actual}"),
            Self::LebMagic { section } => write!(f, "invalid LEB128 marker in {section}"),
            Self::LebSize {
                section,
                actual,
                count,
            } => write!(
                f,
                "{section}: {actual} compressed bytes cannot encode {count} i16 values"
            ),
            Self::LebOverflow { section } => write!(f, "LEB128 i64 overflow in {section}"),
            Self::LebRange { section, value } => {
                write!(f, "LEB128 value outside i16 in {section}: {value}")
            }
            Self::LebCount {
                section,
                expected,
                actual,
            } => write!(
                f,
                "{section}: expected {expected} LEB128 values, got {actual}"
            ),
            Self::LebTrailing { section, count } => {
                write!(f, "{section}: {count} trailing LEB128 bytes")
            }
            Self::PsqtBias { value } => write!(f, "PSQT bias must be zero: {value}"),
            Self::Padding { layer, row, column } => {
                write!(f, "nonzero {layer} padding at ({row}, {column})")
            }
            Self::AccumulatorBound {
                bucket,
                component,
                bound,
            } => write!(
                f,
                "accumulator bound {bound} exceeds 32767 at bucket {bucket}, component {component}"
            ),
            Self::BiasRange { layer, value } => {
                write!(f, "{layer} bias exceeds absolute limit 2^24: {value}")
            }
            Self::PsqtRange { row, value } => {
                write!(f, "PSQT row {row} exceeds absolute limit 2^22: {value}")
            }
            Self::MirrorWeight { row, component } => {
                write!(f, "FT mirror mismatch at row {row}, component {component}")
            }
            Self::MirrorBias { component } => {
                write!(f, "FT bias mirror mismatch at component {component}")
            }
            Self::MirrorPsqt { row } => write!(f, "PSQT mirror mismatch at row {row}"),
            Self::ParameterLength {
                field,
                expected,
                actual,
            } => write!(f, "{field}: expected {expected} parameters, got {actual}"),
        }
    }
}
impl std::error::Error for FormatError {}

/// ファイルの版により選ぶ、入力特徴集合と構造の固定した組。
pub enum Network {
    /// A型と、PSQTを持つLayerStack。
    LayerStack(LayerStackNetwork),
    /// HalfKP_vmとSimple。
    Simple(SimpleNetwork),
}

impl Network {
    /// 探索中に王駒の基準を一意に保てる根だけを受理する。
    pub fn check_search_root(&self, position: &Position) -> Result<(), AnchorError> {
        match self {
            Self::LayerStack(_) => Ok(()),
            Self::Simple(_) => super::input::HalfKpVm::check_search_root(position),
        }
    }
    /// 静かな手の同点破りに使うPSQTの差。Simpleでは生成順を保つため0を返す。
    pub fn quiet_move_delta(&self, position: &Position, mv: minase_core::Move) -> i64 {
        match self {
            Self::LayerStack(net) => net.quiet_move_psqt_delta(position, mv),
            Self::Simple(_) => 0,
        }
    }
    fn accumulator_stats(&self) -> &super::stats::SharedStats {
        match self {
            Self::LayerStack(net) => &net.accumulator_stats,
            Self::Simple(net) => &net.accumulator_stats,
        }
    }
    /// 全ワーカーを回収した後、累算器の更新回数を回収して0へ戻す。
    pub fn take_accumulator_stats(&self) -> super::AccumulatorStats {
        super::stats::take(self.accumulator_stats())
    }

    /// 先頭の版で構造を選び、形式と全ての値域・共有条件を検査する。
    pub fn decode(bytes: &[u8]) -> Result<Self, FormatError> {
        match Reader::new(bytes).u32("version")? {
            0x7AF3_2F21 => LayerStackNetwork::decode(bytes).map(Self::LayerStack),
            0x7AF3_2F16 => SimpleNetwork::decode(bytes).map(Self::Simple),
            actual => Err(FormatError::Version { actual }),
        }
    }
    /// 保持する構造のtatara形式で書き出す。
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::LayerStack(n) => n.encode(),
            Self::Simple(n) => n.encode(),
        }
    }
    /// 入力特徴集合を返す。
    pub fn feature_set(&self) -> FeatureSet {
        match self {
            Self::LayerStack(_) => FeatureSet::A,
            Self::Simple(_) => FeatureSet::HalfKpVm,
        }
    }
    /// 第1層の幅を返す。
    pub fn width(&self) -> usize {
        match self {
            Self::LayerStack(n) => n.width(),
            Self::Simple(n) => n.width(),
        }
    }
    /// 整数の出力を評価値に換算する正の除数を返す。
    pub fn fv_scale(&self) -> i32 {
        match self {
            Self::LayerStack(n) => n.fv_scale(),
            Self::Simple(n) => n.fv_scale(),
        }
    }
    /// 手番側の視点で全再計算する。Simpleの王駒の文脈が不正なら拒否する。
    pub fn evaluate(&self, position: &Position) -> Result<i32, AnchorError> {
        self.evaluate_for(position, position.side_to_move())
    }
    /// 指定陣営を手番側として全再計算する。Simpleでは両視点の文脈を検査する。
    pub fn evaluate_for(&self, position: &Position, side: Color) -> Result<i32, AnchorError> {
        match self {
            Self::LayerStack(n) => Ok(n.evaluate_accumulator(&n.refresh(position), side)),
            Self::Simple(n) => n.evaluate_for(position, side),
        }
    }
}

fn check_width(width: usize) -> Result<(), FormatError> {
    if !(2..=1024).contains(&width) || !width.is_multiple_of(2) {
        return Err(FormatError::Width {
            value: width.to_string(),
        });
    }
    Ok(())
}
fn check_scale(scale: i32) -> Result<(), FormatError> {
    if scale < 1 {
        return Err(FormatError::FvScale {
            value: scale.to_string(),
        });
    }
    Ok(())
}
fn check_len(field: &'static str, expected: usize, actual: usize) -> Result<(), FormatError> {
    if expected != actual {
        return Err(FormatError::ParameterLength {
            field,
            expected,
            actual,
        });
    }
    Ok(())
}

/// 各升には駒が1枚、先獅子は1つだけなので、バイアスの絶対値、升ごとの
/// 重みの絶対値の最大値144個、先獅子の最大値の和は最終累算値の絶対値の上限になる。
/// 区分と成分ごとにこの和を32,767以下に制限すれば、i16剰余演算の最終値は真の和と一致する。
fn validate_ft(
    set: FeatureSet,
    width: usize,
    biases: &[i16],
    weights: &[i16],
) -> Result<(), FormatError> {
    check_width(width)?;
    let spec = set.spec();
    check_len("ft_biases", width, biases.len())?;
    check_len("ft_weights", spec.dimensions() * width, weights.len())?;
    let rows = spec.piece_inputs();
    for bucket in 0..spec.king_buckets() {
        let mut maxima = vec![0_i32; 145 * width];
        for row in 0..rows {
            let square = if row < rows - 144 { row % 144 } else { 144 };
            for (component, &weight) in weights
                [(bucket * rows + row) * width..(bucket * rows + row + 1) * width]
                .iter()
                .enumerate()
            {
                let cell = &mut maxima[square * width + component];
                *cell = (*cell).max(i32::from(weight).abs());
            }
        }
        for (component, &bias) in biases.iter().enumerate() {
            let bound = i32::from(bias).abs()
                + (0..145)
                    .map(|square| maxima[square * width + component])
                    .sum::<i32>();
            if bound > 32767 {
                return Err(FormatError::AccumulatorBound {
                    bucket,
                    component,
                    bound,
                });
            }
        }
    }
    Ok(())
}
fn transformer<I: Indexer>(
    width: usize,
    biases: Vec<i16>,
    weights: Vec<i16>,
) -> FeatureTransformer<I> {
    FeatureTransformer::new(width, biases.into_boxed_slice(), weights.into_boxed_slice())
        .expect("network constructor has checked transformer dimensions")
}

/// 後段の最大積和は2^24 + 2×1024×128×127でありi32に収まる。
/// LayerStackのskipの加算もこの範囲に収まり、2乗だけはi64で計算する。
/// PSQTは145×2^22の和と両視点の差までi32に収まる。
fn check_biases(layer: &'static str, values: &[i32]) -> Result<(), FormatError> {
    for &value in values {
        if value.unsigned_abs() > 1 << 24 {
            return Err(FormatError::BiasRange { layer, value });
        }
    }
    Ok(())
}

const PAIR_INPUT_MAX: i16 = 127;
const _: () = assert!(PAIR_INPUT_MAX as i32 * PAIR_INPUT_MAX as i32 <= i16::MAX as i32);

/// 両因子を0〜127へ制限し、積を7ビット右シフトする。結果の上限は126。
fn pair(a: i16, b: i16) -> i16 {
    (a.clamp(0, PAIR_INPUT_MAX) * b.clamp(0, PAIR_INPUT_MAX)) >> 7
}
/// i64で2乗して19ビット右シフトし、0〜127へ制限する。負の入力も2乗する。
fn square_activation(y: i32) -> u8 {
    ((i64::from(y) * i64::from(y)) >> 19).clamp(0, 127) as u8
}
/// 6ビットの算術右シフト後に0〜127へ制限する。
fn activation(y: i32) -> u8 {
    (y >> 6).clamp(0, 127) as u8
}
/// 正の除数で0方向に切り捨て、詰み値と重ならない±28,999へ制限する。
fn score(out: i64, scale: i32) -> i32 {
    (out / i64::from(scale)).clamp(-i64::from(EVALUATION_LIMIT), i64::from(EVALUATION_LIMIT)) as i32
}
/// 両視点の差を2で割り、負の奇数も0方向に切り捨てる。
fn psqt_half(us: i32, them: i32) -> i32 {
    (us - them) / 2
}
fn affine(bias: i32, weights: &[i8], input: &[u8]) -> i32 {
    bias + weights
        .iter()
        .zip(input)
        .map(|(&w, &x)| i32::from(w) * i32::from(x))
        .sum::<i32>()
}

struct Dense {
    l1_biases: Vec<i32>,
    l1_weights: Vec<i8>,
    l2_biases: Vec<i32>,
    l2_weights: Vec<i8>,
    output_bias: i32,
    output_weights: Vec<i8>,
}
impl Dense {
    fn validate(&self, input: usize, l1: usize, l2_input: usize) -> Result<(), FormatError> {
        for (field, expected, actual) in [
            ("l1_biases", l1, self.l1_biases.len()),
            ("l1_weights", l1 * input, self.l1_weights.len()),
            ("l2_biases", 32, self.l2_biases.len()),
            ("l2_weights", 32 * l2_input, self.l2_weights.len()),
            ("output_weights", 32, self.output_weights.len()),
        ] {
            check_len(field, expected, actual)?;
        }
        check_biases("L1", &self.l1_biases)?;
        check_biases("L2", &self.l2_biases)?;
        check_biases("output", &[self.output_bias])
    }
    fn encode(&self, bytes: &mut Vec<u8>, input: usize, l2_input: usize) {
        write_i32(bytes, &self.l1_biases);
        write_weights(bytes, &self.l1_weights, input);
        write_i32(bytes, &self.l2_biases);
        write_weights(bytes, &self.l2_weights, l2_input);
        write_i32(bytes, &[self.output_bias]);
        write_weights(bytes, &self.output_weights, 32);
    }
    fn decode(
        reader: &mut Reader<'_>,
        input: usize,
        l1: usize,
        l2_input: usize,
    ) -> Result<Self, FormatError> {
        Ok(Self {
            l1_biases: reader.i32s(l1, "L1 biases")?,
            l1_weights: reader.weights(l1, input, "L1 weights")?,
            l2_biases: reader.i32s(32, "L2 biases")?,
            l2_weights: reader.weights(32, l2_input, "L2 weights")?,
            output_bias: reader.i32s(1, "output bias")?[0],
            output_weights: reader.weights(1, 32, "output weights")?,
        })
    }
}
fn version(set: FeatureSet) -> u32 {
    match set {
        FeatureSet::A => 0x7AF3_2F21,
        FeatureSet::HalfKpVm => 0x7AF3_2F16,
    }
}
fn arch(set: FeatureSet, width: usize, scale: i32) -> String {
    let spec = set.spec();
    let features = format!(
        "Features={}(Friend)[{}->{width}x2]",
        spec.arch_name(),
        spec.dimensions()
    );
    let combined = 2 * width;
    match set {
        FeatureSet::A => format!(
            "{features},PSQT=1,Network=AffineTransform[1<-32](ClippedReLU[32](AffineTransform[32<-30](SqrClippedReLU[30](AffineTransform[16<-{combined}](InputSlice[{combined}(0:{combined})]))))),fv_scale={scale}"
        ),
        FeatureSet::HalfKpVm => format!(
            "{features},Network=AffineTransform[1<-32](ClippedReLU[32](AffineTransform[32<-32](ClippedReLU[32](AffineTransformSparseInput[32<-{combined}](InputSlice[{combined}(0:{combined})]))))),fv_scale={scale}"
        ),
    }
}
fn ft_hash(set: FeatureSet, width: usize) -> u32 {
    set.spec().feature_hash() ^ (2 * width) as u32
}
fn fc_hash(set: FeatureSet, width: usize) -> u32 {
    let first = match set {
        FeatureSet::A => width,
        FeatureSet::HalfKpVm => 32,
    };
    let mut prev = 0xEC42_E90D ^ (2 * width) as u32;
    for (out, relu) in [(first, true), (32, true), (1, false)] {
        let mut h = 0xCC03_DAE4_u32.wrapping_add(out as u32);
        h ^= prev >> 1;
        h ^= prev << 31;
        if relu {
            h = h.wrapping_add(0x538D_24C7);
        }
        prev = h;
    }
    prev
}
fn header(set: FeatureSet, width: usize, scale: i32) -> Vec<u8> {
    let arch = arch(set, width, scale);
    let mut bytes = Vec::new();
    for value in [
        version(set),
        ft_hash(set, width) ^ fc_hash(set, width),
        arch.len() as u32,
    ] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(arch.as_bytes());
    bytes
}
fn read_header(reader: &mut Reader<'_>, set: FeatureSet) -> Result<(usize, i32), FormatError> {
    let actual = reader.u32("version")?;
    if actual != version(set) {
        return Err(FormatError::Version { actual });
    }
    let hash = reader.u32("network_hash")?;
    let length = reader.u32("architecture length")?;
    if !(1..=16384).contains(&length) {
        return Err(FormatError::ArchLength { actual: length });
    }
    let actual = std::str::from_utf8(reader.take(length as usize, "architecture")?)
        .map_err(|_| FormatError::ArchUtf8)?;
    let syntax = || FormatError::Architecture {
        expected: "Features=<name>(Friend)[<dimension>-><H>x2],<network>,fv_scale=<m>".into(),
        actual: actual.into(),
    };
    let rest = actual.strip_prefix("Features=").ok_or_else(syntax)?;
    let (name, rest) = rest.split_once("(Friend)[").ok_or_else(syntax)?;
    if name != set.spec().arch_name() {
        return Err(FormatError::FeatureName {
            actual: name.into(),
        });
    }
    let (feature, _) = rest.split_once(']').ok_or_else(syntax)?;
    let (_, width) = feature.split_once("->").ok_or_else(syntax)?;
    let width = width.strip_suffix("x2").ok_or_else(syntax)?;
    let width: usize = width.parse().map_err(|_| FormatError::Width {
        value: width.into(),
    })?;
    check_width(width)?;
    let (_, scale) = actual.rsplit_once(",fv_scale=").ok_or_else(syntax)?;
    let scale: i32 = scale.parse().map_err(|_| FormatError::FvScale {
        value: scale.into(),
    })?;
    check_scale(scale)?;
    let expected = arch(set, width, scale);
    if actual != expected {
        return Err(FormatError::Architecture {
            expected,
            actual: actual.into(),
        });
    }
    check_hash(
        "network_hash",
        ft_hash(set, width) ^ fc_hash(set, width),
        hash,
    )?;
    Ok((width, scale))
}
fn check_hash(section: &'static str, expected: u32, actual: u32) -> Result<(), FormatError> {
    if actual != expected {
        return Err(FormatError::Hash {
            section,
            expected,
            actual,
        });
    }
    Ok(())
}
fn write_i32(bytes: &mut Vec<u8>, values: &[i32]) {
    for value in values {
        bytes.extend(value.to_le_bytes());
    }
}
fn write_weights(bytes: &mut Vec<u8>, weights: &[i8], columns: usize) {
    for row in weights.chunks_exact(columns) {
        bytes.extend(row.iter().map(|&w| w as u8));
        bytes.resize(bytes.len() + columns.next_multiple_of(32) - columns, 0);
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    fn take(&mut self, count: usize, section: &'static str) -> Result<&'a [u8], FormatError> {
        if self.bytes.len() < count {
            return Err(FormatError::Truncated { section });
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }
    fn u32(&mut self, section: &'static str) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(
            self.take(4, section)?
                .try_into()
                .expect("four bytes checked"),
        ))
    }
    fn i16s(&mut self, count: usize, section: &'static str) -> Result<Vec<i16>, FormatError> {
        Ok(self
            .take(count * 2, section)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect())
    }
    fn i32s(&mut self, count: usize, section: &'static str) -> Result<Vec<i32>, FormatError> {
        Ok(self
            .take(count * 4, section)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    }
    fn weights(
        &mut self,
        rows: usize,
        columns: usize,
        layer: &'static str,
    ) -> Result<Vec<i8>, FormatError> {
        let padded = columns.next_multiple_of(32);
        let bytes = self.take(rows * padded, layer)?;
        let mut weights = Vec::with_capacity(rows * columns);
        for (row, values) in bytes.chunks_exact(padded).enumerate() {
            for (column, &value) in values.iter().enumerate().skip(columns) {
                if value != 0 {
                    return Err(FormatError::Padding { layer, row, column });
                }
            }
            weights.extend(values[..columns].iter().map(|&v| v as i8));
        }
        Ok(weights)
    }
    fn finish(self) -> Result<(), FormatError> {
        if !self.bytes.is_empty() {
            return Err(FormatError::TrailingBytes {
                count: self.bytes.len(),
            });
        }
        Ok(())
    }
}

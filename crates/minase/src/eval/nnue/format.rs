//! MNUE版5のバイト配置と検証を定める。
//! 多バイト数値はリトルエンディアンであり、ヘッダは192バイトである。
//! オフセット0は識別子`MNUE`、4は版u32の5、8はASCIIの構造記述96バイトである。
//! 104は幅Hのu32で2〜512の偶数、108は勝率尺度（1ロジットあたりのcp）のf32で正の有限値かつ100,000以下である。
//! 112はUTF-8の規則セット名32バイト、144は本体のSHA-256の32バイト、176は0の予約16バイトである。
//! 文字列の余りはNULで埋める。
//!
//! 本体は線形`i16[13680][2]`、探索用駒価値`i32[47]`、第1層`i16[D][H]`、バイアス`i32[H]`、出力重み`i16[2H]`、出力バイアスi32の順である。
//! Dは入力特徴数であり、直接出力部のファイル長は`192 + 13,680×4 + 47×4 + D×2H + 4H + 4H + 4`となる。
//! 長さはヘッダの幅と型の構造から導いて検査し、固定のファイルサイズとは比較しない。
//! 版1〜4は受理しない。
//!
//! 本体の先頭を0としたバイト位置は、線形重みが0、第1層より前の駒価値が54,720、
//! 第1層が54,908、バイアスが`54,908 + 2DH`、出力重みが`54,908 + 2DH + 4H`、
//! 出力バイアスが`54,908 + 2DH + 8H`であり、ファイル先頭からはそれぞれ192を足す。
//! 線形重みは特徴番号順に序中盤、終盤の順で並び、尺度は1/8センチポーンである。
//! 第1層は特徴番号ごとにH個を連続して置き、重みとバイアスの絶対値は1,016以下とする。
//! 直接出力部の重みは手番側H個、相手側H個の順で、絶対値は8,192以下とする。
//! 出力バイアスの絶対値は4,161,536以下とする。
//! 構造記述は`features=<特徴集合名>;ft=<H>;linear=tapered2;head=<出力部名>`で、
//! 復号する型から組み立てた文字列と完全一致しなければならない。
//! 線形の鏡映共有、特徴集合ごとの共有条件、および駒価値の値域は[`Network::decode`]で検査する。

use super::feature_set::{FeatureSet, Mirror};
use super::head::{DirectScrelu, Head};
use super::{EVALUATION_LIMIT, FEATURE_COUNT, Network, PIECE_STATE_COUNT};
use crate::eval::features::{mirror_feature, piece_state};
use core::fmt;
use minase_core::{Color, PieceCode, PieceKind};
use sha2::{Digest, Sha256};
use std::marker::PhantomData;

pub(super) const HEADER_LENGTH: usize = 192;

/// MNUE版5へ書き出す量子化済みの重みとメタデータ。
#[derive(Clone, Copy)]
pub struct EncodeInput<'a> {
    /// 第1層の幅H。2以上512以下の偶数を指定する。
    pub width: u32,
    /// 勝率尺度。正の有限値で100,000以下を指定する。
    pub cp_per_logit: f32,
    /// 生成時の規則セット名。NULを含まない32バイト以下のUTF-8文字列を指定する。
    pub rule_set: &'a str,
    /// 駒状態番号順の探索用駒価値。
    pub piece_values: &'a [i32; PIECE_STATE_COUNT],
    /// 基底特徴ごとの序中盤用と終盤用の線形重み。要素数は13,680。
    pub linear: &'a [[i16; 2]],
    /// 特徴ごとにH個が連続する第1層の重み。要素数は入力特徴数×H。
    pub weights: &'a [i16],
    /// 第1層のバイアス。要素数はH。
    pub biases: &'a [i32],
    /// 手番側視点のH個に相手側視点のH個が続く出力重み。
    pub output_weights: &'a [i16],
    /// 出力バイアス。
    pub output_bias: i32,
}

/// 入力特徴集合`F`と直接出力部の重みを検証し、MNUE版5へ符号化する。
///
/// 構造記述文字列は`F`の名前と幅から組み立て、本体のSHA-256をヘッダへ格納する。
/// 量子化済み整数を受け取り、復号と同じ範囲と鏡映共有を検査する。
/// 実数への尺度の乗算と偶数丸めは学習器の`nnue::model::quantize`が行い、書き出し直後に復号して再検査する。
///
/// # Errors
/// 幅、勝率尺度、文字列、配列長、駒価値、重みの範囲または鏡映共有が不正ならエラーを返す。
pub fn encode<F: FeatureSet>(input: &EncodeInput<'_>) -> Result<Vec<u8>, Error> {
    let width = input.width as usize;
    let text = architecture_text::<F, DirectScrelu>(width);
    if text.len() > 96 {
        return Err(Error::InvalidArchitecture);
    }
    let mut header = [0_u8; HEADER_LENGTH];
    header[..4].copy_from_slice(b"MNUE");
    header[4..8].copy_from_slice(&5_u32.to_le_bytes());
    header[8..8 + text.len()].copy_from_slice(text.as_bytes());
    header[104..108].copy_from_slice(&input.width.to_le_bytes());
    architecture(&header)?;
    validate_cp_per_logit(input.cp_per_logit)?;
    header[108..112].copy_from_slice(&input.cp_per_logit.to_le_bytes());
    if input.rule_set.len() > 32 || input.rule_set.as_bytes().contains(&0) {
        return Err(Error::InvalidRuleSet);
    }
    header[112..112 + input.rule_set.len()].copy_from_slice(input.rule_set.as_bytes());

    let invalid_dimensions = Error::InvalidDimensions {
        actual: F::DIMENSIONS,
    };
    let weight_count = F::DIMENSIONS
        .checked_mul(width)
        .ok_or_else(|| invalid_dimensions.clone())?;
    for (parameter, expected, actual) in [
        ("linear", FEATURE_COUNT, input.linear.len()),
        ("weights", weight_count, input.weights.len()),
        ("biases", width, input.biases.len()),
        ("output_weights", 2 * width, input.output_weights.len()),
    ] {
        if actual != expected {
            return Err(Error::InvalidParameterLength {
                parameter,
                expected,
                actual,
            });
        }
    }
    let length = weight_count
        .checked_mul(2)
        .and_then(|bytes| {
            bytes.checked_add(
                HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4 + 8 * width + 4,
            )
        })
        .filter(|&bytes| bytes <= isize::MAX as usize)
        .ok_or(invalid_dimensions)?;
    validate_piece_values(input.piece_values)?;
    validate_features::<F>(width, input.linear, input.weights, input.biases)?;

    // 出力区画は最大2,052バイト。既存の復号と同じ範囲・共有条件で検査する。
    let mut output = Vec::with_capacity(DirectScrelu::byte_len(width));
    for value in input.output_weights {
        output.extend_from_slice(&value.to_le_bytes());
    }
    output.extend_from_slice(&input.output_bias.to_le_bytes());
    DirectScrelu::decode(&output, width, F::MIRROR == Mirror::Shared)?;

    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(&header);
    for pair in input.linear {
        for value in pair {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for value in input.piece_values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in input.weights {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in input.biases {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&output);
    let checksum = Sha256::digest(&bytes[HEADER_LENGTH..]);
    bytes[144..176].copy_from_slice(&checksum);
    Ok(bytes)
}

/// `features=<入力特徴集合名>;ft=<H>;linear=tapered2;head=<出力部名>`を組み立てる。
/// `linear=tapered2`は基底特徴上の2端点を盤上総駒数で補間する線形の経路を表す。
fn architecture_text<F: FeatureSet, H: Head>(width: usize) -> String {
    format!(
        "features={};ft={width};linear=tapered2;head={}",
        F::NAME,
        H::NAME
    )
}

/// ヘッダの版、幅、ASCIIおよびNUL埋めを検証し、構造記述文字列を返す。
/// 本体を読む側は、型から組み立てた文字列との完全一致も検査する。
pub fn architecture(bytes: &[u8]) -> Result<&str, Error> {
    if bytes.len() < HEADER_LENGTH {
        return Err(Error::InvalidLength {
            expected: HEADER_LENGTH,
            actual: bytes.len(),
        });
    }
    let magic = bytes[..4].try_into().expect("checked header length");
    if magic != *b"MNUE" {
        return Err(Error::InvalidMagic { actual: magic });
    }
    let version = read_u32(bytes, 4);
    if version != 5 {
        return Err(Error::UnsupportedVersion { actual: version });
    }
    let width = read_u32(bytes, 104);
    if !(2..=512).contains(&width) || !width.is_multiple_of(2) {
        return Err(Error::InvalidWidth { actual: width });
    }
    let field = &bytes[8..104];
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    if !field[..end].is_ascii() || field[end..].iter().any(|&byte| byte != 0) {
        return Err(Error::InvalidArchitecture);
    }
    Ok(std::str::from_utf8(&field[..end]).expect("validated ASCII"))
}

impl<F: FeatureSet, H: Head> Network<F, H> {
    /// MNUE版5を検証して復号する。
    /// 識別子、版、幅、型から組み立てた構造記述との完全一致、勝率尺度、予約域0、規則セット名、構造から導いた長さ、本体のSHA-256を検査する。
    /// 第1層の重みとバイアスは絶対値1,016以下、直接出力部の重みは8,192以下、バイアスは4,161,536以下を要求する。
    /// 全型で線形の鏡映対を検査し、[`Mirror::Shared`]では第1層、バイアス、出力重みの半幅の共有も検査する。
    /// 駒価値には[`Self::piece_value`]の正値性と王駒の条件を課す。
    /// 換算係数は`round_ties_even(f64(cp_per_logit)×65,536/(127×4,096))`で求める。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let actual_architecture = architecture(bytes)?;
        let width = read_u32(bytes, 104);
        let width = width as usize;
        let expected_architecture = architecture_text::<F, H>(width);
        if actual_architecture != expected_architecture {
            return Err(Error::UnexpectedArchitecture {
                actual: actual_architecture.to_owned(),
            });
        }
        let cp_per_logit =
            f32::from_le_bytes(bytes[108..112].try_into().expect("checked header length"));
        validate_cp_per_logit(cp_per_logit)?;
        if bytes[176..192].iter().any(|&b| b != 0) {
            return Err(Error::NonZeroReserved);
        }
        let rule_set: [u8; 32] = bytes[112..144].try_into().expect("checked header length");
        let end = rule_set.iter().position(|&b| b == 0).unwrap_or(32);
        if rule_set[end..].iter().any(|&b| b != 0) || std::str::from_utf8(&rule_set[..end]).is_err()
        {
            return Err(Error::InvalidRuleSet);
        }
        let expected = HEADER_LENGTH
            + FEATURE_COUNT * 4
            + PIECE_STATE_COUNT * 4
            + F::DIMENSIONS * 2 * width
            + 4 * width
            + H::byte_len(width);
        if bytes.len() != expected {
            return Err(Error::InvalidLength {
                expected,
                actual: bytes.len(),
            });
        }
        let body = &bytes[HEADER_LENGTH..];
        let checksum: [u8; 32] = bytes[144..176].try_into().expect("checked header length");
        let actual: [u8; 32] = Sha256::digest(body).into();
        if checksum != actual {
            return Err(Error::ChecksumMismatch);
        }
        // 全体の長さを検査済みなので、以下の固定長読み出しは常に範囲内。
        let linear: Box<[[i16; 2]]> = body[..FEATURE_COUNT * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| [read_i16(b, 0), read_i16(b, 2)])
            .collect();
        let mut offset = FEATURE_COUNT * 4;
        let piece_values = std::array::from_fn(|state| read_i32(body, offset + state * 4));
        validate_piece_values(&piece_values)?;
        offset += PIECE_STATE_COUNT * 4;
        let weights: Box<[i16]> = body[offset..offset + F::DIMENSIONS * width * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| read_i16(b, 0))
            .collect();
        offset += F::DIMENSIONS * width * 2;
        let biases: Box<[i32]> = body[offset..offset + width * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| read_i32(b, 0))
            .collect();
        offset += width * 4;
        let head = H::decode(&body[offset..], width, F::MIRROR == Mirror::Shared)?;
        validate_features::<F>(width, &linear, &weights, &biases)?;
        let max_promotion_gain = PieceKind::ALL
            .into_iter()
            .filter_map(|kind| PieceCode::new(Color::Black, kind))
            .filter_map(|piece| {
                piece.promote().map(|promoted| {
                    piece_values[piece_state(promoted)] - piece_values[piece_state(piece)]
                })
            })
            .fold(0, i32::max);
        let scale =
            (f64::from(cp_per_logit) * 65_536.0 / (127.0 * 4_096.0)).round_ties_even() as i64;
        Ok(Self {
            accumulator_stats: Default::default(),
            linear,
            weights,
            biases,
            head,
            feature_set: PhantomData,
            width,
            scale,
            rule_set,
            piece_values,
            max_promotion_gain,
            cp_per_logit,
            checksum,
        })
    }
}

fn validate_cp_per_logit(cp_per_logit: f32) -> Result<(), Error> {
    if !cp_per_logit.is_finite() || cp_per_logit <= 0.0 || cp_per_logit > 100_000.0 {
        return Err(Error::InvalidCpPerLogit {
            actual: cp_per_logit,
        });
    }
    Ok(())
}

/// 長さを検査済みの特徴重みについて、範囲と鏡映共有を検査する。
fn validate_features<F: FeatureSet>(
    width: usize,
    linear: &[[i16; 2]],
    weights: &[i16],
    biases: &[i32],
) -> Result<(), Error> {
    if F::MIRROR == Mirror::Shared && F::DIMENSIONS < FEATURE_COUNT {
        return Err(Error::InvalidDimensions {
            actual: F::DIMENSIONS,
        });
    }
    validate_range("weights", weights.iter().copied().map(i32::from), 1_016)?;
    validate_range("biases", biases.iter().copied(), 1_016)?;
    for feature in 0..FEATURE_COUNT {
        let mirrored = mirror_feature(feature);
        if linear[feature] != linear[mirrored] {
            return Err(Error::MirrorMismatch {
                parameter: "linear",
                index: feature,
            });
        }
        if F::MIRROR != Mirror::Shared {
            continue;
        }
        for j in 0..width {
            if weights[mirrored * width + j] != weights[feature * width + (j + width / 2) % width] {
                return Err(Error::MirrorMismatch {
                    parameter: "weights",
                    index: feature * width + j,
                });
            }
        }
    }
    if F::MIRROR == Mirror::Shared {
        for j in 0..width {
            let partner = (j + width / 2) % width;
            if biases[j] != biases[partner] {
                return Err(Error::MirrorMismatch {
                    parameter: "biases",
                    index: j,
                });
            }
        }
    }
    Ok(())
}

pub(super) fn validate_range(
    parameter: &'static str,
    values: impl IntoIterator<Item = i32>,
    limit: i32,
) -> Result<(), Error> {
    for (index, value) in values.into_iter().enumerate() {
        if !(-limit..=limit).contains(&value) {
            return Err(Error::ParameterOutOfRange {
                parameter,
                index,
                value,
            });
        }
    }
    Ok(())
}

pub(super) fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("validated body length"),
    )
}
pub(super) fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated body length"),
    )
}

/// MNUEの符号化・復号における検証失敗。
#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    /// ファイル長がヘッダの幅から導いた長さに一致しない。
    InvalidLength {
        /// 期待するファイル長。
        expected: usize,
        /// 実際のファイル長。
        actual: usize,
    },
    /// 符号化する配列の要素数が特徴数と幅から導いた値に一致しない。
    InvalidParameterLength {
        /// 配列名。
        parameter: &'static str,
        /// 期待する要素数。
        expected: usize,
        /// 実際の要素数。
        actual: usize,
    },
    /// 入力特徴数が鏡映共有に必要な行数を満たさないか、格納可能な長さを超える。
    InvalidDimensions {
        /// 入力特徴数。
        actual: usize,
    },
    /// ファイル識別子がMNUEではない。
    InvalidMagic {
        /// 読み取った識別子。
        actual: [u8; 4],
    },
    /// 形式の版が本実装の版と異なる。
    UnsupportedVersion {
        /// 読み取った版。
        actual: u32,
    },
    /// 構造記述文字列が96バイトを超えるか、ASCIIまたはNUL埋めの規約を満たさない。
    InvalidArchitecture,
    /// 構造記述文字列が復号する型と一致しない。
    UnexpectedArchitecture {
        /// 読み取った構造記述文字列。
        actual: String,
    },
    /// 幅が2以上512以下の偶数ではない。
    InvalidWidth {
        /// 読み取った幅。
        actual: u32,
    },
    /// 予約領域が0ではない。
    NonZeroReserved,
    /// 重みまたはバイアスが許容範囲を超える。
    ParameterOutOfRange {
        /// 配列名。
        parameter: &'static str,
        /// 配列内の添字。
        index: usize,
        /// 読み取った値。
        value: i32,
    },
    /// 鏡映による重み共有が成立しない。
    MirrorMismatch {
        /// 配列名。
        parameter: &'static str,
        /// 配列内の添字。
        index: usize,
    },
    /// 規則セット名欄が32バイトを超えるか、UTF-8またはNUL埋めの規約を満たさない。
    InvalidRuleSet,
    /// 勝率尺度が正の有限値ではないか、100,000を超える。
    InvalidCpPerLogit {
        /// 読み取った勝率尺度。
        actual: f32,
    },
    /// 重み本体のSHA-256がヘッダの値と一致しない。
    ChecksumMismatch,
    /// 盤上に現れ得る非王駒の格納値が正ではない。
    NonPositivePieceValue {
        /// 格納値が正ではない駒種。
        kind: PieceKind,
        /// 成った状態かどうか。
        promoted: bool,
        /// 格納されたセンチポーン単位の駒価値。
        value: i32,
    },
    /// 王または太子の値が最大非王駒価値と歩価値の和に一致しない。
    InconsistentRoyalValue {
        /// 王駒の種類。
        kind: PieceKind,
        /// 期待する駒価値。
        expected: i32,
        /// 格納された駒価値。
        actual: i32,
    },
    /// 駒価値が探索で扱える範囲の外にある。
    PieceValueOutOfRange {
        /// 駒状態番号。
        state: usize,
        /// 格納された駒価値。
        value: i32,
    },
}

impl fmt::Display for Error {
    /// エラーの説明を整形する。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "invalid MNUE length: expected {expected}, got {actual}"
                )
            }
            Self::InvalidParameterLength {
                parameter,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid MNUE {parameter} length: expected {expected}, got {actual}"
            ),
            Self::InvalidDimensions { actual } => {
                write!(formatter, "invalid MNUE input dimensions: {actual}")
            }
            Self::InvalidMagic { actual } => write!(formatter, "invalid MNUE magic: {actual:?}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported MNUE version: {actual}")
            }
            Self::InvalidArchitecture => formatter.write_str("invalid MNUE architecture field"),
            Self::UnexpectedArchitecture { actual } => {
                write!(formatter, "unsupported MNUE architecture: {actual}")
            }
            Self::InvalidWidth { actual } => write!(formatter, "invalid MNUE width: {actual}"),
            Self::NonZeroReserved => formatter.write_str("nonzero MNUE reserved field"),
            Self::ParameterOutOfRange {
                parameter,
                index,
                value,
            } => write!(formatter, "MNUE {parameter}[{index}] out of range: {value}"),
            Self::MirrorMismatch { parameter, index } => write!(
                formatter,
                "MNUE {parameter}[{index}] violates mirror sharing"
            ),
            Self::InvalidRuleSet => formatter.write_str("invalid MNUE rule-set field"),
            Self::InvalidCpPerLogit { actual } => {
                write!(formatter, "invalid MNUE cp_per_logit: {actual}")
            }
            Self::ChecksumMismatch => formatter.write_str("MNUE weight checksum mismatch"),
            Self::NonPositivePieceValue {
                kind,
                promoted,
                value,
            } => write!(
                formatter,
                "non-positive MNUE piece value: kind={kind:?}, promoted={promoted}, value={value}"
            ),
            Self::InconsistentRoyalValue {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "inconsistent MNUE royal value: kind={kind:?}, expected {expected}, got {actual}"
            ),
            Self::PieceValueOutOfRange { state, value } => write!(
                formatter,
                "MNUE piece value out of range: state={state}, value={value}"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// 格納された固定の探索用駒価値が[`Network::piece_value`]の不変条件を満たすかを検査する。
/// 全要素を0以上[`EVALUATION_LIMIT`]以下に制限し、盤上に現れ得る非王駒の正値性と王駒価値の式を確かめる。
/// 盤上に現れない未使用の駒状態には0を許す。
fn validate_piece_values(values: &[i32; PIECE_STATE_COUNT]) -> Result<(), Error> {
    for (state, &value) in values.iter().enumerate() {
        if !(0..=EVALUATION_LIMIT).contains(&value) {
            return Err(Error::PieceValueOutOfRange { state, value });
        }
    }

    let mut max_non_royal = 0;
    for kind in PieceKind::ALL {
        if kind.can_promote() {
            let piece = PieceCode::new(Color::Black, kind)
                .expect("promotable kind must have an unpromoted code");
            validate_piece_value(values, piece_state(piece), kind, false, &mut max_non_royal)?;
            if kind.unpromoted().is_some() {
                validate_piece_value(values, kind.index(), kind, true, &mut max_non_royal)?;
            }
        } else if !matches!(kind, PieceKind::King | PieceKind::CrownPrince) {
            let promoted = PieceCode::new(Color::Black, kind).is_none();
            validate_piece_value(values, kind.index(), kind, promoted, &mut max_non_royal)?;
        }
    }

    let pawn =
        PieceCode::new(Color::Black, PieceKind::Pawn).expect("pawn must have an unpromoted code");
    let pawn_value = values[piece_state(pawn)];
    let royal_value = max_non_royal + pawn_value;
    for kind in [PieceKind::King, PieceKind::CrownPrince] {
        let actual = values[kind.index()];
        if actual != royal_value {
            return Err(Error::InconsistentRoyalValue {
                kind,
                expected: royal_value,
                actual,
            });
        }
    }
    Ok(())
}

/// 盤上に現れ得る非王駒の値を検査し、最大値を更新する。
fn validate_piece_value(
    values: &[i32; PIECE_STATE_COUNT],
    state: usize,
    kind: PieceKind,
    promoted: bool,
    max_non_royal: &mut i32,
) -> Result<(), Error> {
    let value = values[state];
    if value <= 0 {
        return Err(Error::NonPositivePieceValue {
            kind,
            promoted,
            value,
        });
    }
    *max_non_royal = (*max_non_royal).max(value);
    Ok(())
}

/// リトルエンディアンのu32を指定位置から読む。
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("slice length is fixed"),
    )
}

//! MNUE版5の配置、範囲、検査和および鏡映共有を検証する。

use super::feature_set::{FeatureSet, Mirror};
use super::head::Head;
use super::{EVALUATION_LIMIT, FEATURE_COUNT, Network, PIECE_STATE_COUNT};
use crate::eval::features::{mirror_feature, piece_state};
use core::fmt;
use minase_core::{Color, PieceCode, PieceKind};
use sha2::{Digest, Sha256};
use std::marker::PhantomData;

pub(super) const HEADER_LENGTH: usize = 192;

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
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let actual_architecture = architecture(bytes)?;
        let width = read_u32(bytes, 104);
        let width = width as usize;
        let expected_architecture = format!(
            "features={};ft={width};linear=tapered2;head={}",
            F::NAME,
            H::NAME
        );
        if actual_architecture != expected_architecture {
            return Err(Error::UnexpectedArchitecture {
                actual: actual_architecture.to_owned(),
            });
        }
        let k = f32::from_le_bytes(bytes[108..112].try_into().expect("checked header length"));
        if !k.is_finite() || k <= 0.0 || k > 100_000.0 {
            return Err(Error::InvalidK { actual: k });
        }
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
                if weights[mirrored * width + j]
                    != weights[feature * width + (j + width / 2) % width]
                {
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
        let max_promotion_gain = PieceKind::ALL
            .into_iter()
            .filter_map(|kind| PieceCode::new(Color::Black, kind))
            .filter_map(|piece| {
                piece.promote().map(|promoted| {
                    piece_values[piece_state(promoted)] - piece_values[piece_state(piece)]
                })
            })
            .fold(0, i32::max);
        let scale = (f64::from(k) * 65_536.0 / (127.0 * 4_096.0)).round_ties_even() as i64;
        Ok(Self {
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
            k,
            checksum,
        })
    }
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

/// MNUEバイト列の検証失敗。
#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    /// ファイル長がヘッダの幅から導いた長さに一致しない。
    InvalidLength {
        /// 期待するファイル長。
        expected: usize,
        /// 実際のファイル長。
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
    /// 構造記述文字列がASCIIまたはNUL埋めの規約を満たさない。
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
    /// 規則セット名欄がUTF-8またはNUL埋めの規約を満たさない。
    InvalidRuleSet,
    /// 勝率尺度Kが正の有限値ではないか、100,000を超える。
    InvalidK {
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
            Self::InvalidK { actual } => write!(formatter, "invalid MNUE K: {actual}"),
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

/// 格納された駒価値を検査する。
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

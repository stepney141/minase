//! MNPT形式の学習PST重みを復号し、ヘッダと駒価値を検証する。

use core::fmt;

use sha2::{Digest, Sha256};

use super::features::{FEATURE_COUNT, piece_state};
use super::fm::{ENCODED_LENGTH, FM_RANK, Fm};
use super::{EVALUATION_LIMIT, PIECE_STATE_COUNT, Pst};
use minase_core::{Color, PieceCode, PieceKind};

/// MNPTヘッダのバイト数。
pub(super) const HEADER_LENGTH: usize = 80;
/// 対応するMNPT形式の版。
const FORMAT_VERSION: u32 = 3;

impl Pst {
    /// 復号時の版と規則セットを保ち、MNPTバイト列へ符号化する。
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MNPT");
        bytes
            .extend_from_slice(&(if self.fm.is_some() { FORMAT_VERSION } else { 2 }).to_le_bytes());
        bytes.extend_from_slice(&(FEATURE_COUNT as u32).to_le_bytes());
        bytes.extend_from_slice(&self.k.to_le_bytes());
        bytes.extend_from_slice(&self.rule_set);
        bytes.extend_from_slice(&[0; 32]);
        for endpoint in 0..2 {
            for pair in &self.weights {
                bytes.extend_from_slice(&pair[endpoint].to_le_bytes());
            }
        }
        for value in self.piece_values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        if let Some(fm) = &self.fm {
            fm.encode_into(&mut bytes);
        }
        let checksum = Sha256::digest(&bytes[HEADER_LENGTH..]);
        bytes[48..80].copy_from_slice(&checksum);
        bytes
    }

    /// MNPTバイト列を検証し、学習PSTへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let expected_length = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        if bytes.len() < HEADER_LENGTH {
            return Err(Error::InvalidLength {
                expected: expected_length,
                actual: bytes.len(),
            });
        }
        let magic = bytes[0..4].try_into().expect("slice length is fixed");
        if magic != *b"MNPT" {
            return Err(Error::InvalidMagic { actual: magic });
        }
        let version = read_u32(bytes, 4);
        if !matches!(version, 2 | FORMAT_VERSION) {
            return Err(Error::UnsupportedVersion { actual: version });
        }
        let expected_length = expected_length
            + if version == FORMAT_VERSION {
                ENCODED_LENGTH
            } else {
                0
            };
        if bytes.len() != expected_length {
            return Err(Error::InvalidLength {
                expected: expected_length,
                actual: bytes.len(),
            });
        }
        let feature_count = read_u32(bytes, 8);
        if feature_count != FEATURE_COUNT as u32 {
            return Err(Error::UnexpectedFeatureCount {
                actual: feature_count,
            });
        }
        let k = f32::from_le_bytes(bytes[12..16].try_into().expect("slice length is fixed"));
        if !k.is_finite() || k <= 0.0 {
            return Err(Error::InvalidK { actual: k });
        }
        let rule_set = &bytes[16..48];
        let rule_end = rule_set.iter().position(|&byte| byte == 0).unwrap_or(32);
        if rule_set[rule_end..].iter().any(|&byte| byte != 0)
            || std::str::from_utf8(&rule_set[..rule_end]).is_err()
        {
            return Err(Error::InvalidRuleSet);
        }
        let body = &bytes[HEADER_LENGTH..];
        let checksum: [u8; 32] = bytes[48..80].try_into().expect("slice length is fixed");
        let actual_checksum: [u8; 32] = Sha256::digest(body).into();
        if actual_checksum != checksum {
            return Err(Error::ChecksumMismatch);
        }

        let mut weights = [[0_i16; 2]; FEATURE_COUNT];
        for (feature, pair) in weights.iter_mut().enumerate() {
            for (endpoint, weight) in pair.iter_mut().enumerate() {
                let offset = (endpoint * FEATURE_COUNT + feature) * 2;
                *weight = i16::from_le_bytes(
                    body[offset..offset + 2]
                        .try_into()
                        .expect("slice length is fixed"),
                );
            }
        }
        let mut piece_values = [0_i32; PIECE_STATE_COUNT];
        for (state, value) in piece_values.iter_mut().enumerate() {
            let offset = FEATURE_COUNT * 4 + state * 4;
            *value = i32::from_le_bytes(
                body[offset..offset + 4]
                    .try_into()
                    .expect("slice length is fixed"),
            );
        }
        validate_piece_values(&piece_values)?;
        let max_promotion_gain = PieceKind::ALL
            .into_iter()
            .filter_map(|kind| PieceCode::new(Color::Black, kind))
            .filter_map(|piece| {
                piece.promote().map(|promoted| {
                    piece_values[piece_state(promoted)] - piece_values[piece_state(piece)]
                })
            })
            .fold(0, i32::max);
        Ok(Self {
            fm: if version == FORMAT_VERSION {
                Some(Fm::decode(
                    &body[FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4..],
                )?)
            } else {
                None
            },
            rule_set: rule_set.try_into().expect("validated rule-set length"),
            max_promotion_gain,
            weights,
            piece_values,
            k,
            checksum,
        })
    }
}

/// MNPTバイト列の検証失敗。
#[derive(Clone, PartialEq, Debug)]
pub enum Error {
    /// ファイル長がヘッダと重み本体の固定長に一致しない。
    InvalidLength {
        /// 期待するファイル長。
        expected: usize,
        /// 実際のファイル長。
        actual: usize,
    },
    /// ファイル識別子がMNPTではない。
    InvalidMagic {
        /// 読み取った識別子。
        actual: [u8; 4],
    },
    /// 形式の版が本実装の版と異なる。
    UnsupportedVersion {
        /// 読み取った版。
        actual: u32,
    },
    /// 特徴数が本実装の特徴数と異なる。
    UnexpectedFeatureCount {
        /// 読み取った特徴数。
        actual: u32,
    },
    /// FMの潜在次元が埋め込み候補の次元と異なる。
    UnexpectedFmRank {
        /// 読み取った次元。
        actual: u32,
    },
    /// FMの量子化指数が0から30の範囲外である。
    InvalidFmExponent {
        /// 読み取った指数。
        actual: u32,
    },
    /// FMの符号が+1でも−1でもない。
    InvalidFmSign {
        /// 不正な符号の潜在次元番号。
        dimension: usize,
        /// 読み取った符号。
        actual: i8,
    },
    /// 規則セット名欄がUTF-8またはNUL埋めの規約を満たさない。
    InvalidRuleSet,
    /// 勝率尺度Kが正の有限値ではない。
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
                    "invalid MNPT length: expected {expected}, got {actual}"
                )
            }
            Self::InvalidMagic { actual } => write!(formatter, "invalid MNPT magic: {actual:?}"),
            Self::UnsupportedVersion { actual } => {
                write!(formatter, "unsupported MNPT version: {actual}")
            }
            Self::UnexpectedFeatureCount { actual } => write!(
                formatter,
                "invalid MNPT feature count: expected {FEATURE_COUNT}, got {actual}"
            ),
            Self::UnexpectedFmRank { actual } => write!(
                formatter,
                "invalid MNPT FM rank: expected {FM_RANK}, got {actual}"
            ),
            Self::InvalidFmExponent { actual } => {
                write!(formatter, "invalid MNPT FM exponent: {actual}")
            }
            Self::InvalidFmSign { dimension, actual } => write!(
                formatter,
                "invalid MNPT FM sign at dimension {dimension}: {actual}"
            ),
            Self::InvalidRuleSet => formatter.write_str("invalid MNPT rule-set field"),
            Self::InvalidK { actual } => write!(formatter, "invalid MNPT K: {actual}"),
            Self::ChecksumMismatch => formatter.write_str("MNPT weight checksum mismatch"),
            Self::NonPositivePieceValue {
                kind,
                promoted,
                value,
            } => write!(
                formatter,
                "non-positive MNPT piece value: kind={kind:?}, promoted={promoted}, value={value}"
            ),
            Self::InconsistentRoyalValue {
                kind,
                expected,
                actual,
            } => write!(
                formatter,
                "inconsistent MNPT royal value: kind={kind:?}, expected {expected}, got {actual}"
            ),
            Self::PieceValueOutOfRange { state, value } => write!(
                formatter,
                "MNPT piece value out of range: state={state}, value={value}"
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

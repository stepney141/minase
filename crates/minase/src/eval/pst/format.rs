//! MNPT版3のバイト配置と検証を定める。
//! 多バイト数値はリトルエンディアンで、ヘッダは80バイトである。
//! ヘッダの`0..4`は識別子`MNPT`、`4..8`は版u32の3、`8..12`は特徴数u32の13,680である。
//! `12..16`は勝率尺度Kのf32で正の有限値、`16..48`はUTF-8の規則セット名である。
//! 規則名はNULを含まない最大32バイトの文字列で、空文字列も認め、不足分を0で埋める。
//! `48..80`はFM節を含む本体全体（オフセット80から末尾まで）のSHA-256の32バイトである。
//!
//! 本体は序中盤のPST重み`i16[13680]`、終盤のPST重み`i16[13680]`、
//! 探索用駒価値`i32[47]`、FM節の順に並ぶ。
//! 本体先頭からの位置は順に0、27,360、54,720、54,908であり、ファイル先頭からは80を足す。
//! PST重みは特徴番号順、単位は1/8センチポーンで、i16の全範囲を受理する。
//! 駒価値は駒状態番号順、単位はセンチポーンで、0〜28,999とする。
//! 盤上に現れ得る非王駒の値は正で、王と太子の値は最大非王駒価値と歩価値の和に一致しなければならない。
//!
//! FM節の`0..4`は次元Dのu32で0または32、`4..8`は量子化尺度の2進指数のu32である。
//! Dが0なら指数も0とし、FM節はこの8バイトだけで終わる。復号後のFMはNoneとなる。
//! Dが32なら指数は0〜30とし、続けて各次元の符号`i8[32]`（+1または−1）、
//! 特徴番号ごとに32個を連続して置く埋め込み`i16[13680][32]`を格納する。
//! 埋め込みはi16の全範囲を受理し、自己相互作用項は復号時に導出する。
//! FM節の長さは`8 + D + 13,680×D×2`バイトである。
//! 読み手は次元の欄まで読めることを確認してからDを検査し、
//! ファイル長が`80 + 13,680×4 + 47×4 + 8 + D + 13,680×D×2`と一致することを確かめる。
//! 版3以外、未定義の次元・指数・符号、切り詰め、余分なバイト、検査和の不一致は拒否する。

use core::fmt;

use sha2::{Digest, Sha256};

use super::features::{FEATURE_COUNT, piece_state};
use super::fm::{FM_RANK, Fm};
use super::{EVALUATION_LIMIT, PIECE_STATE_COUNT, Pst};
use minase_core::{Color, PieceCode, PieceKind};

/// MNPTヘッダのバイト数。
pub(super) const HEADER_LENGTH: usize = 80;
/// 対応するMNPT形式の版。
const FORMAT_VERSION: u32 = 3;

impl Pst {
    /// 学習済みの2端点と探索用駒価値から、FM補正を持たないMNPT版3を作る。
    ///
    /// 重みは特徴番号順、各行は序中盤・終盤の順で、単位は1/8センチポーン。
    /// 配列長、駒価値、正の有限なK、32バイト以内でNULを含まない規則名を検査する。
    /// 鏡映対称性は要求しない。検査和と探索用の派生値は復号時と同じ方法で求める。
    pub fn from_weights(
        weights: &[[i16; 2]],
        piece_values: [i32; PIECE_STATE_COUNT],
        k: f32,
        rule_set: &str,
    ) -> Result<Self, Error> {
        let weights = weights
            .try_into()
            .map_err(|_| Error::UnexpectedFeatureCount {
                actual: weights.len() as u32,
            })?;
        if rule_set.len() > 32 || rule_set.as_bytes().contains(&0) {
            return Err(Error::InvalidRuleSet);
        }
        let mut rules = [0; 32];
        rules[..rule_set.len()].copy_from_slice(rule_set.as_bytes());
        let candidate = Self {
            weights,
            fm: None,
            rule_set: rules,
            piece_values,
            max_promotion_gain: 0,
            k,
            checksum: [0; 32],
        };
        Self::decode(&candidate.encode())
    }

    /// 規則セットを保ち、MNPT版3へ符号化する。
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MNPT");
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
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
        } else {
            bytes.extend_from_slice(&[0; 8]);
        }
        let checksum = Sha256::digest(&bytes[HEADER_LENGTH..]);
        bytes[48..80].copy_from_slice(&checksum);
        bytes
    }

    /// MNPTバイト列を検証し、学習PSTへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let fm_offset = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
        if bytes.len() < HEADER_LENGTH {
            return Err(Error::InvalidLength {
                expected: HEADER_LENGTH,
                actual: bytes.len(),
            });
        }
        let magic = bytes[0..4].try_into().expect("slice length is fixed");
        if magic != *b"MNPT" {
            return Err(Error::InvalidMagic { actual: magic });
        }
        let version = read_u32(bytes, 4);
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion { actual: version });
        }
        if bytes.len() < fm_offset + 4 {
            return Err(Error::InvalidLength {
                expected: fm_offset + 4,
                actual: bytes.len(),
            });
        }
        let rank = read_u32(bytes, fm_offset);
        if rank != 0 && rank != FM_RANK as u32 {
            return Err(Error::UnexpectedFmRank { actual: rank });
        }
        let expected_length = fm_offset + 8 + rank as usize * (1 + FEATURE_COUNT * 2);
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
            fm: if rank != 0 {
                Some(Fm::decode(
                    &body[FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4..],
                )?)
            } else {
                let exponent = read_u32(bytes, fm_offset + 4);
                if exponent != 0 {
                    return Err(Error::InvalidFmExponent { actual: exponent });
                }
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
    /// ファイル長がヘッダとFMの次元から導いた長さに一致しない。
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
    /// FMの潜在次元が0でも32でもない。
    UnexpectedFmRank {
        /// 読み取った次元。
        actual: u32,
    },
    /// FMの量子化指数が、次元0では0、次元32では0〜30という条件を満たさない。
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
                "invalid MNPT FM rank: expected 0 or {FM_RANK}, got {actual}"
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

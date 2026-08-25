//! 量子化した学習PSTの復号、埋め込みおよび整数評価。

use core::fmt;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use super::features::{FEATURE_COUNT, active_features};
use crate::Position;

/// MNPTヘッダのバイト数。
const HEADER_LENGTH: usize = 80;
/// 対応するMNPT形式の版。
const FORMAT_VERSION: u32 = 1;
/// 静的評価値の絶対値上限。
const EVALUATION_LIMIT: i32 = 28_999;

/// 学習PSTの量子化重みと勝率尺度。
pub struct Pst {
    /// 1/8センチポーン単位の特徴重み。
    weights: [i16; FEATURE_COUNT],
    /// 学習時に使ったセンチポーンから勝率ロジットへの尺度。
    k: f32,
}

impl Pst {
    /// MNPTバイト列を検証し、学習PSTへ復号する。
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let expected_length = HEADER_LENGTH + FEATURE_COUNT * 2;
        if bytes.len() != expected_length {
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
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion { actual: version });
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
        let body = &bytes[HEADER_LENGTH..];
        let expected_checksum: [u8; 32] = bytes[48..80].try_into().expect("slice length is fixed");
        let actual_checksum: [u8; 32] = Sha256::digest(body).into();
        if actual_checksum != expected_checksum {
            return Err(Error::ChecksumMismatch);
        }

        let mut weights = [0_i16; FEATURE_COUNT];
        for (index, weight) in weights.iter_mut().enumerate() {
            let offset = index * 2;
            *weight = i16::from_le_bytes(
                body[offset..offset + 2]
                    .try_into()
                    .expect("slice length is fixed"),
            );
        }
        Ok(Self { weights, k })
    }

    /// 学習時に使った勝率尺度Kを返す。
    pub const fn k(&self) -> f32 {
        self.k
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
    /// 勝率尺度Kが正の有限値ではない。
    InvalidK {
        /// 読み取った勝率尺度。
        actual: f32,
    },
    /// 重み本体のSHA-256がヘッダの値と一致しない。
    ChecksumMismatch,
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
            Self::InvalidK { actual } => write!(formatter, "invalid MNPT K: {actual}"),
            Self::ChecksumMismatch => formatter.write_str("MNPT weight checksum mismatch"),
        }
    }
}

impl std::error::Error for Error {}

/// 学習PSTで局面を手番側の視点からセンチポーン評価する。
pub fn evaluate(pst: &Pst, position: &Position) -> i32 {
    let mut sum = 0_i32;
    active_features(position, |feature| {
        sum += i32::from(pst.weights[feature]);
    });
    (sum / 8).clamp(-EVALUATION_LIMIT, EVALUATION_LIMIT)
}

/// リトルエンディアンのu32を指定位置から読む。
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("slice length is fixed"),
    )
}

/// 実行バイナリへ埋め込むMNPTバイト列。
static EMBEDDED: &[u8] = include_bytes!("../../nets/pst.bin");
/// 埋め込み重みの復号結果を保持する領域。
static WEIGHTS: OnceLock<Result<Pst, Error>> = OnceLock::new();

/// 検証済みの埋め込み学習PSTを返す。
pub fn weights() -> Result<&'static Pst, &'static Error> {
    match WEIGHTS.get_or_init(|| Pst::decode(EMBEDDED)) {
        Ok(pst) => Ok(pst),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::handcrafted::piece_value;
    use crate::test_util::{position_from_codes, sq};
    use crate::{Color, PieceCode, PieceKind, Square};

    /// 検査用の正しいMNPTバイト列を返す。
    fn valid_bytes() -> Vec<u8> {
        include_bytes!("../../nets/pst-init.bin").to_vec()
    }

    /// 指定局面の手番側駒価値差を計算する。
    fn material_score(position: &Position) -> i32 {
        let perspective = position.side_to_move();
        Square::all()
            .filter_map(|square| position.piece_at(square))
            .map(|piece| {
                let sign = if piece.color() == Some(perspective) {
                    1
                } else {
                    -1
                };
                sign * piece_value(piece.kind().unwrap())
            })
            .sum()
    }

    /// 固定長より短いMNPTが拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_length() {
        let mut bytes = valid_bytes();
        bytes.pop();
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::InvalidLength { .. })
        ));
    }

    /// MNPT識別子の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_invalid_magic() {
        let mut bytes = valid_bytes();
        bytes[0] ^= 1;
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::InvalidMagic { .. })
        ));
    }

    /// 未対応のMNPT版が拒否されることを検査する。
    #[test]
    fn decode_rejects_unsupported_version() {
        let mut bytes = valid_bytes();
        bytes[4] ^= 1;
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::UnsupportedVersion { .. })
        ));
    }

    /// MNPT特徴数の不一致が拒否されることを検査する。
    #[test]
    fn decode_rejects_unexpected_feature_count() {
        let mut bytes = valid_bytes();
        bytes[8] ^= 1;
        assert!(matches!(
            Pst::decode(&bytes),
            Err(Error::UnexpectedFeatureCount { .. })
        ));
    }

    /// MNPT重み本体の改変がSHA-256で拒否されることを検査する。
    #[test]
    fn decode_rejects_checksum_mismatch() {
        let mut bytes = valid_bytes();
        bytes[HEADER_LENGTH] ^= 1;
        assert!(matches!(Pst::decode(&bytes), Err(Error::ChecksumMismatch)));
    }

    /// v0初期重みが先獅子のない局面の駒価値差と一致することを検査する。
    #[test]
    fn initialized_pst_matches_material_evaluation() {
        let pst = Pst::decode(include_bytes!("../../nets/pst-init.bin")).unwrap();
        let promoted_gold = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted_lion = PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap();
        let positions = [
            Position::initial(),
            position_from_codes(
                Color::Black,
                &[
                    (sq(1, 2), promoted_gold),
                    (sq(7, 8), promoted_lion),
                    (sq(4, 6), PieceCode::new(Color::White, PieceKind::King)),
                ],
            ),
            position_from_codes(
                Color::White,
                &[
                    (sq(0, 0), PieceCode::new(Color::Black, PieceKind::Pawn)),
                    (
                        sq(11, 11),
                        PieceCode::new(Color::White, PieceKind::FreeKing),
                    ),
                ],
            ),
        ];
        for position in positions {
            assert_eq!(evaluate(&pst, &position), material_score(&position));
        }
    }

    /// 埋め込み重みが復号でき、初期局面評価がPython学習器と一致することを検査する。
    #[test]
    fn embedded_pst_matches_python_initial_position_evaluation() {
        assert_eq!(evaluate(weights().unwrap(), &Position::initial()), 12);
    }
}

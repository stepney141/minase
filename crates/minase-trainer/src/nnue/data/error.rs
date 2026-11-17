use std::{fmt, io};

use minase::training::records;

/// NNUEの特徴符号化またはバッチ構築の失敗。
#[derive(Debug)]
pub enum Error {
    /// 評価用または合成局面の基準王駒が一意でない。
    Anchor(minase::eval::nnue::input::AnchorError),
    /// 教師局面の契約違反。元ファイル、対局番号、手数を保持する。
    TeacherAnchor {
        /// 元の教師ファイル。
        path: std::path::PathBuf,
        /// 対局番号。
        game: u32,
        /// 手数。
        ply: u16,
        /// 基準王駒を定められない原因。
        error: minase::eval::nnue::input::AnchorError,
    },
    /// ワーカーの起動またはファイル操作に失敗した。
    Io(io::Error),
    /// 共有データの読み込み、局面復元または教師値の構築に失敗した。
    Data(crate::data::Error),
    /// バッチの局面数が1以上、符号化可能な上限以下ではない。
    BatchSize {
        /// 要求された局面数。
        size: usize,
    },
    /// 逆索引の局面番号、視点または列が範囲外にある。
    PackedOccurrence,
    /// P型の符号化された特徴が範囲外にある。
    PFeature {
        /// 不正な符号化値。
        value: u32,
    },
    /// バッチ構築のワーカーがパニックした。
    WorkerPanicked,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Anchor(error) => error.fmt(f),
            Self::TeacherAnchor {
                path,
                game,
                ply,
                error,
            } => write!(f, "{}: game {game}, ply {ply}: {error}", path.display()),
            Self::Io(error) => error.fmt(f),
            Self::Data(error) => error.fmt(f),
            Self::BatchSize { size } => write!(f, "invalid batch size: {size}"),
            Self::PackedOccurrence => f.write_str("inverse-index occurrence is outside its domain"),
            Self::PFeature { value } => write!(f, "invalid packed P feature: {value}"),
            Self::WorkerPanicked => f.write_str("batch worker panicked"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Anchor(error) | Self::TeacherAnchor { error, .. } => Some(error),
            Self::Io(error) => Some(error),
            Self::Data(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<crate::data::Error> for Error {
    fn from(error: crate::data::Error) -> Self {
        Self::Data(error)
    }
}

impl From<records::Error> for Error {
    fn from(error: records::Error) -> Self {
        Self::Data(error.into())
    }
}

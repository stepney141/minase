use std::{fmt, io};

use minase::training::records_v2;

/// NNUEの特徴符号化またはバッチ構築の失敗。
#[derive(Debug)]
pub enum Error {
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

impl From<records_v2::Error> for Error {
    fn from(error: records_v2::Error) -> Self {
        Self::Data(error.into())
    }
}

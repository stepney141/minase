use std::fmt;

/// PSTの学習、保存、読み込みの失敗。
#[derive(Debug)]
pub enum Error {
    /// 引数、配列または進捗が契約に違反する。
    Invalid(&'static str),
    /// 保存形式が不正である。
    Format(&'static str),
    /// ファイルの入出力に失敗した。
    Io(std::io::Error),
    /// 共有データの検査に失敗した。
    Data(crate::data::Error),
    /// 汎用の数値演算または配列の検査に失敗した。
    Nn(crate::nn::Error),
    /// JSONの処理に失敗した。
    Json(serde_json::Error),
    /// MNPTの処理に失敗した。
    Mnpt(minase::eval::pst::Error),
    /// 記録からの局面復元に失敗した。
    Record(minase::training::records::Error),
    /// 実行時刻の取得に失敗した。
    Time(std::time::SystemTimeError),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) | Self::Format(reason) => f.write_str(reason),
            Self::Io(e) => e.fmt(f),
            Self::Data(e) => e.fmt(f),
            Self::Nn(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
            Self::Mnpt(e) => e.fmt(f),
            Self::Record(e) => e.fmt(f),
            Self::Time(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(_) | Self::Format(_) => None,
            Self::Io(e) => Some(e),
            Self::Data(e) => Some(e),
            Self::Nn(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Mnpt(e) => Some(e),
            Self::Record(e) => Some(e),
            Self::Time(e) => Some(e),
        }
    }
}
macro_rules! from_error {
    ($source:ty, $variant:ident) => {
        impl From<$source> for Error {
            fn from(error: $source) -> Self {
                Self::$variant(error)
            }
        }
    };
}
from_error!(std::io::Error, Io);
from_error!(crate::data::Error, Data);
from_error!(crate::nn::Error, Nn);
from_error!(serde_json::Error, Json);
from_error!(minase::eval::pst::Error, Mnpt);
from_error!(minase::training::records::Error, Record);
from_error!(std::time::SystemTimeError, Time);

/// PSTパイプラインの処理結果。
pub type Result<T> = std::result::Result<T, Error>;

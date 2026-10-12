//! 評価関数に依存しない数値更新、損失、並列処理と配列の入出力。

mod adam;
mod io;
mod loss;
pub mod npz;
pub mod parallel;
mod stopping;

pub use adam::{AdamCoefficients, AdamWGroup};
pub use io::atomic_write;
pub use loss::{bce_with_logits, bce_with_logits_gradient};
pub use stopping::{BestEpoch, patience_reached, update_best};

/// 数値計算または配列の入出力の失敗。
#[derive(Debug)]
pub enum Error {
    /// ファイル操作またはスレッドの起動に失敗した。
    Io(std::io::Error),
    /// 引数が演算の定義域にない。
    Invalid(&'static str),
    /// 配列の保存形式が不正、または未対応。
    Format(&'static str),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Invalid(reason) | Self::Format(reason) => f.write_str(reason),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
/// 汎用部品の計算結果。
pub type Result<T> = std::result::Result<T, Error>;

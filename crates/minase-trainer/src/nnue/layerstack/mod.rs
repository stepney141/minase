//! P型LayerStackのCPU学習、再開、およびtatara形式への書き出し。
//! パラメータ、AdamWの状態、局面ごとの演算はf32、局面方向の縮約はf64とする。

mod checkpoint;
pub(crate) mod diagnostics;
mod export;
// 学習ループへの接続はフェーズ3cで行う。
#[allow(dead_code)]
pub(crate) mod gpu;
mod model;
mod optimizer;
mod reference;

pub use checkpoint::{Checkpoint, Settings};
pub use export::{QuantizationReport, export, quantization_report, quantize};
pub use model::{H, Parameter, Parameters, ROWS, Scale};
pub use optimizer::{AdamW, OptimizerConfig};
pub use reference::{Features, Forward, backward, forward, loss, train_batch};

/// 引数、数値演算、保存または書き出しの失敗。
#[derive(Debug)]
pub enum Error {
    /// 教師尺度が正の有限数でない。
    TeacherScale(f64),
    /// 丸めた出力の除数が正のi32に収まらない。
    OutputScale(f64),
    /// 配列、入力、または設定が契約に反する。
    Invalid(&'static str),
    /// 数値が有限でない。配列名と行優先の位置を持つ。
    NonFinite(&'static str, usize),
    /// 量子化した値が格納型に収まらない。
    Quantization {
        /// 展開後の配列名。共有L1では加算する両配列の名前。
        name: &'static str,
        /// 展開後の行優先の要素番号。
        index: usize,
        /// 尺度を掛けて丸めた値。
        value: f64,
    },
    /// 再開時の設定が保存された設定と異なる。
    SettingsMismatch,
    /// 共通の配列入出力または数値処理の失敗。
    Common(crate::nn::Error),
    /// 教師値または入力バッチの不正。
    Data(crate::nnue::data::Error),
    /// エンジンが量子化したネットを拒否した。
    Network(minase::eval::nnue::network::FormatError),
    /// ファイル操作の失敗。
    Io(std::io::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TeacherScale(v) => write!(f, "invalid teacher scale: {v}"),
            Self::OutputScale(v) => write!(f, "invalid output divisor: {v}"),
            Self::Invalid(s) => write!(f, "invalid LayerStack input: {s}"),
            Self::NonFinite(n, i) => write!(f, "non-finite {n}[{i}]"),
            Self::Quantization { name, index, value } => {
                write!(f, "quantization overflow: {name}[{index}] = {value}")
            }
            Self::SettingsMismatch => f.write_str("LayerStack checkpoint settings mismatch"),
            Self::Common(e) => e.fmt(f),
            Self::Data(e) => e.fmt(f),
            Self::Network(e) => e.fmt(f),
            Self::Io(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Common(e) => Some(e),
            Self::Data(e) => Some(e),
            Self::Network(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
impl From<crate::nn::Error> for Error {
    fn from(e: crate::nn::Error) -> Self {
        Self::Common(e)
    }
}
impl From<crate::nnue::data::Error> for Error {
    fn from(e: crate::nnue::data::Error) -> Self {
        Self::Data(e)
    }
}
impl From<minase::eval::nnue::network::FormatError> for Error {
    fn from(e: minase::eval::nnue::network::FormatError) -> Self {
        Self::Network(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
/// LayerStackの処理結果。
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests;

//! 入力、数値計算、入出力の失敗を原因別に保持する。

use std::{fmt, path::PathBuf};

use minase::training::records::Origin;

/// 結果回帰の入力、数値計算または書き出しの失敗。
#[derive(Debug)]
pub enum Error {
    /// ファイル操作またはスレッドの起動に失敗した。
    Io(std::io::Error),
    /// 教師ファイルまたは局面を復号できなかった。
    Data(crate::data::Error),
    /// 基点または補正したMNUEが不正だった。
    Mnue(minase::eval::nnue::Error),
    /// JSONへの符号化に失敗した。
    Json(serde_json::Error),
    /// 並列度を決定できなかった。
    Workers(crate::nn::Error),
    /// 入力ファイルが指定されていない。
    MissingData,
    /// τの候補が指定されていない。
    MissingTaus,
    /// τが正の有限値ではない。
    InvalidTau {
        /// 指定順の0始まりの番号。
        index: usize,
        /// 指定値。
        value: f64,
    },
    /// 同じτが複数指定された。
    DuplicateTau {
        /// 重複した値。
        value: f64,
    },
    /// τの逆分散を正の有限なf64として表せない。
    UnrepresentablePrecision {
        /// 指定されたτ。
        tau: f64,
    },
    /// 正規化した入力パスが重複している。
    DuplicatePath {
        /// 重複したパス。
        path: PathBuf,
    },
    /// 入力ファイルのヘッダのシードが重複している。
    DuplicateSeed {
        /// 重複したシード。
        seed: u64,
    },
    /// 乱数開始の自己対局以外、または対局のないファイルだった。
    NonRandomSelfplay {
        /// 入力パス。
        path: PathBuf,
        /// ファイルの由来。対局がなければNone。
        origin: Option<Origin>,
    },
    /// 出力先が既に空でない。
    NonEmptyOutput {
        /// 出力先。
        path: PathBuf,
    },
    /// 必要な集合に選択された対局がない。
    EmptySplit {
        /// 集合名。
        split: &'static str,
    },
    /// 読み込みスレッドが異常終了した。
    WorkerPanicked,
    /// 数値計算が非有限になった。
    NonFinite {
        /// 計算段階。
        stage: &'static str,
    },
    /// ヘッセ行列が正定値でなく、ニュートンステップを求められない。
    NotPositiveDefinite {
        /// 計算段階。
        stage: &'static str,
        /// コレスキー分解の対角の番号。
        pivot: usize,
        /// 平方根を取る前の対角値。
        value: f64,
    },
    /// 100反復で収束しなかった。
    NonConvergence {
        /// 計算段階。
        stage: &'static str,
    },
    /// 後退線形探索でArmijo条件を満たす幅を得られなかった。
    LineSearch {
        /// 計算段階。
        stage: &'static str,
    },
    /// 推定した勝率尺度が許容範囲の内部にない。
    CalibrationOutOfRange {
        /// 手数帯の番号。0が60〜79手、4が140〜159手。
        band: usize,
        /// 推定した1センチポーンあたりのロジット。
        beta: f64,
        /// 推定した1ロジットあたりのセンチポーン。
        cp_per_logit: f64,
    },
    /// 丸めた補正をi16で表せない。
    CorrectionOutOfRange {
        /// 基底特徴の番号。
        feature: usize,
        /// 1/8センチポーン単位の補正。
        value: f64,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Data(error) => error.fmt(f),
            Self::Mnue(error) => error.fmt(f),
            Self::Json(error) => error.fmt(f),
            Self::Workers(error) => error.fmt(f),
            Self::MissingData => f.write_str("at least one MNSD file is required"),
            Self::MissingTaus => f.write_str("at least one tau is required"),
            Self::InvalidTau { index, value } => {
                write!(f, "tau[{index}] must be positive and finite: {value}")
            }
            Self::DuplicateTau { value } => write!(f, "duplicate tau: {value}"),
            Self::UnrepresentablePrecision { tau } => write!(
                f,
                "tau has no representable positive finite inverse variance: {tau}"
            ),
            Self::DuplicatePath { path } => write!(f, "duplicate MNSD path: {}", path.display()),
            Self::DuplicateSeed { seed } => write!(f, "duplicate MNSD header seed: {seed}"),
            Self::NonRandomSelfplay { path, origin } => write!(
                f,
                "{}: expected random selfplay, got {origin:?}",
                path.display()
            ),
            Self::NonEmptyOutput { path } => {
                write!(f, "output directory is not empty: {}", path.display())
            }
            Self::EmptySplit { split } => write!(f, "no selected games in {split}"),
            Self::WorkerPanicked => f.write_str("MNSD reader worker panicked"),
            Self::NonFinite { stage } => write!(f, "non-finite calculation in {stage}"),
            Self::NotPositiveDefinite {
                stage,
                pivot,
                value,
            } => write!(
                f,
                "{stage} Hessian is not positive definite at {pivot}: {value}"
            ),
            Self::NonConvergence { stage } => {
                write!(f, "{stage} did not converge in 100 Newton iterations")
            }
            Self::LineSearch { stage } => write!(f, "{stage} Armijo line search failed"),
            Self::CalibrationOutOfRange {
                band,
                beta,
                cp_per_logit,
            } => write!(
                f,
                "calibration band {band}: beta={beta}, K={cp_per_logit}; require 50 < K < 4000"
            ),
            Self::CorrectionOutOfRange { feature, value } => {
                write!(f, "linear correction[{feature}] exceeds i16: {value}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Data(error) => Some(error),
            Self::Mnue(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Workers(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<crate::data::Error> for Error {
    fn from(error: crate::data::Error) -> Self {
        Self::Data(error)
    }
}
impl From<minase::eval::nnue::Error> for Error {
    fn from(error: minase::eval::nnue::Error) -> Self {
        Self::Mnue(error)
    }
}
impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// 結果回帰の計算結果。
pub type Result<T> = std::result::Result<T, Error>;

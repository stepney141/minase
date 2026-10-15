use std::{fmt, io, path::PathBuf};

use minase::training::records;

/// データ経路の失敗。下位の復号・入出力エラーは原因を保持する。
#[derive(Debug)]
pub enum Error {
    /// ファイル操作またはワーカーの起動に失敗した。
    Io(io::Error),
    /// MNSDの復号または局面復元に失敗した。
    Mnsd(records::Error),
    /// MCTSの教師値には対応していない。
    MctsTeacher {
        /// 問題のファイル。
        path: PathBuf,
    },
    /// 局面の組み立てに失敗した。
    PositionBuild(minase_core::PositionBuildError),
    /// 復元局面の不変条件に違反した。
    Position(minase_core::PositionError),
    /// 記録された麒麟成りの印が復元結果と異なる。
    LionPromotionMismatch {
        /// 記録された値。
        recorded: bool,
        /// 復元した値。
        restored: bool,
    },
    /// 1ファイルに複数の由来が含まれる。
    MixedOrigins {
        /// 問題のファイル。
        path: PathBuf,
    },
    /// 学習に使えない合成の記録が含まれる。
    SyntheticOrigin {
        /// 問題のファイル。
        path: PathBuf,
    },
    /// ファイルの内容またはファイル群の組合せが契約に違反した。
    Dataset {
        /// 問題のファイル。ファイル群が空の場合だけ空のパス。
        path: PathBuf,
        /// 違反した契約。
        reason: &'static str,
    },
    /// 通し番号がデータ集合の外にある。
    RecordIndex {
        /// 要求された通し番号。
        index: u64,
        /// データ集合の記録数。
        count: u64,
    },
    /// γまたは先読み窓の手数が定義域外にある。
    LookaheadParameters,
    /// 先読みの配列の長さが異なるか探索値が有限でない。
    LookaheadSamples,
    /// 教師の混合比が有限な0以上1以下の値ではない。
    TeacherLambda,
    /// 教師値の構築に必要な分類番号、教師の勝率尺度（1ロジットあたりのcp）または探索値が不正。
    TeacherTargets,
    /// 対局番号または同じ対局内の手数が順序規則に違反した。
    RecordOrder {
        /// 最初に違反したファイル内の記録番号。
        index: usize,
    },
    /// 教師の勝率尺度の推定に使う標本が空、長さが不一致、または値が定義域外にある。
    TeacherSamples,
    /// 探索値がすべて0で、教師の勝率尺度を決められない。
    UndeterminedScale,
    /// 較正ファイルのJSON構造または構文が不正である。
    CalibrationJson(serde_json::Error),
    /// 較正ファイルの形式識別子が一致しない。
    CalibrationFormat,
    /// 較正ファイルの教師の勝率尺度が有限で正の数ではない。
    CalibrationScale {
        /// 較正ファイル内の項目番号。
        entry: usize,
    },
    /// λが0でない教師分類に対応する較正項目がない。
    MissingCalibrationClass {
        /// 教師の分類番号。
        class: usize,
    },
    /// 較正項目に対応する教師分類が入力にない。
    UnexpectedCalibrationClass {
        /// 較正ファイル内の項目番号。
        entry: usize,
    },
    /// 同じ6属性を持つ較正項目が複数ある。
    DuplicateCalibrationClass {
        /// 重複した後の項目番号。
        entry: usize,
    },
    /// 較正の出力先が既に存在する。
    CalibrationOutputExists {
        /// 出力先のパス。
        path: PathBuf,
    },
    /// 指定した教師の分類に訓練局面がない。
    EmptyTeacherClass {
        /// 教師の分類番号。
        class: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MctsTeacher { path } => write!(f, "{}: MCTS teacher data is not supported", path.display()),
            Self::PositionBuild(error) => write!(f, "cannot restore position: {error}"),
            Self::Position(error) => write!(f, "restored position is invalid: {error}"),
            Self::LionPromotionMismatch { recorded, restored } => write!(f, "lion-promotion flag mismatch: recorded {recorded}, restored {restored}"),
            Self::Io(error) => error.fmt(f),
            Self::Mnsd(error) => error.fmt(f),
            Self::MixedOrigins { path } => write!(f, "{}: mixed record origins", path.display()),
            Self::SyntheticOrigin { path } => write!(f, "{}: synthetic records cannot be used as teacher data", path.display()),
            Self::Dataset { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::RecordIndex { index, count } => {
                write!(f, "record index {index} is outside 0..{count}")
            }
            Self::LookaheadParameters => {
                f.write_str("lookahead requires finite gamma in (0, 1) and plies in 1..=4096")
            }
            Self::LookaheadSamples => {
                f.write_str("lookahead requires matching arrays and finite scores")
            }
            Self::TeacherLambda => f.write_str("teacher lambda must be finite and in 0..=1"),
            Self::TeacherTargets => {
                f.write_str("teacher targets require matching classes, finite positive active teacher cp_per_logit values and finite active scores")
            }
            Self::RecordOrder { index } => write!(
                f,
                "record {index}: games must be nondecreasing and plies strictly increasing within a game"
            ),
            Self::TeacherSamples => f.write_str(
                "teacher cp_per_logit estimation requires nonempty matching finite scores and results in 0..=2",
            ),
            Self::UndeterminedScale => f.write_str("teacher cp_per_logit is undetermined: all training scores are zero"),
            Self::CalibrationJson(error) => write!(f, "invalid teacher calibration JSON: {error}"),
            Self::CalibrationFormat => f.write_str("invalid teacher calibration format"),
            Self::CalibrationScale { entry } => write!(f, "calibration entry {entry}: teacher_cp_per_logit must be finite and positive"),
            Self::MissingCalibrationClass { class } => write!(f, "teacher class {class} has no calibration entry"),
            Self::UnexpectedCalibrationClass { entry } => write!(f, "calibration entry {entry} has no matching teacher class"),
            Self::DuplicateCalibrationClass { entry } => write!(f, "calibration entry {entry} duplicates a teacher class"),
            Self::CalibrationOutputExists { path } => write!(f, "calibration output already exists: {}", path.display()),
            Self::EmptyTeacherClass { class } => {
                write!(f, "teacher class {class} has no training records")
            }

        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PositionBuild(error) => Some(error),
            Self::Position(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Mnsd(error) => Some(error),
            Self::CalibrationJson(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<records::Error> for Error {
    fn from(error: records::Error) -> Self {
        Self::Mnsd(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::CalibrationJson(error)
    }
}

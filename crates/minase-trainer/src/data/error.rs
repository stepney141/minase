use std::{fmt, io, path::PathBuf};

use minase::training::{provenance, records};

/// データ経路の失敗。下位の復号・来歴・入出力エラーは原因を保持する。
#[derive(Debug)]
pub enum Error {
    /// ファイル操作またはワーカーの起動に失敗した。
    Io(io::Error),
    /// MNSDの復号または局面復元に失敗した。
    Mnsd(records::Error),
    /// 来歴JSONの検証に失敗した。
    Provenance(provenance::Error),
    /// 一時的な照合用再生ファイルが契約に違反した。
    Replay {
        /// 問題の再生ファイル。
        path: PathBuf,
        /// 違反した契約。
        reason: &'static str,
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
    /// 教師値の構築に必要な分類番号、Kまたは探索値が不正。
    TeacherTargets,
    /// 対局番号または同じ対局内の手数が順序規則に違反した。
    RecordOrder {
        /// 最初に違反したファイル内の記録番号。
        index: usize,
    },
    /// K推定の標本が空、長さが不一致、または値が定義域外にある。
    TeacherSamples,
    /// 指定した教師の分類に訓練局面がない。
    EmptyTeacherClass {
        /// 教師の分類番号。
        class: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Mnsd(error) => error.fmt(f),
            Self::Provenance(error) => error.fmt(f),
            Self::Dataset { path, reason } | Self::Replay { path, reason } => write!(f, "{}: {reason}", path.display()),
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
                f.write_str("teacher targets require matching classes, finite positive active K values and finite active scores")
            }
            Self::RecordOrder { index } => write!(
                f,
                "record {index}: games must be nondecreasing and plies strictly increasing within a game"
            ),
            Self::TeacherSamples => f.write_str(
                "K estimation requires nonempty matching finite scores and results in 0..=2",
            ),
            Self::EmptyTeacherClass { class } => {
                write!(f, "teacher class {class} has no training records")
            }

        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Mnsd(error) => Some(error),
            Self::Provenance(error) => Some(error),
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

impl From<provenance::Error> for Error {
    fn from(error: provenance::Error) -> Self {
        Self::Provenance(error)
    }
}

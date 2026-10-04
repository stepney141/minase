//! lishogi棋譜の読み込みと変換のエラー。

use std::io;

#[derive(Debug)]
pub(super) enum ImportError {
    ReadLine {
        line: usize,
        source: io::Error,
    },
    JsonLine {
        line: usize,
        source: serde_json::Error,
    },
    InvalidId {
        line: usize,
    },
    DirtyTree,
    WorkerPanic,
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadLine { line, source } => write!(f, "NDJSON line {line}: {source}"),
            Self::JsonLine { line, source } => write!(f, "NDJSON line {line}: {source}"),
            Self::InvalidId { line } => write!(
                f,
                "NDJSON line {line}: id must be nonempty and contain no whitespace"
            ),
            Self::DirtyTree => {
                f.write_str("the working tree is dirty; commit changes or pass --allow-dirty")
            }
            Self::WorkerPanic => f.write_str("import worker panicked"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::JsonLine { source, .. } => Some(source),
            Self::ReadLine { source, .. } => Some(source),
            _ => None,
        }
    }
}

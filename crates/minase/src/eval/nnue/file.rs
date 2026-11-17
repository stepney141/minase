//! 外部のNNUEファイルの読み込みと識別。

use super::{Error, Nnue};
use sha2::{Digest, Sha256};
use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// 同じバイト列から復号したネットと、ファイル全体のSHA-256。
pub struct NetworkFile {
    /// 検証済みのネット。
    pub network: Arc<Nnue>,
    /// ヘッダを含むファイル全体のSHA-256。
    pub sha256: [u8; 32],
}

/// 外部のNNUEファイルを読み込めない原因。
#[derive(Debug)]
pub enum NetworkFileError {
    /// ファイルの読み取りに失敗した。
    Read {
        /// 指定されたパス。
        path: PathBuf,
        /// 読み取りの原因。
        source: io::Error,
    },
    /// ファイルの復号に失敗した。
    Decode {
        /// 指定されたパス。
        path: PathBuf,
        /// 復号の原因。
        source: Error,
    },
}

impl fmt::Display for NetworkFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => write!(
                f,
                "failed to read NNUE network file '{}': {source}",
                path.display()
            ),
            Self::Decode { path, source } => write!(
                f,
                "invalid NNUE network file '{}': {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for NetworkFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            Self::Decode { source, .. } => Some(source),
        }
    }
}

/// ファイルを1回読み、同じバイト列からネットと識別子を得る。
/// 識別子はヘッダを含むファイル全体のSHA-256である。
/// MNUEのヘッダのチェックサムは本体の破損の検査に残し、識別子には使わない。
///
/// # Errors
/// 読み取りと復号の失敗を区別し、パスと元の原因を保持する。
pub fn read_network_file(path: &Path) -> Result<NetworkFile, NetworkFileError> {
    let bytes = std::fs::read(path).map_err(|source| NetworkFileError::Read {
        path: path.to_owned(),
        source,
    })?;
    let sha256 = Sha256::digest(&bytes).into();
    let network = Nnue::decode(&bytes).map_err(|source| NetworkFileError::Decode {
        path: path.to_owned(),
        source,
    })?;
    Ok(NetworkFile {
        network: Arc::new(network),
        sha256,
    })
}

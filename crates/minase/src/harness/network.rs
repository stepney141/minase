//! エンジンが報告するネットのSHA-256の検査と測定条件の不一致。

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// 小文字の16進数64桁として検査済みのネットのSHA-256。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct NetworkSha256(String);

impl TryFrom<String> for NetworkSha256 {
    type Error = InvalidNetworkSha256;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(InvalidNetworkSha256 { value })
        }
    }
}

impl From<NetworkSha256> for String {
    fn from(value: NetworkSha256) -> Self {
        value.0
    }
}

impl fmt::Display for NetworkSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// 小文字の16進数64桁ではないチェックサム。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidNetworkSha256 {
    value: String,
}

impl fmt::Display for InvalidNetworkSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "received {:?}; network_sha256 must contain exactly 64 lowercase hexadecimal digits",
            self.value
        )
    }
}

impl std::error::Error for InvalidNetworkSha256 {}

/// `null`を許しつつ、欄の欠落を拒否するネットのチェックサムの読み取り。
pub fn deserialize_network_sha256<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<NetworkSha256>, D::Error> {
    Option::deserialize(deserializer)
}

/// 反則負けとして扱えない、ネットの報告または測定条件の誤り。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkError {
    /// エンジンが形式の不正なチェックサムを報告した。
    Invalid {
        /// 記録された値。事前の起動では`None`。
        recorded: Option<NetworkSha256>,
        /// 不正な値とその原因。
        source: InvalidNetworkSha256,
    },
    /// 1回の準備で複数のチェックサム行が届いた。
    Duplicate {
        /// 記録された値。事前の起動では`None`。
        recorded: Option<NetworkSha256>,
        /// 最初に届いた値。
        first: NetworkSha256,
        /// 2行目に届いた値。
        received: String,
    },
    /// 準備で報告された値が記録された値と一致しない。
    Mismatch {
        /// 記録された値。
        recorded: Option<NetworkSha256>,
        /// エンジンが報告した値。
        received: Option<NetworkSha256>,
    },
}

fn checksum_text(value: &Option<NetworkSha256>) -> &str {
    match value {
        Some(value) => &value.0,
        None => "null",
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid { recorded, source } => write!(
                f,
                "invalid network_sha256: recorded {}, {source}",
                checksum_text(recorded)
            ),
            Self::Duplicate {
                recorded,
                first,
                received,
            } => write!(
                f,
                "duplicate network_sha256: recorded {}, received {first} and {received:?}",
                checksum_text(recorded)
            ),
            Self::Mismatch { recorded, received } => write!(
                f,
                "network_sha256 mismatch: recorded {}, received {}",
                checksum_text(recorded),
                checksum_text(received)
            ),
        }
    }
}

impl std::error::Error for NetworkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid { source, .. } => Some(source),
            Self::Duplicate { .. } | Self::Mismatch { .. } => None,
        }
    }
}

/// 測定の候補側と基準側。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineSide {
    /// 候補側。
    Candidate,
    /// 基準側。
    Baseline,
}

impl fmt::Display for EngineSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Candidate => "candidate",
            Self::Baseline => "baseline",
        })
    }
}

/// 測定を中止するネットの誤りと、それを報告した側。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchEngineError {
    /// 候補側または基準側。
    pub side: EngineSide,
    /// 元のネットの誤り。
    pub source: NetworkError,
}

impl fmt::Display for MatchEngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} engine: {}", self.side, self.source)
    }
}

impl std::error::Error for MatchEngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

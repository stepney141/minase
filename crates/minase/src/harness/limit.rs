//! 思考制限と時間制御の構文。

use crate::harness::clock::Clock;
use crate::search::MAX_PLY;

/// USIの`go`へ渡す思考制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SearchLimit {
    /// 深さまたはノード数による固定制限。
    Fixed {
        /// 探索深さの上限。
        depth: Option<u32>,
        /// 探索ノード数の上限。
        nodes: Option<u64>,
    },
    /// 持ち時間による時間制御。
    Time(TimeControl),
}

/// ミリ秒単位の持ち時間、加算時間、秒読み。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimeControl {
    /// 持ち時間(ms)。
    pub base_ms: u64,
    /// 1手ごとの加算時間(ms)。
    pub increment_ms: u64,
    /// 1手ごとの秒読み(ms)。
    pub byoyomi_ms: u64,
}

impl SearchLimit {
    /// CLI表示用の制限文字列を返す。
    pub fn cli_text(self) -> String {
        match self {
            Self::Fixed {
                depth: Some(depth),
                nodes: Some(nodes),
            } => format!("depth={depth},nodes={nodes}"),
            Self::Fixed {
                depth: Some(depth),
                nodes: None,
            } => format!("depth={depth}"),
            Self::Fixed {
                depth: None,
                nodes: Some(nodes),
            } => format!("nodes={nodes}"),
            Self::Fixed {
                depth: None,
                nodes: None,
            } => unreachable!("a validated fixed limit contains depth or nodes"),
            Self::Time(time) if time.byoyomi_ms == 0 => {
                format!("time={}+{}", time.base_ms, time.increment_ms)
            }
            Self::Time(time) => format!(
                "time={}+{},byoyomi={}",
                time.base_ms, time.increment_ms, time.byoyomi_ms
            ),
        }
    }

    /// 固定制限のUSI `go`引数を返す。
    pub(super) fn fixed_go_text(self) -> Option<String> {
        match self {
            Self::Fixed {
                depth: Some(depth),
                nodes: Some(nodes),
            } => Some(format!("depth {depth} nodes {nodes}")),
            Self::Fixed {
                depth: Some(depth),
                nodes: None,
            } => Some(format!("depth {depth}")),
            Self::Fixed {
                depth: None,
                nodes: Some(nodes),
            } => Some(format!("nodes {nodes}")),
            Self::Fixed {
                depth: None,
                nodes: None,
            } => unreachable!("a validated fixed limit contains depth or nodes"),
            Self::Time(_) => None,
        }
    }

    /// 時間制御なら時計の初期状態を返す。
    pub(super) fn clock(self) -> Option<Clock> {
        match self {
            Self::Fixed { .. } => None,
            Self::Time(time) => Some(Clock::new(time)),
        }
    }
}

/// 思考制限の構文エラー、または深さの範囲違反。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchLimitError {
    /// 深さは1〜MAX_PLYでなければならない。
    DepthOutOfRange { depth: u32 },
    /// 整数または制限の構文が不正。
    InvalidSyntax(String),
}

impl From<String> for SearchLimitError {
    fn from(message: String) -> Self {
        Self::InvalidSyntax(message)
    }
}

impl std::fmt::Display for SearchLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DepthOutOfRange { depth } => {
                write!(f, "search depth {depth} must be between 1 and {MAX_PLY}")
            }
            Self::InvalidSyntax(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SearchLimitError {}

/// `--each`などへ渡す思考制限を解析する。
///
/// 文法は`depth=N[,nodes=M]`、`nodes=M`、`time=B+I[,byoyomi=ms]`である。
/// 深さは1〜[`MAX_PLY`]、ノード数は1以上、時間の各値は0以上の整数ミリ秒とする。
/// `B`は基本時間、`I`は加算時間であり、省略した秒読みは0となる。
pub fn parse_search_limit(input: &str) -> Result<SearchLimit, SearchLimitError> {
    let mut fields = input.split(',');
    let first = fields.next().expect("split always returns one field");
    if let Some(value) = first.strip_prefix("time=") {
        let (base_ms, increment_ms) = value
            .split_once('+')
            .ok_or_else(|| "time limit must be 'time=<base_ms>+<inc_ms>'".to_owned())?;
        if increment_ms.contains('+') {
            return Err("time limit must contain exactly one '+'".to_owned().into());
        }
        let base_ms = parse_nonnegative_u64(base_ms)?;
        let increment_ms = parse_nonnegative_u64(increment_ms)?;
        let byoyomi_ms = fields
            .next()
            .map(|field| {
                field
                    .strip_prefix("byoyomi=")
                    .ok_or_else(|| "time limit may only be followed by 'byoyomi=<ms>'".to_owned())
                    .and_then(parse_nonnegative_u64)
            })
            .transpose()?
            .unwrap_or(0);
        if fields.next().is_some() {
            return Err("limit has too many comma-separated fields"
                .to_owned()
                .into());
        }
        return Ok(SearchLimit::Time(TimeControl {
            base_ms,
            increment_ms,
            byoyomi_ms,
        }));
    }

    let (depth, nodes) = if let Some(value) = first.strip_prefix("depth=") {
        let depth = parse_search_depth(value)?;
        let nodes = fields
            .next()
            .map(|field| {
                field
                    .strip_prefix("nodes=")
                    .ok_or_else(|| "the second limit field must be 'nodes=M'".to_owned())
                    .and_then(parse_positive_u64)
            })
            .transpose()?;
        (Some(depth), nodes)
    } else if let Some(value) = first.strip_prefix("nodes=") {
        (None, Some(parse_positive_u64(value)?))
    } else {
        return Err(
            "limit must be 'depth=N', 'nodes=M', 'depth=N,nodes=M', or 'time=B+I'"
                .to_owned()
                .into(),
        );
    };
    if fields.next().is_some() {
        return Err("limit has too many comma-separated fields"
            .to_owned()
            .into());
    }
    Ok(SearchLimit::Fixed { depth, nodes })
}

/// 0以上の`u64`を解析する。
fn parse_nonnegative_u64(text: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .map_err(|error| format!("invalid nonnegative integer '{text}': {error}"))
}

/// 0より大きい`u64`を解析する。
pub fn parse_positive_u64(text: &str) -> Result<u64, String> {
    let value = text
        .parse::<u64>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 0より大きい`u32`を解析する。
pub fn parse_positive_u32(text: &str) -> Result<u32, String> {
    let value = text
        .parse::<u32>()
        .map_err(|error| format!("invalid positive integer '{text}': {error}"))?;
    if value == 0 {
        return Err("value must be at least 1".to_owned());
    }
    Ok(value)
}

/// 探索が扱える範囲の深さを解析する。
fn parse_search_depth(text: &str) -> Result<u32, SearchLimitError> {
    let depth = text
        .parse::<u32>()
        .map_err(|error| format!("invalid search depth '{text}': {error}"))?;
    if depth == 0 || depth > MAX_PLY {
        return Err(SearchLimitError::DepthOutOfRange { depth });
    }
    Ok(depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    // [`parse_search_limit`]の思考制限の文法を検査する。
    // 規範文書に明文がないため、depthとnodesの併記を受理することを実装契約とする。
    #[test]
    fn search_limits_accept_the_documented_grammar() {
        assert_eq!(
            parse_search_limit("depth=4"),
            Ok(SearchLimit::Fixed {
                depth: Some(4),
                nodes: None
            })
        );
        assert_eq!(
            parse_search_limit("nodes=100000"),
            Ok(SearchLimit::Fixed {
                depth: None,
                nodes: Some(100_000)
            })
        );
        assert_eq!(
            parse_search_limit("time=10000+100"),
            Ok(SearchLimit::Time(TimeControl {
                base_ms: 10_000,
                increment_ms: 100,
                byoyomi_ms: 0,
            }))
        );
        assert_eq!(
            parse_search_limit("time=0+0,byoyomi=1000"),
            Ok(SearchLimit::Time(TimeControl {
                base_ms: 0,
                increment_ms: 0,
                byoyomi_ms: 1_000,
            }))
        );
        // [実装契約] 併記形
        assert_eq!(
            parse_search_limit("depth=3,nodes=400"),
            Ok(SearchLimit::Fixed {
                depth: Some(3),
                nodes: Some(400)
            })
        );
    }

    /// parse_search_limitのdocに定める深さ1〜256の両端を検証する。
    #[test]
    fn search_depth_accepts_bounds_and_rejects_adjacent_values() {
        assert_eq!(MAX_PLY, 256, "数値例の前提");
        for (text, expected) in [("depth=1", 1), ("depth=256", 256)] {
            assert_eq!(
                parse_search_limit(text),
                Ok(SearchLimit::Fixed {
                    depth: Some(expected),
                    nodes: None
                })
            );
        }
        for (text, depth) in [("depth=0", 0), ("depth=257", 257)] {
            assert_eq!(
                parse_search_limit(text),
                Err(SearchLimitError::DepthOutOfRange { depth })
            );
        }
    }

    // 規範文書に明文がないため、文法に合致しない制限を拒否することを実装契約とする。
    // エラー文言は契約ではない。
    #[test]
    fn search_limits_reject_malformed_inputs() {
        for invalid in [
            "",
            "byoyomi=1000",
            "time=1000",
            "time=1000+10+5",
            "time=1000+10,depth=1",
            "depth=1,time=1000+10",
            "nodes=1,depth=1",
            "depth=0",
            "nodes=0",
            "depth=2147483648",
            "depth=4294967296",
            "nodes=18446744073709551616",
            "depth=1,nodes=18446744073709551616",
            "time=18446744073709551616+0",
            "time=0+18446744073709551616",
            "time=0+0,byoyomi=18446744073709551616",
            "depth=1,nodes=2,x=3",
        ] {
            assert!(
                parse_search_limit(invalid).is_err(),
                "limit {invalid:?} must be rejected"
            );
        }
    }
}

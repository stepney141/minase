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

/// 思考制限を解析する。
pub fn parse_search_limit(input: &str) -> Result<SearchLimit, String> {
    let mut fields = input.split(',');
    let first = fields.next().expect("split always returns one field");
    if let Some(value) = first.strip_prefix("time=") {
        let (base_ms, increment_ms) = value
            .split_once('+')
            .ok_or_else(|| "time limit must be 'time=<base_ms>+<inc_ms>'".to_owned())?;
        if increment_ms.contains('+') {
            return Err("time limit must contain exactly one '+'".to_owned());
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
            return Err("limit has too many comma-separated fields".to_owned());
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
            "limit must be 'depth=N', 'nodes=M', 'depth=N,nodes=M', or 'time=B+I'".to_owned(),
        );
    };
    if fields.next().is_some() {
        return Err("limit has too many comma-separated fields".to_owned());
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
fn parse_search_depth(text: &str) -> Result<u32, String> {
    let depth = parse_positive_u32(text)?;
    if depth > MAX_PLY {
        return Err(format!("search depth must not exceed {MAX_PLY}"));
    }
    Ok(depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    // D8-HARN-09(sprt.md・search.md実施状況): 思考制限は
    // `depth=N|nodes=M|time=<base_ms>+<inc_ms>[,byoyomi=<ms>]`。
    // depthとnodesの併記受理はSPEC_UNCLEAR-08につき[実装契約]。
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

    // SPEC_UNCLEAR-08 [実装契約]: 文法に合致しない制限は拒否される。
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
            "depth=1,nodes=2,x=3",
        ] {
            assert!(
                parse_search_limit(invalid).is_err(),
                "limit {invalid:?} must be rejected"
            );
        }
    }
}

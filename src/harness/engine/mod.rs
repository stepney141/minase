//! 外部エンジンの駆動と、思考の要求および応答。

mod cecp;
mod channel;
mod ponder;
mod probe;
mod process;
mod resources;
mod think;
mod usi;

#[cfg(test)]
mod tests;

pub(in crate::harness) use cecp::{CECP_FIXED_TIME_CS, canonical_jitto, validate_cecp_limit};
pub(in crate::harness) use process::EngineProcess;
pub(in crate::harness) use resources::EngineResourceUsage;

pub use probe::{probe_engine_defaults, probe_usi_options};

use crate::harness::records::{ScoreBound, StopReasonRecord};
use std::time::Duration;

/// 1回の思考に必要なプロトコル別の制限値。
pub(in crate::harness) struct ThinkRequest {
    /// USIの`go`へ渡す引数。
    pub(in crate::harness) go_text: String,
    /// CECPの`time`へ渡す手番側の残り時間（センチ秒）。
    pub(in crate::harness) own_cs: u64,
    /// CECPの`otim`へ渡す相手側の残り時間（センチ秒）。
    pub(in crate::harness) opponent_cs: u64,
}

/// エンジンが返した対局上の応答。
#[derive(PartialEq, Eq, Debug)]
pub(in crate::harness) enum EngineResponse {
    /// エンジンが選んだ指し手表記。
    Move(String),
    /// エンジンが着手を返さず投了した。
    Resigned,
}

/// エンジンが最後に報告した評価値。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::harness) struct EngineEvaluation {
    /// 報告された探索深さ。省略された場合は`None`。
    pub(in crate::harness) depth: Option<u32>,
    /// センチポーンまたは詰み手数による評価値。
    pub(in crate::harness) score: EngineScore,
    /// 評価値が上下界ならその種別。
    pub(in crate::harness) bound: ScoreBound,
}

/// USI `info score`の評価値。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::harness) enum EngineScore {
    /// センチポーン単位の評価値。
    Cp(i32),
    /// 手番側が詰ませるまでの手数。手数不明なら`None`。
    MateIn(Option<u32>),
    /// 手番側が詰むまでの手数。手数不明なら`None`。
    MatedIn(Option<u32>),
}

/// 1回の思考で得た応答、所要時間、および探索情報。
pub(in crate::harness) struct ThinkResult {
    /// エンジンが返した着手または投了。
    pub(in crate::harness) response: EngineResponse,
    /// 着手とともに返された予想手。
    pub(in crate::harness) ponder: Option<String>,
    /// `go`、`ponderhit`または`stop`から応答までの実測時間。
    pub(in crate::harness) elapsed: Duration,
    /// 最後の有効な`info score`。報告がなければ`None`。
    pub(in crate::harness) evaluation: Option<EngineEvaluation>,
    /// エンジンが報告した停止理由。報告がなければ`None`。
    pub(in crate::harness) stop_reason: Option<StopReasonRecord>,
    /// 最後の`info`行が報告した経過時間(ms)。報告がなければ`None`。
    pub(in crate::harness) completed_time_ms: Option<u64>,
}

/// USI初期化応答が報告する既定の探索資源。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct EngineDefaults {
    /// `Threads`の既定値。
    pub threads: Option<u32>,
    /// `USI_Hash`の既定値(MB)。
    pub hash_mb: Option<u64>,
}

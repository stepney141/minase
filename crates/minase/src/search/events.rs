//! 探索の進捗通知と完了結果。

use std::time::Duration;

use minase_core::mv::Move;

/// 探索を停止した条件。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StopReason {
    /// 指定深さを完了した。
    DepthCompleted,
    /// 指定ノード数へ達した。
    NodeLimit,
    /// soft limitまたは完了反復の評価値に基づく時間管理の判断で停止した。
    SoftLimit,
    /// 探索中にhard limitへ達した。
    HardLimit,
    /// 呼び出し側から停止を要求された。
    ExternalStop,
}

impl StopReason {
    /// 探索を実際に終わらせた理由を、対局と診断で共通の固定語へ写す。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DepthCompleted => "depth",
            Self::NodeLimit => "nodes",
            Self::SoftLimit => "soft",
            Self::HardLimit => "hard",
            Self::ExternalStop => "external",
        }
    }
}

/// 順位つきの読み筋の1本。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PvLine {
    /// この読み筋の深さ。
    pub depth: u32,
    /// 根の手番側から見た評価値。PV-MCTSの未訪問の辺では持たない。
    pub score: Option<i32>,
    /// 根から指す手の列。先頭がこの読み筋の根の手である。
    pub moves: Vec<Move>,
}

/// 探索スレッドから届く進捗または完了通知。
///
/// `lines`は1要素以上を順位順に持ち、先頭が1本目（最善）である。
/// `MultiPV`が1なら1要素で、その読み筋、評価値、深さは単一の主変化と同じである。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchEvent {
    /// 主ワーカーが窓内で反復深化の1反復を完了したときだけ送る進捗通知。
    /// 補助ワーカーと、窓外れを読み直している途中からは送らない。
    Progress {
        /// 通知元の探索ID。
        search_id: u64,
        /// 通知時点のチーム全体の合計ノード数。数え方は[`SearchResult::nodes`]に従う。
        nodes: u64,
        /// 探索開始からの経過時間。
        elapsed: Duration,
        /// 完了した反復の読み筋を最善から順位順に並べた列。
        lines: Vec<PvLine>,
    },
    /// 全ワーカーが終了した後に1回だけ送る完了通知。
    /// 深さは最後の[`Self::Progress`]以上であり、補助ワーカーの採用により最善手と評価値は最後の進捗から変わり得る。
    /// 途中結果を採った場合も着手と主変化が変わり得るが、`score`と`depth`は完了反復のものを保つ。
    /// ワーカーがパニックした場合は送信しない。
    Finished {
        /// hard limitで中断した反復から採用した途中結果の下界。完了反復の値とは区別する。
        partial_score: Option<i32>,
        /// 通知元の探索ID。
        search_id: u64,
        /// 選んだ着手。
        best_move: Move,
        /// 最後まで完了した深さの評価値。
        score: i32,
        /// 最後まで完了した深さ。
        depth: u32,
        /// 通知時点のチーム全体の合計ノード数。数え方は[`SearchResult::nodes`]に従う。
        nodes: u64,
        /// 探索開始からの経過時間。
        elapsed: Duration,
        /// 採用した読み筋を最善から順位順に並べた列。
        /// 途中結果を採ったときは先頭の`moves`が途中結果の読み筋となり、
        /// 値の下界は`partial_score`で示す。深さと評価値は完了反復のものを保つ。
        lines: Vec<PvLine>,
        /// 探索を停止した条件。
        stop_reason: StopReason,
    },
}

impl SearchEvent {
    /// 通知元の探索IDを返す。
    /// 呼び出し側は現在の探索IDと照合し、停止済みの旧探索から遅れて届いた通知を破棄する。
    pub fn search_id(&self) -> u64 {
        match self {
            Self::Progress { search_id, .. } | Self::Finished { search_id, .. } => *search_id,
        }
    }
}

/// 完了した探索の結果。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchResult {
    /// 全ワーカーの探索終了時点の統計を合算した値。
    #[cfg(feature = "search-stats")]
    pub stats: super::SearchStats,
    /// 選んだ着手。
    pub best_move: Move,
    /// 最後まで完了した反復の評価値。途中結果を採用しても更新しない。
    pub score: i32,
    /// 最後まで完了した反復深化の深さ。
    pub depth: u32,
    /// 探索開始から全ワーカーが実着手を盤面へ適用した合計回数。
    /// 通常探索と静止探索の着手適用直前に数え、null move、根、最後の王駒を取る手は数えない。
    /// 反復または置換表で打ち切られる子局面も、着手を適用して到達するため1ノードと数える。
    pub nodes: u64,
}

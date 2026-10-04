//! エンジン異常の分類と件数。

/// USIセッションの異常分類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EngineFailure {
    /// `bestmove`が審判層の合法手にない。
    IllegalMove,
    /// プロセス終了またはパイプ切断。
    Crash,
    /// 応答期限までに応答がない。
    Timeout,
    /// 時間制御対局での時間切れ。
    TimeForfeit,
    /// 審判層が合法とした相手の着手を`Illegal move`で拒否した。
    RejectedMove,
}

/// 異常理由別の発生件数。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureCounts {
    /// 不正着手の件数。
    pub illegal_moves: u64,
    /// クラッシュの件数。
    pub crashes: u64,
    /// 応答タイムアウトの件数。
    pub timeouts: u64,
    /// 時間切れの件数。
    pub time_forfeits: u64,
    /// 審判層の合法手を拒否した件数。
    pub rejected_moves: u64,
}

impl FailureCounts {
    /// 1局の異常を加算する。
    pub fn record(&mut self, failure: EngineFailure) {
        match failure {
            EngineFailure::IllegalMove => self.illegal_moves += 1,
            EngineFailure::Crash => self.crashes += 1,
            EngineFailure::Timeout => self.timeouts += 1,
            EngineFailure::TimeForfeit => self.time_forfeits += 1,
            EngineFailure::RejectedMove => self.rejected_moves += 1,
        }
    }

    /// 別の集計値を加算する。
    pub fn add(&mut self, other: Self) {
        self.illegal_moves += other.illegal_moves;
        self.crashes += other.crashes;
        self.timeouts += other.timeouts;
        self.time_forfeits += other.time_forfeits;
        self.rejected_moves += other.rejected_moves;
    }
}

impl std::fmt::Display for EngineFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for EngineFailure {}

#[cfg(test)]
mod tests {
    use super::*;

    // D8-HARN-14（sprt.md「異常時の裁定」）: `engine_failures:`は不正着手・
    // クラッシュ・応答タイムアウト・時間切れ・着手拒否の理由別件数を報告する。
    // 理由別件数の合計は反則負けとして算入された局数と一致する(保存則)。
    #[test]
    fn failure_reasons_are_counted_separately_and_conserved() {
        let mut counts = FailureCounts::default();
        counts.record(EngineFailure::IllegalMove);
        counts.record(EngineFailure::Crash);
        counts.record(EngineFailure::Timeout);
        counts.record(EngineFailure::TimeForfeit);
        counts.record(EngineFailure::RejectedMove);
        assert_eq!(
            counts,
            FailureCounts {
                illegal_moves: 1,
                crashes: 1,
                timeouts: 1,
                time_forfeits: 1,
                rejected_moves: 1,
            }
        );
        // 集計の合成でも件数は保存される
        let mut total = FailureCounts::default();
        total.add(counts);
        total.add(counts);
        assert_eq!(
            total.illegal_moves
                + total.crashes
                + total.timeouts
                + total.time_forfeits
                + total.rejected_moves,
            10
        );
    }
}

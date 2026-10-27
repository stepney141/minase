//! 探索と先読み的中の経過時間を同じ起点から測る時計。

use std::time::{Duration, Instant};

/// 探索開始からの単調増加する経過時間を返す。
pub(crate) trait SearchClock: Send + Sync {
    fn elapsed(&self) -> Duration;
}

/// スレッド生成前の起点から実時間を測る。
pub(crate) struct MonotonicClock {
    started: Instant,
}

impl MonotonicClock {
    pub(crate) fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl SearchClock for MonotonicClock {
    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

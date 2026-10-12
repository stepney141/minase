//! スレッド局所の累算器計数を、スレッド終了時とbenchの回収時だけ合算する。

use std::cell::{Cell, RefCell};
use std::ops::AddAssign;
use std::sync::{Arc, Mutex};

use super::{Network, feature_set::FeatureSet, head::Head};

/// 両視点を1組とした累算器の更新回数。整合性検査の再計算は含めない。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AccumulatorStats {
    /// 文脈を保った差分更新の回数。null moveも含む。
    pub incremental_updates: u64,
    /// 根の初期化、または一方以上の文脈変更による再計算の回数。
    pub refreshes: u64,
}

impl AccumulatorStats {
    /// 根の初期化と文脈変更による再計算を含む、全更新回数。
    pub const fn updates(self) -> u64 {
        self.incremental_updates + self.refreshes
    }
}

impl AddAssign for AccumulatorStats {
    fn add_assign(&mut self, rhs: Self) {
        self.incremental_updates += rhs.incremental_updates;
        self.refreshes += rhs.refreshes;
    }
}

pub(super) type SharedStats = Arc<Mutex<AccumulatorStats>>;

thread_local! {
    // Dropを持つ送信先と分け、更新ごとは局所カウンタへの加算1回だけにする。
    static COUNTS: [Cell<u64>; 2] = const { [Cell::new(0), Cell::new(0)] };
    static DESTINATION: RefCell<Option<Destination>> = const { RefCell::new(None) };
}

struct Destination(SharedStats);

impl Destination {
    fn collect(&self) {
        let counts = COUNTS.with(|counts| AccumulatorStats {
            incremental_updates: counts[0].replace(0),
            refreshes: counts[1].replace(0),
        });
        *self
            .0
            .lock()
            .expect("accumulator statistics mutex poisoned") += counts;
    }
}

impl Drop for Destination {
    fn drop(&mut self) {
        self.collect();
    }
}

/// 根を作る時点で回収先を登録する。探索中は送信先へ触れない。
pub(super) fn attach(destination: &SharedStats) {
    DESTINATION.with_borrow_mut(|current| {
        if !current
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(&current.0, destination))
        {
            *current = Some(Destination(Arc::clone(destination)));
        }
    });
}

/// 差分更新と再計算を排他的に数え、全更新数は回収後に求める。
#[inline]
pub(super) fn record(refresh: bool) {
    COUNTS.with(|counts| {
        let count = &counts[usize::from(refresh)];
        count.set(count.get() + 1);
    });
}

impl<F: FeatureSet, H: Head> Network<F, H> {
    /// 終了済みの探索スレッドと呼び出し元スレッドの計数を回収し、0に戻す。
    ///
    /// 全探索ワーカーをjoinした後に呼ぶ。各スレッドは根を作ったネットへ計数を送る。
    ///
    /// # Panics
    ///
    /// 計数の回収中に別スレッドがパニックし、Mutexがpoisonedになった場合にパニックする。
    pub fn take_accumulator_stats(&self) -> AccumulatorStats {
        DESTINATION.with_borrow(|current| {
            if let Some(current) = current
                && Arc::ptr_eq(&current.0, &self.accumulator_stats)
            {
                current.collect();
            }
        });
        std::mem::take(
            &mut *self
                .accumulator_stats
                .lock()
                .expect("accumulator statistics mutex poisoned"),
        )
    }
}

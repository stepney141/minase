//! 探索の制限と持ち時間の入力。

use core::num::{NonZeroU32, NonZeroU64};

use crate::search::MAX_PLY;
use crate::search::error::SearchError;

/// 1回の有限探索に適用する制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FiniteSearchLimits {
    /// 反復深化で完了を目指す最大深さ。
    pub(super) depth: Option<NonZeroU32>,
    /// 探索中に実際の着手を盤面へ適用する回数の上限。
    pub(super) nodes: Option<NonZeroU64>,
    /// 1手に使う固定時間(ms)。
    pub(super) movetime_ms: Option<NonZeroU64>,
    /// 持ち時間、加算時間、秒読みによる制限。
    pub(super) clock: Option<ClockLimits>,
}

/// 1回の探索に適用する検証済み制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchLimits {
    kind: SearchLimitKind,
}

/// 有限探索と無期限探索を排他的に表す。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SearchLimitKind {
    /// 1個以上の停止条件を持つ有限探索。
    Finite(FiniteSearchLimits),
    /// 外部停止要求だけを停止条件とする探索。
    Infinite,
}

impl SearchLimits {
    /// 有限探索の制限を検証して構築する。
    ///
    /// # Errors
    ///
    /// 制約が1つもない場合、深さが範囲外の場合、ノード数が0の場合、または
    /// 固定時間が0 msの場合は[`SearchError`]を返す。
    pub fn new(
        depth: Option<u32>,
        nodes: Option<u64>,
        movetime_ms: Option<u64>,
        clock: Option<ClockLimits>,
    ) -> Result<Self, SearchError> {
        if depth.is_none() && nodes.is_none() && movetime_ms.is_none() && clock.is_none() {
            return Err(SearchError::MissingLimit);
        }
        let depth = depth
            .map(|depth| {
                NonZeroU32::new(depth)
                    .filter(|depth| depth.get() <= MAX_PLY)
                    .ok_or(SearchError::InvalidDepth { depth })
            })
            .transpose()?;
        let nodes = nodes
            .map(|nodes| NonZeroU64::new(nodes).ok_or(SearchError::ZeroNodeLimit))
            .transpose()?;
        let movetime_ms = movetime_ms
            .map(|milliseconds| NonZeroU64::new(milliseconds).ok_or(SearchError::ZeroMoveTime))
            .transpose()?;
        Ok(Self {
            kind: SearchLimitKind::Finite(FiniteSearchLimits {
                depth,
                nodes,
                movetime_ms,
                clock,
            }),
        })
    }

    /// 外部停止要求だけで停止する無期限探索の制限を返す。
    pub const fn infinite() -> Self {
        Self {
            kind: SearchLimitKind::Infinite,
        }
    }

    /// 外部停止要求だけで停止する無期限探索かを返す。
    pub const fn is_infinite(self) -> bool {
        matches!(self.kind, SearchLimitKind::Infinite)
    }

    /// 有限探索の深さ上限を返す。
    pub const fn depth(self) -> Option<u32> {
        match self.kind {
            SearchLimitKind::Finite(limits) => match limits.depth {
                Some(depth) => Some(depth.get()),
                None => None,
            },
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索で実際の着手を盤面へ適用する回数の上限を返す。
    pub const fn nodes(self) -> Option<u64> {
        match self.kind {
            SearchLimitKind::Finite(limits) => match limits.nodes {
                Some(nodes) => Some(nodes.get()),
                None => None,
            },
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索の固定時間(ms)を返す。
    pub const fn movetime_ms(self) -> Option<u64> {
        match self.kind {
            SearchLimitKind::Finite(limits) => match limits.movetime_ms {
                Some(milliseconds) => Some(milliseconds.get()),
                None => None,
            },
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索の持ち時間制限を返す。
    pub const fn clock(self) -> Option<ClockLimits> {
        match self.kind {
            SearchLimitKind::Finite(limits) => limits.clock,
            SearchLimitKind::Infinite => None,
        }
    }

    /// 有限探索の制限を返す。
    pub(super) fn finite(self) -> Option<FiniteSearchLimits> {
        match self.kind {
            SearchLimitKind::Finite(limits) => Some(limits),
            SearchLimitKind::Infinite => None,
        }
    }
}

/// 持ち時間から1手の予算を求めるための制限。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClockLimits {
    /// 手番開始時の残り時間(ms)。
    pub(super) remaining_ms: u64,
    /// 1手ごとの加算時間(ms)。
    pub(super) increment_ms: u64,
    /// 1手ごとの秒読み時間(ms)。
    pub(super) byoyomi_ms: u64,
    /// 開始局面から現局面までの手数。
    pub(super) ply: u32,
}

impl ClockLimits {
    /// 持ち時間制の制限を検証して構築する。
    ///
    /// # Errors
    ///
    /// 残り時間、加算時間、秒読み時間がすべて0 msの場合は
    /// [`SearchError::EmptyClock`]を返す。
    pub const fn new(
        remaining_ms: u64,
        increment_ms: u64,
        byoyomi_ms: u64,
        ply: u32,
    ) -> Result<Self, SearchError> {
        if remaining_ms == 0 && increment_ms == 0 && byoyomi_ms == 0 {
            return Err(SearchError::EmptyClock);
        }
        Ok(Self {
            remaining_ms,
            increment_ms,
            byoyomi_ms,
            ply,
        })
    }

    /// 手番開始時の残り時間(ms)を返す。
    pub const fn remaining_ms(self) -> u64 {
        self.remaining_ms
    }

    /// 1手ごとの加算時間(ms)を返す。
    pub const fn increment_ms(self) -> u64 {
        self.increment_ms
    }

    /// 1手ごとの秒読み時間(ms)を返す。
    pub const fn byoyomi_ms(self) -> u64 {
        self.byoyomi_ms
    }

    /// 開始局面から現局面までの手数を返す。
    pub const fn ply(self) -> u32 {
        self.ply
    }
}

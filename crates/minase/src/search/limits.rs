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
///
/// 複数の制限を持つ有限探索は、最も早く満たされた条件で止まる。
/// `movetime`と時計の併用時はsoftとhardのそれぞれで両者の小さい予算を採る。
/// 秒読みが0で`movetime`を併用しない時計には、相手との残り時間の差で目安時間を補正するため、
/// 相手の残り時間が必要である。秒読みが正の時計と`movetime`を併用する時計ではこの補正を使わない。
/// 無期限探索にはsoftもhardもなく、外部停止要求だけを停止条件とする。
/// [`crate::search::start_search`]で先読みとして開始すると、的中の通知まで時間の制限を使わない。
/// 深さの範囲は[`Self::new`]とプロトコル側の入力検査の両方で拒否する。
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
    /// 制約が1つもない場合、深さが`1..=MAX_PLY`（1〜256）から外れる場合、ノード数が0の場合、
    /// 固定時間が0 msの場合、または秒読みが0で固定時間を併用しない時計に相手の残り時間がない場合は
    /// [`SearchError`]を返す。
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
        if clock.is_some_and(|clock| {
            clock.byoyomi_ms == 0 && movetime_ms.is_none() && clock.opponent_remaining_ms.is_none()
        }) {
            return Err(SearchError::MissingOpponentClock);
        }
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
    /// 相手側の残り時間(ms)。
    pub(super) opponent_remaining_ms: Option<u64>,
    /// 1手ごとの加算時間(ms)。
    pub(super) increment_ms: u64,
    /// 1手ごとの秒読み時間(ms)。
    pub(super) byoyomi_ms: u64,
    /// 秒読みつき時計の締切に残す余裕(ms)。
    pub(super) byoyomi_margin_ms: u64,
    /// [`ClockLimits::ply`]の契約に従う、初形を0とした絶対手数。
    pub(super) ply: u32,
}

impl ClockLimits {
    /// 秒読みつき時計の安全上限から差し引く余裕の既定値。単位はmsである。
    pub const DEFAULT_BYOYOMI_MARGIN_MS: u64 = 30;

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
        opponent_remaining_ms: Option<u64>,
        ply: u32,
    ) -> Result<Self, SearchError> {
        if remaining_ms == 0 && increment_ms == 0 && byoyomi_ms == 0 {
            return Err(SearchError::EmptyClock);
        }
        Ok(Self {
            remaining_ms,
            opponent_remaining_ms,
            increment_ms,
            byoyomi_ms,
            byoyomi_margin_ms: Self::DEFAULT_BYOYOMI_MARGIN_MS,
            ply,
        })
    }

    /// 秒読みが正で、固定時間、深さ、ノード数を併用しない時計の締切に残す余裕をmsで設定する。
    /// 残り時間と秒読みの合計から差し引き、余裕が合計以上でも締切は最低1 msとする。
    pub const fn with_byoyomi_margin_ms(mut self, milliseconds: u64) -> Self {
        self.byoyomi_margin_ms = milliseconds;
        self
    }

    /// [`Self::with_byoyomi_margin_ms`]で設定する締切の余裕をmsで返す。
    pub const fn byoyomi_margin_ms(self) -> u64 {
        self.byoyomi_margin_ms
    }

    /// 手番開始時の残り時間(ms)を返す。
    pub const fn remaining_ms(self) -> u64 {
        self.remaining_ms
    }

    /// 相手側の残り時間(ms)を返す。
    pub const fn opponent_remaining_ms(self) -> Option<u64> {
        self.opponent_remaining_ms
    }

    /// 1手ごとの加算時間(ms)を返す。
    pub const fn increment_ms(self) -> u64 {
        self.increment_ms
    }

    /// 1手ごとの秒読み時間(ms)を返す。
    pub const fn byoyomi_ms(self) -> u64 {
        self.byoyomi_ms
    }

    /// 初形を0とした現局面の絶対手数を返す。
    ///
    /// プロトコルが[`crate::protocol::Engine::ply`]から渡す値であり、
    /// SFENの次の手番号から1を引いた基底手数と適用済みの手数を足したものである。
    /// 探索履歴の長さから推測せず、残り手数の見積りと序盤係数の入力に使う。
    /// 理由は`docs/adr/estimate-moves-to-go-from-ply.md`と
    /// `docs/adr/scale-only-the-byoyomi-term-in-the-opening.md`にある。
    pub const fn ply(self) -> u32 {
        self.ply
    }
}

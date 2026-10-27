//! αβ探索が対局内で持ち越す状態と、非同期探索の開始。

use core::num::NonZeroUsize;
use std::sync::Arc;

use crate::search::clock::MonotonicClock;
use crate::search::handle::start_search_with_clock;
use crate::search::{
    DEFAULT_THREADS, DEFAULT_TT_SIZE_MB, HistoryTables, SearchHandle, SearchLimits, SearchOption,
    SearchOptionKind, SearchOptionValue, SearchSnapshot, Searcher, SetOptionError, StartError,
    StartFailure, TranspositionTable,
};

/// αβ探索の置換表、history表、およびワーカー数。
///
/// 表は対局内で持ち越し、新規対局や規則の変更で消去する。
/// 理由は`docs/adr/carry-histories-and-correction-histories-within-a-game.md`にある。
/// NNUEの重みは探索の開始時に取得する。
pub struct Engine {
    /// 準備または最初の探索まで未確保の置換表。
    pub(in crate::search) transposition_table: Option<TranspositionTable>,
    /// 次の探索へ持ち越す履歴表。
    pub(in crate::search) histories: HistoryTables,
    /// 探索ワーカー数。履歴表のワーカー数と一致する。
    pub(in crate::search) threads: NonZeroUsize,
}

impl Engine {
    /// 置換表を確保せず、既定のワーカー数で履歴表を作る。
    pub(in crate::search) fn new() -> Self {
        Self {
            transposition_table: None,
            histories: HistoryTables::new(DEFAULT_THREADS),
            threads: DEFAULT_THREADS,
        }
    }

    /// αβ探索が受理するオプションを宣言順に返す。
    pub(in crate::search) fn options(&self) -> Vec<SearchOption> {
        vec![
            SearchOption {
                name: "USI_Hash",
                kind: SearchOptionKind::Spin {
                    default: DEFAULT_TT_SIZE_MB,
                    min: 1,
                    max: 65_536,
                },
            },
            SearchOption {
                name: "Threads",
                kind: SearchOptionKind::Spin {
                    default: DEFAULT_THREADS.get(),
                    min: 1,
                    max: 256,
                },
            },
        ]
    }

    /// 値を宣言範囲で検査し、確保が成功した場合だけ状態を変更する。
    pub(in crate::search) fn set_option(
        &mut self,
        name: &str,
        value: SearchOptionValue,
    ) -> Result<(), SetOptionError> {
        let option = self
            .options()
            .into_iter()
            .find(|option| option.name == name)
            .ok_or_else(|| SetOptionError::UnknownOption {
                name: name.to_owned(),
            })?;
        let SearchOptionKind::Spin { min, max, .. } = option.kind;
        let SearchOptionValue::Spin(value) = value;
        if !(min..=max).contains(&value) {
            return Err(SetOptionError::OutOfRange {
                name: option.name,
                value,
                min,
                max,
            });
        }
        match option.name {
            "USI_Hash" => {
                match &mut self.transposition_table {
                    Some(table) => table.resize(value),
                    None => TranspositionTable::new(value).map(|table| {
                        self.transposition_table = Some(table);
                    }),
                }
                .map_err(SetOptionError::TranspositionTable)?;
                self.histories.clear();
            }
            "Threads" => {
                let threads = NonZeroUsize::new(value).expect("validated non-zero worker count");
                if self.threads != threads {
                    self.histories = HistoryTables::new(threads);
                    self.threads = threads;
                }
            }
            _ => unreachable!("all declared alpha-beta options are handled"),
        }
        Ok(())
    }

    /// 置換表と履歴表を空にする。未確保の置換表は確保しない。
    pub(in crate::search) fn clear(&mut self) {
        self.histories.clear();
        if let Some(table) = &mut self.transposition_table {
            table.clear();
        }
    }

    /// USIの`isready`に先立ち、未確保の置換表の確保を試みる。
    /// 最初の`go`での確保が時計上の思考時間に含まれるのを避ける。
    /// 失敗は`go`で再試行した際に報告する。
    pub(in crate::search) fn prepare(&mut self) {
        if self.transposition_table.is_none()
            && let Ok(table) = TranspositionTable::new(DEFAULT_TT_SIZE_MB)
        {
            self.transposition_table = Some(table);
        }
    }

    /// 重みと置換表を準備し、状態を探索スレッドへ移す。
    #[allow(clippy::result_large_err)] // 失敗時も呼び出し側が探索状態を回収する。
    pub(in crate::search) fn start(
        mut self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
    ) -> Result<SearchHandle, StartFailure> {
        let nnue = match crate::eval::weights() {
            Ok(nnue) => nnue,
            Err(error) => {
                return Err(StartFailure {
                    searcher: Searcher::AlphaBeta(self),
                    error: StartError::Weights(error),
                });
            }
        };
        if self.transposition_table.is_none() {
            match TranspositionTable::new(DEFAULT_TT_SIZE_MB) {
                Ok(table) => self.transposition_table = Some(table),
                Err(error) => {
                    return Err(StartFailure {
                        searcher: Searcher::AlphaBeta(self),
                        error: StartError::TranspositionTable(error),
                    });
                }
            }
        }
        Ok(start_search_with_clock(
            self,
            nnue,
            snapshot,
            limits,
            search_id,
            ponder,
            Arc::new(MonotonicClock::new()),
        ))
    }
}

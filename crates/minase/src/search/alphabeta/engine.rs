//! 探索の間で持ち越すαβ探索の状態。

use core::num::NonZeroUsize;
use std::sync::Arc;

use crate::eval::Pst;
use crate::search::alphabeta::history::HistoryTables;
use crate::search::alphabeta::tt::{DEFAULT_SIZE_MB, TranspositionTable, TranspositionTableError};
use crate::search::handle::{SearchHandle, start_search};
use crate::search::limits::SearchLimits;
use crate::search::searcher::{OptionKind, OptionSpec};
use crate::search::snapshot::SearchSnapshot;

/// オプション`USI_Hash`で設定できる置換表容量の上限(MiB)。
pub const MAX_HASH_SIZE_MB: usize = 65_536;

/// オプション`Threads`で設定できるワーカー数の上限。
pub const MAX_THREADS: usize = 256;

/// αβ探索が宣言するオプション。宣言の順序はUSIの`option`行の順序になる。
const OPTIONS: [OptionSpec; 2] = [
    OptionSpec {
        name: "USI_Hash",
        kind: OptionKind::Spin {
            default: DEFAULT_SIZE_MB as i64,
            min: 1,
            max: MAX_HASH_SIZE_MB as i64,
        },
    },
    OptionSpec {
        name: "Threads",
        kind: OptionKind::Spin {
            default: crate::search::DEFAULT_THREADS.get() as i64,
            min: 1,
            max: MAX_THREADS as i64,
        },
    },
];

/// αβ探索が探索の間で持ち越す評価重み、置換表、history表、およびワーカー数。
///
/// 置換表は最初の[`prepare`](Self::prepare)、容量の設定、または探索の開始まで
/// 確保しない。起動直後に既定容量を確保すると、容量の設定を先に受けた場合にも
/// 既定容量分のメモリを書き込むためである。
pub struct Engine {
    /// 静的評価に使う学習PST。
    pst: Arc<Pst>,
    /// 次の探索に使うワーカー数。
    threads: NonZeroUsize,
    /// 探索の間で引き継ぐ置換表。未確保の間は`None`。
    tt: Option<TranspositionTable>,
    /// ワーカー別に対局内で持ち越すhistory表。ワーカー数は`threads`と同じ。
    histories: HistoryTables,
}

impl Engine {
    /// 評価重みとワーカー数から、置換表を確保していない状態を作る。
    pub fn new(pst: Arc<Pst>, threads: NonZeroUsize) -> Self {
        Self {
            pst,
            threads,
            tt: None,
            histories: HistoryTables::new(threads),
        }
    }

    /// 探索スレッドから返された部品で状態を組み立て直す。
    pub(in crate::search) fn from_parts(
        pst: Arc<Pst>,
        threads: NonZeroUsize,
        tt: TranspositionTable,
        histories: HistoryTables,
    ) -> Self {
        Self {
            pst,
            threads,
            tt: Some(tt),
            histories,
        }
    }

    /// 宣言するオプションを返す。
    pub fn options(&self) -> &'static [OptionSpec] {
        &OPTIONS
    }

    /// 名前が`USI_Hash`または`Threads`（大文字小文字を区別しない）なら値を設定する。
    ///
    /// 未知の名前には`None`を返す。値が不正な場合や置換表を確保できない場合は、
    /// 状態を変えずに理由を`Some(Err)`で返す。
    pub fn set_option(&mut self, name: &str, value: Option<&str>) -> Option<Result<(), String>> {
        if name.eq_ignore_ascii_case("USI_Hash") {
            let Some(value) = value else {
                return Some(Err("USI_Hash requires a value".to_owned()));
            };
            let Some(size_mb) = value
                .parse::<usize>()
                .ok()
                .filter(|&size| (1..=MAX_HASH_SIZE_MB).contains(&size))
            else {
                return Some(Err(format!(
                    "USI_Hash must be an integer from 1 to {MAX_HASH_SIZE_MB}"
                )));
            };
            Some(self.set_hash_mb(size_mb).map_err(|error| error.to_string()))
        } else if name.eq_ignore_ascii_case("Threads") {
            let Some(value) = value else {
                return Some(Err("Threads requires a value".to_owned()));
            };
            let Some(threads) = value
                .parse::<usize>()
                .ok()
                .and_then(NonZeroUsize::new)
                .filter(|threads| threads.get() <= MAX_THREADS)
            else {
                return Some(Err(format!(
                    "Threads must be an integer from 1 to {MAX_THREADS}"
                )));
            };
            self.set_threads(threads);
            Some(Ok(()))
        } else {
            None
        }
    }

    /// 置換表を指定容量(MB)で作り直し、history表を0にする。
    ///
    /// # Errors
    ///
    /// 置換表を作れない場合はエラーを返し、置換表もhistory表も変更しない。
    pub fn set_hash_mb(&mut self, size_mb: usize) -> Result<(), TranspositionTableError> {
        match &mut self.tt {
            Some(tt) => tt.resize(size_mb)?,
            None => self.tt = Some(TranspositionTable::new(size_mb)?),
        }
        self.histories.clear();
        Ok(())
    }

    /// 次の探索に使うワーカー数を返す。
    pub fn threads(&self) -> NonZeroUsize {
        self.threads
    }

    /// ワーカー数を変える。変わる場合だけhistory表をそのワーカー数で作り直す。
    pub fn set_threads(&mut self, threads: NonZeroUsize) {
        if self.threads != threads {
            self.histories = HistoryTables::new(threads);
            self.threads = threads;
        }
    }

    /// 新しい対局に備えて、置換表とhistory表を空にする。
    pub fn new_game(&mut self) {
        self.histories.clear();
        if let Some(tt) = &mut self.tt {
            tt.clear();
        }
    }

    /// 置換表が未確保なら既定容量で確保する。
    ///
    /// # Errors
    ///
    /// 置換表を確保できない場合はエラーを返す。
    pub fn prepare(&mut self) -> Result<(), TranspositionTableError> {
        if self.tt.is_none() {
            self.tt = Some(TranspositionTable::new(DEFAULT_SIZE_MB)?);
        }
        Ok(())
    }

    /// 状態を探索スレッドへ移して探索を開始する。置換表が未確保なら既定容量で確保する。
    ///
    /// # Errors
    ///
    /// 置換表を確保できない場合は、探索を開始せずに状態とエラーを返す。
    pub fn start(
        mut self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
    ) -> Result<SearchHandle, (Self, TranspositionTableError)> {
        let tt = match self.tt.take() {
            Some(tt) => tt,
            None => match TranspositionTable::new(DEFAULT_SIZE_MB) {
                Ok(tt) => tt,
                Err(error) => return Err((self, error)),
            },
        };
        Ok(start_search(
            self.pst,
            snapshot,
            limits,
            search_id,
            self.threads,
            tt,
            self.histories,
            ponder,
        ))
    }

    /// 静的評価に使う学習PSTを返す。
    pub fn pst(&self) -> &Pst {
        &self.pst
    }

    /// 確保済みの置換表を返す。
    pub fn transposition_table(&self) -> Option<&TranspositionTable> {
        self.tt.as_ref()
    }

    /// 試験用に、探索後の置換表とhistory表を取り出す。
    #[cfg(test)]
    pub(crate) fn into_parts(self) -> (TranspositionTable, HistoryTables) {
        (
            self.tt
                .expect("a searched engine owns its transposition table"),
            self.histories,
        )
    }

    /// 試験用に、探索の間で持ち越すhistory表を返す。
    #[cfg(test)]
    pub(crate) fn histories(&self) -> &HistoryTables {
        &self.histories
    }

    /// 試験用に、探索の間で持ち越すhistory表を可変で返す。
    #[cfg(test)]
    pub(crate) fn histories_mut(&mut self) -> &mut HistoryTables {
        &mut self.histories
    }
}

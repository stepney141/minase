//! 選ばれた探索方式の状態を包み、プロトコル層へ方式に依存しない操作を提供する。

use core::num::NonZeroUsize;

use crate::eval::{Pst, pst};
use crate::search::algorithm::SearchAlgorithm;
use crate::search::alphabeta;
use crate::search::handle::SearchHandle;
use crate::search::limits::SearchLimits;
use crate::search::snapshot::SearchSnapshot;
use crate::search::{DEFAULT_THREADS, TranspositionTable};

/// 探索方式が宣言するオプションの名前と値の種類。
///
/// USIとCECPの文字列への変換はプロトコル層が行う。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OptionSpec {
    /// オプション名。
    pub name: &'static str,
    /// 値の種類と範囲。
    pub kind: OptionKind,
}

/// オプションの値の種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OptionKind {
    /// 範囲つきの整数。
    Spin {
        /// 既定値。
        default: i64,
        /// 最小値。
        min: i64,
        /// 最大値。
        max: i64,
    },
}

/// 起動時に選ばれた探索方式と、その方式が探索の間で持ち越す状態。
///
/// 探索の開始では値ごと探索スレッドへ移り、[`SearchHandle::join`]が返す。
pub enum Searcher {
    /// αβ探索。
    AlphaBeta(alphabeta::Engine),
}

impl Searcher {
    /// 指定した方式を既定の設定で作る。置換表のような大きな領域はまだ確保しない。
    ///
    /// # Errors
    ///
    /// 埋め込みの評価重みを復号できない場合はエラーを返す。
    pub fn new(algorithm: SearchAlgorithm) -> Result<Self, pst::Error> {
        match algorithm {
            SearchAlgorithm::AlphaBeta => Ok(Self::AlphaBeta(alphabeta::Engine::new(
                crate::eval::weights()?,
                DEFAULT_THREADS,
            ))),
        }
    }

    /// 方式が宣言するオプションを宣言順に返す。
    pub fn options(&self) -> &'static [OptionSpec] {
        match self {
            Self::AlphaBeta(engine) => engine.options(),
        }
    }

    /// 方式が宣言したオプションなら値を設定する。
    ///
    /// 名前が方式のオプションでなければ`None`を返す。値が不正な場合や
    /// 資源を確保できない場合は、状態を変えずに理由を`Some(Err)`で返す。
    pub fn set_option(&mut self, name: &str, value: Option<&str>) -> Option<Result<(), String>> {
        match self {
            Self::AlphaBeta(engine) => engine.set_option(name, value),
        }
    }

    /// 置換表の容量をMB単位で設定する。CECPの`memory`に使う。
    ///
    /// # Errors
    ///
    /// 置換表を確保できない場合は理由を返し、状態を変えない。
    pub fn set_hash_mb(&mut self, size_mb: usize) -> Result<(), String> {
        match self {
            Self::AlphaBeta(engine) => engine
                .set_hash_mb(size_mb)
                .map_err(|error| error.to_string()),
        }
    }

    /// 次の探索に使うワーカー数を返す。
    pub fn threads(&self) -> NonZeroUsize {
        match self {
            Self::AlphaBeta(engine) => engine.threads(),
        }
    }

    /// 次の探索に使うワーカー数を設定する。CECPの`cores`に使う。
    pub fn set_threads(&mut self, threads: NonZeroUsize) {
        match self {
            Self::AlphaBeta(engine) => engine.set_threads(threads),
        }
    }

    /// 前の対局または別の規則で得た探索の情報を捨てる。
    pub fn new_game(&mut self) {
        match self {
            Self::AlphaBeta(engine) => engine.new_game(),
        }
    }

    /// 最初の探索の前に済ませておく資源の確保を行う。
    ///
    /// # Errors
    ///
    /// 資源を確保できない場合は理由を返す。
    pub fn prepare(&mut self) -> Result<(), String> {
        match self {
            Self::AlphaBeta(engine) => engine.prepare().map_err(|error| error.to_string()),
        }
    }

    /// 値ごと探索スレッドへ移して探索を開始する。`ponder`が真なら時間制限は
    /// 的中の通知まで無効とする。
    ///
    /// # Errors
    ///
    /// 探索に必要な資源を確保できない場合は、探索を開始せずに値と理由を返す。
    pub fn start(
        self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
    ) -> Result<SearchHandle, (Self, String)> {
        match self {
            Self::AlphaBeta(engine) => engine
                .start(snapshot, limits, search_id, ponder)
                .map_err(|(engine, error)| (Self::AlphaBeta(engine), error.to_string())),
        }
    }

    /// 静的評価に使う学習PSTを返す。診断コマンド`eval`に使う。
    pub fn pst(&self) -> &Pst {
        match self {
            Self::AlphaBeta(engine) => engine.pst(),
        }
    }

    /// 確保済みの置換表を返す。診断コマンド`tt`に使う。
    pub fn transposition_table(&self) -> Option<&TranspositionTable> {
        match self {
            Self::AlphaBeta(engine) => engine.transposition_table(),
        }
    }

    /// 試験用に、探索後のαβ探索の置換表とhistory表を取り出す。
    #[cfg(test)]
    pub(crate) fn into_alphabeta_parts(self) -> (TranspositionTable, crate::search::HistoryTables) {
        match self {
            Self::AlphaBeta(engine) => engine.into_parts(),
        }
    }

    /// 試験用に、αβ探索の状態を返す。
    #[cfg(test)]
    pub(crate) fn alphabeta(&self) -> &alphabeta::Engine {
        match self {
            Self::AlphaBeta(engine) => engine,
        }
    }

    /// 試験用に、αβ探索の状態を可変で返す。
    #[cfg(test)]
    pub(crate) fn alphabeta_mut(&mut self) -> &mut alphabeta::Engine {
        match self {
            Self::AlphaBeta(engine) => engine,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searcher() -> Searcher {
        Searcher::new(SearchAlgorithm::AlphaBeta).unwrap()
    }

    // alphazero.md「共存の構造」: αβ探索はUSI_HashとThreadsをこの順に宣言する。
    #[test]
    fn alphabeta_declares_hash_and_threads_in_order() {
        assert_eq!(
            searcher().options(),
            [
                OptionSpec {
                    name: "USI_Hash",
                    kind: OptionKind::Spin {
                        default: 256,
                        min: 1,
                        max: 65_536,
                    },
                },
                OptionSpec {
                    name: "Threads",
                    kind: OptionKind::Spin {
                        default: 1,
                        min: 1,
                        max: 256,
                    },
                },
            ]
        );
    }

    // 同節: 方式のオプションでない名前は呼び出し側へ返し、状態を変えない。
    #[test]
    fn set_option_leaves_unknown_names_to_the_caller() {
        let mut searcher = searcher();
        assert_eq!(searcher.set_option("ResignValue", Some("1")), None);
        assert_eq!(searcher.threads(), DEFAULT_THREADS);
        assert!(searcher.transposition_table().is_none());
    }

    // 同節: 名前は大文字小文字を区別せず、不正値は状態を変えずに理由を返す。
    #[test]
    fn set_option_validates_hash_and_threads() {
        let mut searcher = searcher();
        assert_eq!(searcher.set_option("threads", Some("4")), Some(Ok(())));
        assert_eq!(searcher.threads().get(), 4);
        for (name, value, message) in [
            ("Threads", None, "Threads requires a value"),
            (
                "Threads",
                Some("0"),
                "Threads must be an integer from 1 to 256",
            ),
            (
                "Threads",
                Some("257"),
                "Threads must be an integer from 1 to 256",
            ),
            ("USI_Hash", None, "USI_Hash requires a value"),
            (
                "usi_hash",
                Some("0"),
                "USI_Hash must be an integer from 1 to 65536",
            ),
            (
                "USI_Hash",
                Some("65537"),
                "USI_Hash must be an integer from 1 to 65536",
            ),
        ] {
            assert_eq!(
                searcher.set_option(name, value),
                Some(Err(message.to_owned()))
            );
        }
        assert_eq!(searcher.threads().get(), 4);
        assert!(searcher.transposition_table().is_none());
        assert_eq!(searcher.set_option("USI_Hash", Some("1")), Some(Ok(())));
        assert!(searcher.transposition_table().is_some());
    }
}

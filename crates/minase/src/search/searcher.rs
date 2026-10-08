//! 選ばれた探索方式の状態を包み、プロトコル層へ方式に依存しない操作を提供する。

use core::num::NonZeroUsize;

use crate::eval::{Pst, pst};
use crate::search::algorithm::SearchAlgorithm;
use crate::search::handle::SearchHandle;
use crate::search::limits::SearchLimits;
use crate::search::snapshot::SearchSnapshot;
use crate::search::{DEFAULT_THREADS, TranspositionTable};
use crate::search::{alphabeta, mcts};

/// MCTSで深さだけが指定された探索に代わりに与えるシミュレーション回数。
///
/// AlphaZeroの自己対局の1手あたりの探索回数800を流用した暫定値であり、深さの制限を誤って
/// 渡された対局が止まらなくなることを防ぐためだけに使う。
const DEPTH_ONLY_SIMULATIONS: u64 = 800;

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
    /// 列挙した候補から1つを選ぶ値。
    Combo {
        /// 既定値。
        default: &'static str,
        /// 候補。
        vars: &'static [&'static str],
    },
    /// 文字列。
    String {
        /// 既定値。空ならUSIでは`<empty>`と書く。
        default: &'static str,
    },
}

/// 起動時に選ばれた探索方式と、その方式が探索の間で持ち越す状態。
///
/// 探索の開始では値ごと探索スレッドへ移り、[`SearchHandle::join`]が返す。
pub enum Searcher {
    /// αβ探索。
    AlphaBeta(alphabeta::Engine),
    /// MCTS。
    Mcts(mcts::Engine),
}

impl Searcher {
    /// 指定した方式を既定の設定で作る。置換表や推論のセッションのような大きな資源はまだ用意しない。
    ///
    /// MCTSは、評価値への写像に使う勝率の尺度Kを学習PSTから取る。
    ///
    /// # Errors
    ///
    /// 埋め込みの評価重みを復号できない場合はエラーを返す。
    pub fn new(algorithm: SearchAlgorithm) -> Result<Self, pst::Error> {
        let weights = crate::eval::weights()?;
        Ok(match algorithm {
            SearchAlgorithm::AlphaBeta => {
                Self::AlphaBeta(alphabeta::Engine::new(weights, DEFAULT_THREADS))
            }
            SearchAlgorithm::Mcts => Self::Mcts(mcts::Engine::for_play(weights.k())),
        })
    }

    /// 試験用に、推論器を差し込んだ対局用の設定のMCTSを作る。
    #[cfg(test)]
    pub(crate) fn mcts_with_evaluator(
        evaluator: Box<dyn crate::nn::evaluator::Evaluator + Send>,
    ) -> Self {
        let weights = crate::eval::weights().expect("embedded evaluation weights must be valid");
        let config = mcts::Engine::for_play(weights.k()).config();
        Self::Mcts(mcts::Engine::new(evaluator, config))
    }

    /// 方式を返す。
    pub fn algorithm(&self) -> SearchAlgorithm {
        match self {
            Self::AlphaBeta(_) => SearchAlgorithm::AlphaBeta,
            Self::Mcts(_) => SearchAlgorithm::Mcts,
        }
    }

    /// 方式が宣言するオプションを宣言順に返す。
    pub fn options(&self) -> &'static [OptionSpec] {
        match self {
            Self::AlphaBeta(engine) => engine.options(),
            Self::Mcts(engine) => engine.options(),
        }
    }

    /// 方式が宣言したオプションなら値を設定する。
    ///
    /// 名前が方式のオプションでなければ`None`を返す。ただし`USI_Hash`を宣言しない方式に
    /// `USI_Hash`を指定した場合は、αβ探索との取り違えに気付けるように理由を`Some(Err)`で返す。
    /// 値が不正な場合や資源を確保できない場合は、状態を変えずに理由を`Some(Err)`で返す。
    pub fn set_option(&mut self, name: &str, value: Option<&str>) -> Option<Result<(), String>> {
        if name.eq_ignore_ascii_case("USI_Hash") && !self.declares_hash() {
            return Some(Err(self.hash_unsupported()));
        }
        match self {
            Self::AlphaBeta(engine) => engine.set_option(name, value),
            Self::Mcts(engine) => engine.set_option(name, value),
        }
    }

    /// 方式が`USI_Hash`を宣言するかどうかを返す。置換表を持つαβ探索だけが宣言する。
    pub fn declares_hash(&self) -> bool {
        self.options()
            .iter()
            .any(|option| option.name == "USI_Hash")
    }

    /// `USI_Hash`に当たる置換表の容量をMB単位で設定する。CECPの`memory`に使う。
    ///
    /// # Errors
    ///
    /// 方式が`USI_Hash`を宣言しない場合（[`Searcher::declares_hash`]）、または置換表を
    /// 確保できない場合は理由を返し、状態を変えない。
    pub fn set_hash_mb(&mut self, size_mb: usize) -> Result<(), String> {
        match self {
            Self::AlphaBeta(engine) => engine
                .set_hash_mb(size_mb)
                .map_err(|error| error.to_string()),
            Self::Mcts(_) => Err(self.hash_unsupported()),
        }
    }

    /// 方式が`USI_Hash`を宣言しないことを告げる理由。
    fn hash_unsupported(&self) -> String {
        format!(
            "USI_Hash is not an option of the {} search",
            self.algorithm()
        )
    }

    /// 次の探索に使うワーカー数を返す。
    pub fn threads(&self) -> NonZeroUsize {
        match self {
            Self::AlphaBeta(engine) => engine.threads(),
            Self::Mcts(_) => DEFAULT_THREADS,
        }
    }

    /// 次の探索に使うワーカー数を設定する。CECPの`cores`に使う。
    ///
    /// # Errors
    ///
    /// 方式がそのワーカー数で探索できない場合は理由を返し、状態を変えない。
    pub fn set_threads(&mut self, threads: NonZeroUsize) -> Result<(), String> {
        match self {
            Self::AlphaBeta(engine) => {
                engine.set_threads(threads);
                Ok(())
            }
            Self::Mcts(engine) => engine.set_threads(threads),
        }
    }

    /// 前の対局または別の規則で得た探索の情報を捨てる。
    pub fn new_game(&mut self) {
        match self {
            Self::AlphaBeta(engine) => engine.new_game(),
            Self::Mcts(engine) => engine.new_game(),
        }
    }

    /// 最初の探索の前に済ませておく資源の確保を行う。MCTSでは推論のセッションを作る。
    ///
    /// # Errors
    ///
    /// 資源を確保できない場合は理由を返す。
    pub fn prepare(&mut self) -> Result<(), String> {
        match self {
            Self::AlphaBeta(engine) => engine.prepare().map_err(|error| error.to_string()),
            Self::Mcts(engine) => engine.prepare(),
        }
    }

    /// プロトコル層が受けた探索制限を、方式が解釈できる形へ直す。直した場合は利用者への説明を返す。
    ///
    /// MCTSには深さに対応する概念がないので、深さの制限を除き、ノード数と時間の制限だけで探索する。
    /// 深さだけが指定された場合は、対局が止まらなくなることを防ぐために
    /// シミュレーション回数の上限800で探索する。
    pub fn adapt_limits(&self, limits: SearchLimits) -> (SearchLimits, Option<String>) {
        let (Self::Mcts(_), Some(depth)) = (self, limits.depth()) else {
            return (limits, None);
        };
        let (nodes, movetime_ms, clock) = (limits.nodes(), limits.movetime_ms(), limits.clock());
        let (nodes, rest) = if nodes.is_none() && movetime_ms.is_none() && clock.is_none() {
            (
                Some(DEPTH_ONLY_SIMULATIONS),
                format!("searching {DEPTH_ONLY_SIMULATIONS} simulations instead"),
            )
        } else {
            (
                nodes,
                "searching with the node and time limits only".to_owned(),
            )
        };
        let limits = SearchLimits::new(None, nodes, movetime_ms, clock)
            .expect("the remaining limits were already validated and at least one is set");
        (
            limits,
            Some(format!(
                "MCTS does not interpret depth {depth} as a simulation limit; {rest}"
            )),
        )
    }

    /// ノード数の定義がαβ探索と異なる方式なら、その定義の説明を返す。
    ///
    /// MCTSのノード数はシミュレーションの回数であり、αβ探索のノード数（着手を盤面へ適用した回数）とは
    /// 定義が異なる（docs/plans/alphazero.md「共存の構造」）。
    pub fn node_count_notice(&self) -> Option<&'static str> {
        match self {
            Self::AlphaBeta(_) => None,
            Self::Mcts(_) => Some(
                "MCTS reports nodes as the number of simulations, not as the number of moves \
                 made on the board as in the alphabeta search",
            ),
        }
    }

    /// 値ごと探索スレッドへ移して探索を開始する。`ponder`が真なら時間制限は
    /// 的中の通知まで無効とする。MCTSの推論のセッションがなければ、ここで作る。
    ///
    /// # Errors
    ///
    /// 探索に必要な資源を確保できない場合と、方式が制限を解釈できない場合は、
    /// 探索を開始せずに値と理由を返す。
    // 失敗時は状態を値のまま呼び出し側へ返すので、Errは状態の大きさを持つ。
    #[allow(clippy::result_large_err)]
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
            Self::Mcts(mut engine) => {
                if let Err(error) = engine.prepare() {
                    return Err((Self::Mcts(engine), error));
                }
                engine
                    .start(snapshot, limits, search_id, ponder)
                    .map_err(|(engine, error)| (Self::Mcts(engine), error.to_string()))
            }
        }
    }

    /// αβ探索の静的評価に使う学習PSTを返す。MCTSでは`None`を返す。診断コマンド`eval`に使う。
    pub fn pst(&self) -> Option<&Pst> {
        match self {
            Self::AlphaBeta(engine) => Some(engine.pst()),
            Self::Mcts(_) => None,
        }
    }

    /// 確保済みの置換表を返す。MCTSでは`None`を返す。診断コマンド`tt`に使う。
    pub fn transposition_table(&self) -> Option<&TranspositionTable> {
        match self {
            Self::AlphaBeta(engine) => engine.transposition_table(),
            Self::Mcts(_) => None,
        }
    }

    /// 試験用に、探索後のαβ探索の置換表とhistory表を取り出す。
    #[cfg(test)]
    pub(crate) fn into_alphabeta_parts(self) -> (TranspositionTable, crate::search::HistoryTables) {
        match self {
            Self::AlphaBeta(engine) => engine.into_parts(),
            Self::Mcts(_) => panic!("the searcher is not the alphabeta search"),
        }
    }

    /// 試験用に、αβ探索の状態を返す。
    #[cfg(test)]
    pub(crate) fn alphabeta(&self) -> &alphabeta::Engine {
        match self {
            Self::AlphaBeta(engine) => engine,
            Self::Mcts(_) => panic!("the searcher is not the alphabeta search"),
        }
    }

    /// 試験用に、αβ探索の状態を可変で返す。
    #[cfg(test)]
    pub(crate) fn alphabeta_mut(&mut self) -> &mut alphabeta::Engine {
        match self {
            Self::AlphaBeta(engine) => engine,
            Self::Mcts(_) => panic!("the searcher is not the alphabeta search"),
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

    // 同節: ハーネスが既定値を照会するので、MCTSもThreadsを宣言し、続けて探索木の
    // 節点数の上限UCT_NodeLimitを宣言する。置換表を持たないのでUSI_Hashは宣言せず、
    // USI_HashとCECPのmemoryに当たる容量の設定は理由を返して拒む。
    // MCTSは静的評価のPSTも置換表も持たない。
    #[test]
    fn mcts_declares_threads_but_not_hash() {
        let searcher = Searcher::new(SearchAlgorithm::Mcts).unwrap();
        assert_eq!(searcher.algorithm(), SearchAlgorithm::Mcts);
        let names: Vec<_> = searcher
            .options()
            .iter()
            .map(|option| option.name)
            .collect();
        assert_eq!(names[..2], ["Threads", "UCT_NodeLimit"]);
        assert!(!names.contains(&"USI_Hash"));
        assert!(!searcher.declares_hash());
        assert!(searcher.pst().is_none());
        assert!(searcher.transposition_table().is_none());

        let mut searcher = searcher;
        let message = "USI_Hash is not an option of the mcts search";
        for value in [Some("1"), None] {
            assert_eq!(
                searcher.set_option("usi_hash", value),
                Some(Err(message.to_owned()))
            );
        }
        assert_eq!(searcher.set_hash_mb(1), Err(message.to_owned()));
        assert!(searcher.transposition_table().is_none());
        assert!(self::searcher().declares_hash());
    }

    // 指示書E第7項: 深さの制限を直すのはMCTSだけであり、αβ探索の制限はそのまま渡す。
    #[test]
    fn only_mcts_adapts_depth_limits() {
        let depth = SearchLimits::new(Some(3), None, None, None).unwrap();
        assert_eq!(searcher().adapt_limits(depth), (depth, None));
        let mcts = Searcher::new(SearchAlgorithm::Mcts).unwrap();
        let (limits, notice) = mcts.adapt_limits(depth);
        assert_eq!(
            limits,
            SearchLimits::new(None, Some(800), None, None).unwrap()
        );
        assert!(notice.is_some());
        let nodes = SearchLimits::new(None, Some(5), None, None).unwrap();
        assert_eq!(mcts.adapt_limits(nodes), (nodes, None));
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

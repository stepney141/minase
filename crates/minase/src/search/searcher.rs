//! 探索方式の選択、オプション、および対局内で持ち越す状態。

use core::{fmt, num::NonZeroUsize};

use super::{
    SearchHandle, SearchLimits, SearchSnapshot, TranspositionTable, TranspositionTableError,
    alphabeta, mcts,
};

/// 選ばれた探索方式と、その方式が対局の間に持ち越す状態。
pub enum Searcher {
    /// αβ探索。
    AlphaBeta(alphabeta::Engine),
    /// モンテカルロ木探索。
    Mcts(mcts::Engine),
}

/// 探索方式が宣言する、プロトコルの表記に依存しないオプション。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SearchOption {
    /// オプションの宣言名。
    pub name: &'static str,
    /// 値の種類と受理する範囲。
    pub kind: SearchOptionKind,
}

/// 探索オプションの値の種類。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchOptionKind {
    /// 範囲を持つ非負の整数。
    Spin {
        /// 既定値。
        default: usize,
        /// 最小値。境界を含む。
        min: usize,
        /// 最大値。境界を含む。
        max: usize,
    },
    /// 自由な文字列。
    String {
        /// 既定値。空文字列も受理する。
        default: String,
    },
    /// 大文字小文字を区別しない選択肢。
    Combo {
        /// 既定値。
        default: &'static str,
        /// 選択肢の宣言順。
        choices: &'static [&'static str],
    },
}

/// プロトコル層が解析した探索オプションの値。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchOptionValue {
    /// 整数オプションの値。
    Spin(usize),
    /// 文字列オプションの値。
    String(String),
    /// 選択肢オプションの値。
    Combo(String),
}

impl fmt::Display for SearchOptionValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spin(value) => value.fmt(formatter),
            Self::String(value) | Self::Combo(value) => value.fmt(formatter),
        }
    }
}

impl SearchOption {
    /// プロトコルを通さない設定も、宣言された種類と範囲で検査する。
    pub(super) fn validate(&self, value: &SearchOptionValue) -> Result<(), SetOptionError> {
        match (&self.kind, value) {
            (SearchOptionKind::Spin { min, max, .. }, SearchOptionValue::Spin(value)) => {
                if !(min..=max).contains(&value) {
                    return Err(SetOptionError::OutOfRange {
                        name: self.name,
                        value: *value,
                        min: *min,
                        max: *max,
                    });
                }
            }
            (SearchOptionKind::String { .. }, SearchOptionValue::String(_)) => {}
            (SearchOptionKind::Combo { choices, .. }, SearchOptionValue::Combo(value))
                if choices
                    .iter()
                    .any(|choice| choice.eq_ignore_ascii_case(value)) => {}
            _ => {
                return Err(SetOptionError::InvalidValue {
                    name: self.name,
                    value: value.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// 探索オプションを適用できない理由。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SetOptionError {
    /// 探索方式が宣言していない名前。
    UnknownOption {
        /// 指定された名前。
        name: String,
    },
    /// 宣言された範囲外の整数。
    OutOfRange {
        /// オプションの宣言名。
        name: &'static str,
        /// 指定された値。
        value: usize,
        /// 最小値。
        min: usize,
        /// 最大値。
        max: usize,
    },
    /// 種類または選択肢が宣言と一致しない値。
    InvalidValue {
        /// オプションの宣言名。
        name: &'static str,
        /// 指定された値。
        value: String,
    },
    /// 置換表の確保または容量の変更に失敗した。
    TranspositionTable(TranspositionTableError),
}

impl fmt::Display for SetOptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOption { name } => write!(formatter, "unknown search option {name}"),
            Self::OutOfRange { name, min, max, .. } => {
                write!(formatter, "{name} must be an integer from {min} to {max}")
            }
            Self::InvalidValue { name, value } => {
                write!(formatter, "invalid {name} value '{value}'")
            }
            Self::TranspositionTable(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SetOptionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TranspositionTable(error) => Some(error),
            Self::UnknownOption { .. } | Self::OutOfRange { .. } | Self::InvalidValue { .. } => {
                None
            }
        }
    }
}

/// 探索前の推論器の準備に失敗した理由。
#[derive(Debug)]
pub enum PrepareError {
    /// TensorRTのキャッシュ先が指定されていない。
    MissingCacheDir,
    /// ネットの推論セッションを作成できない。
    Network(crate::nn::NnError),
}

impl fmt::Display for PrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCacheDir => {
                formatter.write_str("NnCacheDir must be set for NnBackend tensorrt")
            }
            Self::Network(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PrepareError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MissingCacheDir => None,
            Self::Network(error) => Some(error),
        }
    }
}

/// 探索スレッドを開始する前の準備に失敗した理由。
#[derive(Clone, Debug)]
pub enum StartError {
    /// 埋め込みNNUEの重みを読み込めない。
    Weights(crate::eval::nnue::Error),
    /// MCTSの入力または設定が不正。
    Mcts(mcts::Error),
    /// 置換表を確保できない。
    TranspositionTable(TranspositionTableError),
}

impl fmt::Display for StartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Weights(error) => error.fmt(formatter),
            Self::Mcts(error) => error.fmt(formatter),
            Self::TranspositionTable(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Weights(error) => Some(error),
            Self::Mcts(error) => Some(error),
            Self::TranspositionTable(error) => Some(error),
        }
    }
}

/// 開始に失敗した探索器と、その原因。
pub struct StartFailure {
    /// 開始前の状態を保持した探索器。
    pub searcher: Searcher,
    /// 探索を開始できなかった理由。
    pub error: StartError,
}

impl fmt::Debug for StartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for StartFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for StartFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl Searcher {
    /// 置換表を未確保とし、既定のワーカー数でαβ探索器を作る。
    pub fn alpha_beta() -> Self {
        Self::AlphaBeta(alphabeta::Engine::new())
    }

    /// NNUEの勝率尺度を使い、指定されたネットでMCTSを作る。
    /// 推論器は[`Self::prepare`]で作成する。
    ///
    /// # Errors
    /// 勝率尺度を持つNNUEの重みを読み込めない場合は拒否する。
    pub fn mcts(network: crate::nn::Network) -> Result<Self, StartError> {
        let scale = crate::eval::weights()
            .map_err(StartError::Weights)?
            .cp_per_logit();
        Ok(Self::Mcts(mcts::Engine::new(
            network,
            mcts::Config::new(scale),
        )))
    }

    /// 選ばれた方式のオプションを宣言順に返す。
    pub fn options(&self) -> Vec<SearchOption> {
        match self {
            Self::AlphaBeta(engine) => engine.options(),
            Self::Mcts(engine) => engine.options(),
        }
    }

    /// 宣言どおりの名前でオプションを適用する。
    /// 名前の大文字小文字の照合と文字列の解析はプロトコル層が行う。
    /// 値の種類と範囲は探索器でも検査する。
    ///
    /// # Errors
    /// 名前が未知、値の種類・範囲・選択肢が不正、または必要なメモリを確保できない場合は、
    /// 状態を変えずに元の原因を保持したエラーを返す。
    pub fn set_option(
        &mut self,
        name: &str,
        value: SearchOptionValue,
    ) -> Result<(), SetOptionError> {
        match self {
            Self::AlphaBeta(engine) => engine.set_option(name, value),
            Self::Mcts(engine) => engine.set_option(name, value),
        }
    }

    /// 現在の探索ワーカー数を返す。
    pub fn threads(&self) -> NonZeroUsize {
        match self {
            Self::AlphaBeta(engine) => engine.threads,
            Self::Mcts(_) => super::DEFAULT_THREADS,
        }
    }

    /// 対局をまたいで持ち越さない探索状態を消去する。
    pub fn clear(&mut self) {
        match self {
            Self::AlphaBeta(engine) => engine.clear(),
            Self::Mcts(engine) => engine.clear(),
        }
    }

    /// 探索前に必要な領域と推論器を準備する。
    /// αβ探索では未確保の置換表だけを既定容量で確保する。
    /// 置換表の確保の失敗は次の[`Self::start`]で再試行して報告する。
    /// MCTSでは、未作成または設定変更で破棄された推論器を作成する。
    ///
    /// # Errors
    /// MCTSのキャッシュ先が未設定、または推論器を作れない場合は原因を返す。
    pub fn prepare(&mut self) -> Result<(), PrepareError> {
        match self {
            Self::AlphaBeta(engine) => {
                engine.prepare();
                Ok(())
            }
            Self::Mcts(engine) => engine.prepare(),
        }
    }

    /// 診断用に確保済みの置換表を返す。
    pub fn transposition_table(&self) -> Option<&TranspositionTable> {
        match self {
            Self::AlphaBeta(engine) => engine.transposition_table.as_ref(),
            Self::Mcts(_) => None,
        }
    }

    /// 自身を別スレッドへ移して探索を始める。
    /// `ponder`が真なら時間制限は的中の通知まで無効とする。
    ///
    /// 旧探索を置き換える場合は、そのハンドルへ停止を要求して
    /// [`SearchHandle::join`]で回収してから新しい入力を渡す。
    /// 遅れて届いた通知は[`super::SearchEvent::search_id`]で識別する。
    /// 探索ワーカーがパニックした場合は残るワーカーへ停止を通知し、
    /// `join`がペイロードを返す。この場合は`Finished`を送信しない。
    /// MCTSの推論が失敗した場合も`Finished`を送らず、`join`が元の`NnError`を返す。
    ///
    /// # Errors
    /// αβ探索は重みの取得、置換表の確保の順に準備する。
    /// MCTSは推論器、深さ制限、成り規則、および設定を検査する。
    /// 失敗した場合は探索を始めず、探索器と原因を返す。
    #[allow(clippy::result_large_err)] // 再試行に必要な探索状態を所有権ごと返す。
    pub fn start(
        self,
        snapshot: SearchSnapshot,
        limits: SearchLimits,
        search_id: u64,
        ponder: bool,
    ) -> Result<SearchHandle, StartFailure> {
        match self {
            Self::AlphaBeta(engine) => engine.start(snapshot, limits, search_id, ponder),
            Self::Mcts(engine) => engine.start(snapshot, limits, search_id, ponder),
        }
    }

    /// 対局内の状態保持を検査するため、履歴表を参照する。
    #[cfg(test)]
    pub(crate) fn histories_for_test(&self) -> &super::HistoryTables {
        match self {
            Self::AlphaBeta(engine) => &engine.histories,
            Self::Mcts(_) => panic!("alpha-beta histories requested by a test of MCTS"),
        }
    }

    /// 対局内の状態保持を検査するため、履歴表へ値を埋める。
    #[cfg(test)]
    pub(crate) fn histories_mut_for_test(&mut self) -> &mut super::HistoryTables {
        match self {
            Self::AlphaBeta(engine) => &mut engine.histories,
            Self::Mcts(_) => panic!("alpha-beta histories requested by a test of MCTS"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    // 探索方式の境界は、宣言順と既定値を固定し、プロトコルを通さない不正入力も拒否する。
    #[test]
    fn alpha_beta_options_reject_unknown_names_and_out_of_range_values() {
        let mut searcher = Searcher::alpha_beta();
        let options: Vec<_> = searcher
            .options()
            .into_iter()
            .map(|option| {
                let SearchOptionKind::Spin { default, min, max } = option.kind else {
                    panic!("alpha-beta declares only spin options");
                };
                (option.name, default, min, max)
            })
            .collect();
        assert_eq!(
            options,
            [("USI_Hash", 256, 1, 65_536), ("Threads", 1, 1, 256)]
        );
        assert_eq!(searcher.threads().get(), 1);
        assert!(searcher.transposition_table().is_none());
        searcher.histories_mut_for_test().workers[0][0][60][60] = 123;
        for (name, value) in [
            ("USI_Hash", 0),
            ("USI_Hash", 65_537),
            ("Threads", 0),
            ("Threads", 257),
        ] {
            assert!(matches!(
                searcher.set_option(name, SearchOptionValue::Spin(value)),
                Err(SetOptionError::OutOfRange { value: rejected, .. }) if rejected == value
            ));
        }
        assert!(matches!(
            searcher.set_option("Unknown", SearchOptionValue::Spin(1)),
            Err(SetOptionError::UnknownOption { name }) if name == "Unknown"
        ));
        assert_eq!(searcher.threads().get(), 1);
        assert!(searcher.transposition_table().is_none());
        assert_eq!(searcher.histories_for_test().workers[0][0][60][60], 123);
    }

    // メモリ確保の失敗は文字列へ失わず、従来の表示と元のエラー型を保持する。
    #[test]
    fn search_boundary_errors_preserve_allocation_causes_and_messages() {
        let cause = TranspositionTableError::AllocationFailed;
        let option_error = SetOptionError::TranspositionTable(cause);
        assert_eq!(option_error.to_string(), cause.to_string());
        assert_eq!(
            option_error
                .source()
                .unwrap()
                .downcast_ref::<TranspositionTableError>(),
            Some(&cause)
        );
        let start_error = StartError::TranspositionTable(cause);
        assert_eq!(start_error.to_string(), cause.to_string());
        assert_eq!(
            start_error
                .source()
                .unwrap()
                .downcast_ref::<TranspositionTableError>(),
            Some(&cause)
        );
    }

    #[test]
    fn search_options_reject_wrong_value_kinds_without_panicking() {
        assert!(matches!(
            Searcher::alpha_beta().set_option("Threads", SearchOptionValue::String("1".to_owned())),
            Err(SetOptionError::InvalidValue { .. })
        ));
        let mut searcher = Searcher::mcts(crate::nn::Network::from_bytes(Vec::new())).unwrap();
        for (name, value) in [
            ("Threads", SearchOptionValue::Combo("1".to_owned())),
            ("NnBackend", SearchOptionValue::Spin(1)),
            ("NnCacheDir", SearchOptionValue::Combo("/cache".to_owned())),
        ] {
            assert!(matches!(
                searcher.set_option(name, value),
                Err(SetOptionError::InvalidValue { .. })
            ));
        }
    }

    #[test]
    fn preparation_error_preserves_the_network_error_source() {
        let error = PrepareError::Network(crate::nn::NnError::RuntimeLibraryPathUnset);
        assert!(matches!(
            error.source().unwrap().downcast_ref::<crate::nn::NnError>(),
            Some(crate::nn::NnError::RuntimeLibraryPathUnset)
        ));
    }
}

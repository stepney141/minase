//! 明示されたプロトコルでエンジンを起動する。
//!
//! サブコマンドなしの起動では[`super::Arguments`]が`--protocol usi|cecp`を必須とし、
//! 既定値やプロトコルの自動判別を設けない。規則は起動後に`RuleSet`で指定する。
//! `--search`の既定は`alphabeta`で、MCTSはUSIだけで使える。
//! MCTSのネットは`NnModel`で指定し、読み込みと推論器の作成を`isready`で行う。
//! 指定不備、読み込み失敗、推論器の作成失敗は
//! `error: <理由>`を標準エラーへ出して終了コード1で終え、別のネットや実行プロバイダへ切り替えない。
//!
//! 起動時に埋め込み評価重みを検証し、不正なら
//! `error: embedded evaluation weights are invalid: <理由>`を標準エラーへ出して終了コード1で終える。
//! 標準入力を専用スレッドで読み、チャネル経由で選んだプロトコルの`run_channel`へ渡す。
//! セッションの入出力エラーも標準エラーへ報告して終了コード1で終える。
//! 入出力ログの契約は[`super::io_log`]に従う。

use std::error::Error;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::process;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Instant;

use super::io_log::{IoLog, LoggedOutput, read_input};

use super::{ProtocolKind, SearchKind};
use minase::protocol::{CecpProtocol, UsiProtocol};
use minase::search::Searcher;

#[derive(Debug)]
enum SearchArgumentError {
    CecpUnsupported,
}

impl std::fmt::Display for SearchArgumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::CecpUnsupported => "--search mcts requires --protocol usi; CECP is not supported",
        })
    }
}

impl Error for SearchArgumentError {}

fn create_searcher(protocol: ProtocolKind, search: SearchKind) -> Result<Searcher, Box<dyn Error>> {
    match search {
        SearchKind::Alphabeta => Ok(Searcher::alpha_beta()),
        SearchKind::Mcts => {
            if matches!(protocol, ProtocolKind::Cecp) {
                return Err(SearchArgumentError::CecpUnsupported.into());
            }
            Ok(Searcher::mcts()?)
        }
    }
}

/// 本モジュールの起動契約に従って評価重みを検査し、セッションを開始する。
/// 実行時のエラーは標準エラーへ報告し、終了コード1で終える。
pub(super) fn main(protocol: ProtocolKind, io_log: Option<PathBuf>, search: SearchKind) {
    let started = Instant::now();
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = run(started, protocol, io_log, search) {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

/// 指定プロトコルでセッションを実行する。
///
/// USIとCECPは探索中もコマンドを受けるため、標準入力を
/// reader threadで読んでチャネル経由で処理する。
fn run(
    started: Instant,
    protocol: ProtocolKind,
    io_log: Option<PathBuf>,
    search: SearchKind,
) -> Result<(), Box<dyn Error>> {
    let searcher = create_searcher(protocol, search)?;
    let log = io_log
        .map(|path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map(|file| Arc::new(IoLog::new(started, file)))
        })
        .transpose()?;
    let (sender, receiver) = mpsc::channel();
    let input_log = log.clone();
    thread::spawn(move || {
        read_input(io::stdin().lock(), sender, input_log.as_deref());
    });
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    let mut logged_output;
    let output: &mut dyn io::Write = if let Some(log) = log {
        logged_output = LoggedOutput::new(&mut stdout, log);
        &mut logged_output
    } else {
        &mut stdout
    };

    match protocol {
        ProtocolKind::Usi => {
            let mut protocol = UsiProtocol::with_searcher(searcher);
            protocol.run_channel(&receiver, output)?;
            Ok(())
        }
        ProtocolKind::Cecp => {
            let mut protocol = CecpProtocol::new();
            protocol.run_channel(&receiver, output)?;
            Ok(())
        }
    }
}

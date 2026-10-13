//! 明示されたプロトコルと規則でエンジンを起動する。
//!
//! サブコマンドなしの起動では[`super::Arguments`]が`--protocol usi|cecp`と`--rules`を必須とし、
//! 既定値やプロトコルの自動判別を設けない。これらの起動引数は互換性の対象である。
//! `--rules`は[`minase_core::rules::parse_rule_set`]でUSIの規則オプションと同じ文法として解析する。
//! 構文エラーはclapが終了コード2で報告し、解析後の規則集合が不正なら本モジュールが
//! `error: invalid --rules value: <理由>`を標準エラーへ出して終了コード1で終える。
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

use super::{ProtocolKind, RuleSetArgument};
use minase::protocol::{CecpProtocol, Engine, UsiProtocol};

/// 本モジュールの起動契約に従って評価重みを検査し、セッションを開始する。
/// 実行時のエラーは標準エラーへ報告し、終了コード1で終える。
pub(super) fn main(protocol: ProtocolKind, rules: RuleSetArgument, io_log: Option<PathBuf>) {
    let started = Instant::now();
    if let Err(error) = minase::eval::weights() {
        eprintln!("error: embedded evaluation weights are invalid: {error}");
        process::exit(1);
    }
    if let Err(error) = run(started, protocol, rules, io_log) {
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
    rules: RuleSetArgument,
    io_log: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let log = io_log
        .map(|path| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map(|file| Arc::new(IoLog::new(started, file)))
        })
        .transpose()?;
    let mut engine =
        Engine::new(rules.0).map_err(|reason| format!("invalid --rules value: {reason}"))?;

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
            let mut protocol = UsiProtocol::new(&engine);
            protocol.run_channel(&mut engine, &receiver, output)?;
            Ok(())
        }
        ProtocolKind::Cecp => {
            let mut protocol = CecpProtocol::new(&engine);
            protocol.run_channel(&mut engine, &receiver, output)?;
            Ok(())
        }
    }
}

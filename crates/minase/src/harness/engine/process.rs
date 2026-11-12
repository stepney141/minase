//! 子プロセスの起動、送受信、および後始末。

use crate::harness::engine::cecp::{cecp_fixed_time_text, cecp_level_text, cecp_memory_text};
use crate::harness::engine::channel::receive_until;
use crate::harness::failure::EngineFailure;
use crate::harness::limit::SearchLimit;
use crate::harness::player::{PlayerConfig, Protocol};
use crate::harness::{NetworkError, NetworkSha256};
use minase_core::Move;
use std::io::BufReader;
use std::io::{BufRead, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Duration;
use std::{io, thread};

/// エンジンの異常と、測定全体を中止するネットの誤りを区別する。
#[derive(Debug)]
pub(in crate::harness) enum InitializationError {
    Failure(EngineFailure),
    Network(NetworkError),
}

impl std::fmt::Display for InitializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failure(error) => error.fmt(f),
            Self::Network(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for InitializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Failure(error) => Some(error),
            Self::Network(error) => Some(error),
        }
    }
}

impl From<EngineFailure> for InitializationError {
    fn from(error: EngineFailure) -> Self {
        Self::Failure(error)
    }
}

/// 読み取りスレッドからプロトコル出力を受け取る外部エンジン。
pub(in crate::harness) struct EngineProcess {
    /// エンジンの子プロセス。
    pub(super) child: Child,
    /// エンジンの標準入力。dropで読み取りスレッドの回収前に閉じる。
    pub(super) input: Option<ChildStdin>,
    /// 読み取りスレッドが送るUSI出力行。
    pub(super) lines: Receiver<io::Result<String>>,
    /// 読み取りスレッドのハンドル。dropで回収する。
    reader: Option<JoinHandle<()>>,
    /// 1回の応答を待つ期限。
    pub(super) timeout: Duration,
    /// エンジンとの通信プロトコル。
    pub(in crate::harness) protocol: Protocol,
    /// CECPエンジンへ送信済みとして扱う着手数。
    pub(super) sent_moves: usize,
    /// 待機中はNone、先読み中は予想した相手の着手。
    pub(super) pondering: Option<Move>,
}

impl EngineProcess {
    /// プロセスと標準出力読み取りスレッドを起動する。
    pub(super) fn spawn(
        path: &std::path::Path,
        args: &[String],
        protocol: Protocol,
        timeout: Duration,
    ) -> Result<Self, EngineFailure> {
        let mut child = Command::new(path)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| EngineFailure::Crash)?;
        let input = child.stdin.take().ok_or(EngineFailure::Crash)?;
        let output = child.stdout.take().ok_or(EngineFailure::Crash)?;
        let (sender, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let mut line = String::new();
                match output.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        while matches!(line.as_bytes().last(), Some(b'\n' | b'\r')) {
                            line.pop();
                        }
                        if sender.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            lines,
            reader: Some(reader),
            timeout,
            protocol,
            sent_moves: 0,
            pondering: None,
        })
    }

    /// 局ごとのプロセスを起動し、プロトコルの初期化を完了する。
    ///
    /// USIでは握手の後に[`Self::initialize_usi`]を行い、ネットの報告値と記録値を照合する。
    /// 一致した場合に`usinewgame`を送り、形式の誤りと不一致は反則負けから区別して返す。
    /// CECPの初期化列は[`super::cecp`]に従う。
    pub(in crate::harness) fn start(
        config: &PlayerConfig,
        seed: u64,
        timeout: Duration,
    ) -> Result<Self, InitializationError> {
        let mut process = Self::spawn(&config.path, &config.args, config.protocol, timeout)?;
        match config.protocol {
            Protocol::Usi => {
                process.read_usi_declarations()?;
                let received = process.initialize_usi(config, seed)?;
                if received != config.network_sha256 {
                    return Err(InitializationError::Network(NetworkError::Mismatch {
                        recorded: config.network_sha256.clone(),
                        received,
                    }));
                }
                process.send("usinewgame")?;
            }
            Protocol::Cecp => {
                process.send("xboard")?;
                process.send("protover 2")?;
                if process.read_cecp_features()? {
                    process.send(&format!("option RuleSet={}", config.rules_source))?;
                }
                process.send(&cecp_memory_text(config.hash_mb))?;
                process.send("new")?;
                process.send("variant chu")?;
                process.send("easy")?;
                process.send("nopost")?;
                process.send("force")?;
                match config.limit {
                    SearchLimit::Fixed {
                        depth: Some(depth),
                        nodes: None,
                    } => process.send(&format!("sd {depth}"))?,
                    SearchLimit::Time(time) if time.byoyomi_ms > 0 => {
                        process.send(&cecp_fixed_time_text(time))?;
                    }
                    SearchLimit::Time(time) => {
                        process.send(&cecp_level_text(time))?;
                    }
                    SearchLimit::Fixed { .. } | SearchLimit::Movetime { .. } => {
                        unreachable!("CECP limits are validated during player resolution")
                    }
                }
            }
        }
        Ok(process)
    }

    /// 握手後のオプション送信と`isready`を、事前の起動と各対局で共有する。
    ///
    /// `RuleSet`、明示された`USI_Hash`、`ResignValue 99999`、追加オプションの順に送る。
    /// `random`には`USI_Hash`を送らず、`ResignValue`の後に局ごとの`Seed`を送る。
    /// 投了の無効化は`docs/adr/disable-resignation-in-strength-measurements.md`に従う。
    /// `readyok`までに届くチェックサムは0行または1行だけを受理し、同じ値の重複も拒否する。
    /// 未報告を`None`とするのは、期限内に`readyok`を受け取った場合だけである。
    pub(super) fn initialize_usi(
        &mut self,
        config: &PlayerConfig,
        seed: u64,
    ) -> Result<Option<NetworkSha256>, InitializationError> {
        self.send(&format!(
            "setoption name RuleSet value {}",
            config.rules_source
        ))?;
        if !config.is_random
            && let Some(hash_mb) = config.hash_mb
        {
            self.send(&format!("setoption name USI_Hash value {hash_mb}"))?;
        }
        self.send("setoption name ResignValue value 99999")?;
        if config.is_random {
            self.send(&format!("setoption name Seed value {seed}"))?;
        }
        for (name, value) in &config.options {
            self.send(&format!("setoption name {name} value {value}"))?;
        }
        self.send("isready")?;
        let mut checksum = None;
        let mut error = None;
        self.receive_until(|line| {
            let mut words = line.split_whitespace();
            if words.next() == Some("info")
                && words.next() == Some("string")
                && words.next() == Some("network_sha256")
            {
                let received = words.collect::<Vec<_>>().join(" ");
                if let Some(first) = checksum.take() {
                    error = Some(NetworkError::Duplicate {
                        recorded: config.network_sha256.clone(),
                        first,
                        received,
                    });
                } else {
                    match NetworkSha256::try_from(received) {
                        Ok(value) => checksum = Some(value),
                        Err(source) => {
                            error = Some(NetworkError::Invalid {
                                recorded: config.network_sha256.clone(),
                                source,
                            })
                        }
                    }
                }
            }
            error.is_some() || line.trim() == "readyok"
        })?;
        match error {
            Some(error) => Err(InitializationError::Network(error)),
            None => Ok(checksum),
        }
    }

    /// エンジンへ1行を送る。
    pub(super) fn send(&mut self, line: &str) -> Result<(), EngineFailure> {
        let input = self.input.as_mut().ok_or(EngineFailure::Crash)?;
        writeln!(input, "{line}").map_err(|_| EngineFailure::Crash)?;
        input.flush().map_err(|_| EngineFailure::Crash)
    }

    /// 条件を満たす行を、呼び出し全体の期限まで受信する。
    pub(super) fn receive_until(
        &self,
        predicate: impl FnMut(&str) -> bool,
    ) -> Result<String, EngineFailure> {
        receive_until(&self.lines, self.timeout, predicate)
    }
}

impl Drop for EngineProcess {
    fn drop(&mut self) {
        self.stop_ponder();
        // 標準入力を先に閉じる。ラッパースクリプト経由で起動したエンジンは
        // killでは止まらず、入力のEOFで終了して初めて出力パイプが閉じるため、
        // この順序でないと読み取りスレッドの回収が止まる。
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

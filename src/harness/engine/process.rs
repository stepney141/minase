//! 子プロセスの起動、送受信、および後始末。

use crate::Move;
use crate::harness::engine::cecp::{cecp_fixed_time_text, cecp_level_text, cecp_memory_text};
use crate::harness::engine::channel::receive_until;
use crate::harness::failure::EngineFailure;
use crate::harness::limit::SearchLimit;
use crate::harness::player::{PlayerConfig, Protocol};
use std::io::BufReader;
use std::io::{BufRead, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::Duration;
use std::{io, thread};

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
    pub(super) fn spawn(config: &PlayerConfig, timeout: Duration) -> Result<Self, EngineFailure> {
        let mut child = Command::new(&config.path)
            .args(&config.args)
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
            protocol: config.protocol,
            sent_moves: 0,
            pondering: None,
        })
    }

    /// プロセスを起動し、プロトコル初期化列を完了する。
    pub(in crate::harness) fn start(
        config: &PlayerConfig,
        seed: u64,
        timeout: Duration,
    ) -> Result<Self, EngineFailure> {
        let mut process = Self::spawn(config, timeout)?;
        match config.protocol {
            Protocol::Usi => {
                process.read_usi_defaults()?;
                process.send(&format!(
                    "setoption name RuleSet value {}",
                    config.rules_source
                ))?;
                if !config.is_random
                    && let Some(hash_mb) = config.hash_mb
                {
                    process.send(&format!("setoption name USI_Hash value {hash_mb}"))?;
                }
                // 測定の対局は規則上の終局まで指させる（docs/plans/usi-resignation.md
                // 「棋力測定での無効化」）。オプションを持たないエンジンはこの行を無視する。
                process.send("setoption name ResignValue value 99999")?;
                if config.is_random {
                    process.send(&format!("setoption name Seed value {seed}"))?;
                }
                for (name, value) in &config.options {
                    process.send(&format!("setoption name {name} value {value}"))?;
                }
                process.send("isready")?;
                process.wait_for("readyok")?;
                process.send("usinewgame")?;
            }
            Protocol::Cecp => {
                process.send("xboard")?;
                process.send("protover 2")?;
                process.wait_for("feature done=1")?;
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
                    SearchLimit::Fixed { .. } => {
                        unreachable!("CECP limits are validated during player resolution")
                    }
                }
            }
        }
        Ok(process)
    }

    /// エンジンへ1行を送る。
    pub(super) fn send(&mut self, line: &str) -> Result<(), EngineFailure> {
        let input = self.input.as_mut().ok_or(EngineFailure::Crash)?;
        writeln!(input, "{line}").map_err(|_| EngineFailure::Crash)?;
        input.flush().map_err(|_| EngineFailure::Crash)
    }

    /// 指定行を期限まで読み進める。
    pub(super) fn wait_for(&self, expected: &str) -> Result<(), EngineFailure> {
        self.receive_until(|line| line.trim() == expected)
            .map(|_| ())
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

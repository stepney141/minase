//! 実行ファイル内で入出力の行を時刻付きで記録する。

use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

/// 入力スレッドと出力処理が共有するログ。
pub(super) struct IoLog<W> {
    /// エンジンを起動した時刻。
    started: Instant,
    /// 時刻の取得からフラッシュまでを直列化する書き込み先。
    writer: Mutex<W>,
}

impl<W: Write> IoLog<W> {
    /// 起動時刻と書き込み先からログを作る。
    pub(super) fn new(started: Instant, writer: W) -> Self {
        Self {
            started,
            writer: Mutex::new(writer),
        }
    }

    /// 完成した1行を記録し、フラッシュする。
    fn record(&self, direction: char, line: &[u8]) -> io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("I/O log lock poisoned"))?;
        // 時刻の取得もロック内で行い、記録順と時刻の順を一致させる。
        write!(
            writer,
            "{} {direction} ",
            self.started.elapsed().as_millis()
        )?;
        writer.write_all(line)?;
        writer.write_all(b"\n")?;
        writer.flush()
    }
}

/// 受信行を読み終えた直後に記録し、従来の末尾空白除去を施してチャネルへ送る。
pub(super) fn read_input<W: Write>(
    mut input: impl BufRead,
    sender: mpsc::Sender<io::Result<String>>,
    log: Option<&IoLog<W>>,
) {
    let mut line = String::new();
    loop {
        line.clear();
        let result = match input.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let record = if let Some(log) = log {
                    let received = match line.strip_suffix('\n') {
                        Some(body) => body.strip_suffix('\r').unwrap_or(body),
                        None => &line,
                    };
                    log.record('>', received.as_bytes())
                } else {
                    Ok(())
                };
                record.map(|()| line.trim_end().to_owned())
            }
            Err(error) => Err(error),
        };
        let failed = result.is_err();
        if sender.send(result).is_err() || failed {
            break;
        }
    }
}

/// 標準出力が受理したバイト列を改行で区切り、完成した行だけを記録する。
pub(super) struct LoggedOutput<W, L> {
    /// プロトコルの出力先。
    output: W,
    /// 入力スレッドと共有するログ。
    log: Arc<IoLog<L>>,
    /// 書き込み呼び出しをまたいで残る未完の行。
    pending: Vec<u8>,
}

impl<W: Write, L: Write> LoggedOutput<W, L> {
    /// 従来の出力先へログの記録を追加する。
    pub(super) fn new(output: W, log: Arc<IoLog<L>>) -> Self {
        Self {
            output,
            log,
            pending: Vec::new(),
        }
    }
}

impl<W: Write, L: Write> Write for LoggedOutput<W, L> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.output.write(bytes)?;
        for part in bytes[..written].split_inclusive(|byte| *byte == b'\n') {
            if let Some(line) = part.strip_suffix(b"\n") {
                self.pending.extend_from_slice(line);
                self.log.record('<', &self.pending)?;
                self.pending.clear();
            } else {
                self.pending.extend_from_slice(part);
            }
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 書き込みまたはフラッシュの失敗を再現するログの書き込み先。
    struct FailingLog {
        /// trueならフラッシュまで書き込みを受理する。
        fail_on_flush: bool,
    }

    impl Write for FailingLog {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail_on_flush {
                Ok(bytes.len())
            } else {
                Err(io::Error::other("log write failed"))
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("log flush failed"))
        }
    }

    /// debugging-tools.md「入出力のログの形式」: 入力側の記録失敗はチャネルのエラーとして伝える。
    #[test]
    fn input_propagates_log_write_and_flush_errors() {
        for fail_on_flush in [false, true] {
            let log = IoLog::new(Instant::now(), FailingLog { fail_on_flush });
            let (sender, receiver) = mpsc::channel();
            read_input(&b"usi\nisready\n"[..], sender, Some(&log));
            assert!(receiver.recv().unwrap().is_err());
            assert!(receiver.recv().is_err());
        }
    }

    /// debugging-tools.md「入出力のログの形式」: 出力側の記録失敗はWriteのエラーとして伝える。
    #[test]
    fn output_propagates_log_write_and_flush_errors() {
        for fail_on_flush in [false, true] {
            let log = Arc::new(IoLog::new(Instant::now(), FailingLog { fail_on_flush }));
            let mut output = LoggedOutput::new(Vec::new(), log);
            assert!(output.write_all(b"usiok\n").is_err());
            assert_eq!(output.output, b"usiok\n");
        }
    }

    /// debugging-tools.md「入出力のログの形式」: 分割書き込みと複数行を扱い、未完の行は記録しない。
    #[test]
    fn output_records_only_complete_lines_and_flushes_each() {
        /// フラッシュ時点のバイト列を保持する書き込み先。
        #[derive(Default)]
        struct Snapshots {
            bytes: Vec<u8>,
            flushed: Vec<Vec<u8>>,
        }
        impl Write for Snapshots {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushed.push(self.bytes.clone());
                Ok(())
            }
        }
        let log = Arc::new(IoLog::new(Instant::now(), Snapshots::default()));
        let mut output = LoggedOutput::new(Vec::new(), Arc::clone(&log));
        output.write_all(b"usi").unwrap();
        output.flush().unwrap();
        assert!(log.writer.lock().unwrap().bytes.is_empty());
        output.write_all(b"ok\nreadyok\nunfinished").unwrap();
        let writer = log.writer.lock().unwrap();
        assert_eq!(writer.flushed.len(), 2);
        assert!(writer.flushed[0].ends_with(b" < usiok\n"));
        assert!(writer.flushed[1].ends_with(b" < readyok\n"));
        assert_eq!(output.output, b"usiok\nreadyok\nunfinished");
    }

    /// debugging-tools.md「入出力のログの形式」: 出力先が受理した行だけを記録する。
    #[test]
    fn output_does_not_record_unwritten_bytes() {
        let log = Arc::new(IoLog::new(Instant::now(), Vec::new()));
        let mut storage = [0_u8; 6];
        let mut output = LoggedOutput::new(&mut storage[..], Arc::clone(&log));
        assert!(output.write_all(b"usiok\nreadyok\n").is_err());
        let writer = log.writer.lock().unwrap();
        let text = std::str::from_utf8(&writer).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with(" < usiok\n"));
    }
}

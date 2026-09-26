//! 応答期限を設けた行の受信。

use crate::harness::failure::EngineFailure;
use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// USI出力を条件一致、切断、または期限切れまで受信する。
pub(super) fn receive_until(
    lines: &Receiver<io::Result<String>>,
    timeout: Duration,
    predicate: impl FnMut(&str) -> bool,
) -> Result<String, EngineFailure> {
    receive_until_deadline(lines, Instant::now(), timeout, predicate)
}

/// 複数の応答を読む場合も、同じ起点から期限を測る。
pub(super) fn receive_until_deadline(
    lines: &Receiver<io::Result<String>>,
    start: Instant,
    timeout: Duration,
    mut predicate: impl FnMut(&str) -> bool,
) -> Result<String, EngineFailure> {
    loop {
        let remaining = timeout
            .checked_sub(start.elapsed())
            .ok_or(EngineFailure::Timeout)?;
        match lines.recv_timeout(remaining) {
            Ok(Ok(line)) if predicate(&line) => return Ok(line),
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(RecvTimeoutError::Disconnected) => {
                return Err(EngineFailure::Crash);
            }
            Err(RecvTimeoutError::Timeout) => return Err(EngineFailure::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    // D8-HARN-06(2)(3)(sprt.md異常時の裁定節): プロセス終了・パイプ切断は
    // クラッシュ、応答期限超過は応答タイムアウトとして分類する。
    #[test]
    fn response_channel_classifies_disconnect_and_timeout() {
        let (sender, lines) = mpsc::channel();
        drop(sender);
        assert_eq!(
            receive_until(&lines, Duration::from_secs(1), |_| false),
            Err(EngineFailure::Crash)
        );

        let (_sender, lines) = mpsc::channel();
        assert_eq!(
            receive_until(&lines, Duration::from_millis(1), |_| false),
            Err(EngineFailure::Timeout)
        );
    }

    // D8-HARN-11(match-harness.md USIセッション管理節): ハーネスは`bestmove`を
    // 受けて着手を適用する。それ以外の行(infoなど)は応答待ちで読み飛ばす。
    #[test]
    fn response_channel_waits_for_bestmove_ignoring_other_lines() {
        let (sender, lines) = mpsc::channel();
        sender
            .send(Ok("info depth 1 score cp 0".to_owned()))
            .expect("the receiver must be alive");
        sender
            .send(Ok("bestmove 1a1b".to_owned()))
            .expect("the receiver must be alive");
        assert_eq!(
            receive_until(&lines, Duration::from_secs(1), |line| {
                line.split_whitespace().next() == Some("bestmove")
            }),
            Ok("bestmove 1a1b".to_owned())
        );
    }
}

//! 応答期限を設けた行の受信。

use crate::harness::failure::EngineFailure;
use std::io;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// 条件を満たす行、切断、または期限切れまで受信する。
///
/// ハーネスが着手を待つときは`bestmove`の行まで読み、`info`など他の行は読み飛ばす。
/// `predicate`は読み飛ばす行の評価値なども観測できる。期限は受信処理全体に適用する。
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

    // docs/guides/sprt.md異常時の裁定節に従い、プロセス終了・パイプ切断は
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
            receive_until_deadline(&lines, Instant::now(), Duration::ZERO, |_| false),
            Err(EngineFailure::Timeout)
        );
    }

    // [`receive_until`]はbestmoveまで他の行を読み飛ばす。
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

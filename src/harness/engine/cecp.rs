//! CECPの送信文と応答の解釈。

use crate::Move;
use crate::harness::engine::channel::receive_until;
use crate::harness::engine::process::EngineProcess;
use crate::harness::engine::{EngineResponse, ThinkRequest, ThinkResult};
use crate::harness::failure::EngineFailure;
use crate::harness::limit::{SearchLimit, TimeControl};
use crate::notation::cecp;
use std::io;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// CECPエンジンへ割り当てる置換表容量の既定値。HaChuは`memory`受信前の`go`で
/// 異常終了するため明示し、256 MBはminaseの`USI_Hash`既定値に合わせる。
pub(super) const CECP_MEMORY_MB: u64 = 256;

/// 固定制限時にCECPエンジンへ通知する時計残量。時計残量0ではHaChuが
/// 反復深化を即座に打ち切り、HaChuは5倍した残量を32ビット整数で計算するため、
/// 十分大きくかつその範囲に収まる3,000,000センチ秒とする。
pub(in crate::harness) const CECP_FIXED_TIME_CS: u64 = 3_000_000;

impl EngineProcess {
    /// CECPの差分着手と時計を送り、`pong`までの応答を解釈する。
    pub(super) fn bestmove_cecp(
        &mut self,
        history: &[Move],
        request: &ThinkRequest,
    ) -> Result<ThinkResult, EngineFailure> {
        for &mv in &history[self.sent_moves..] {
            self.send(&format!("usermove {}", cecp::legs(mv).concat()))?;
        }
        self.sent_moves = history.len();
        self.send(&format!("time {}", request.own_cs))?;
        self.send(&format!("otim {}", request.opponent_cs))?;
        let start = Instant::now();
        self.send("go")?;
        let pong_number = history.len();
        self.send(&format!("ping {pong_number}"))?;
        let (response, elapsed) =
            receive_cecp_response(&self.lines, self.timeout, pong_number, start)?;
        if matches!(response, EngineResponse::Move(_)) {
            self.send("force")?;
            self.sent_moves += 1;
        }
        Ok(ThinkResult {
            response,
            elapsed,
            ponder: None,
            evaluation: None,
            stop_reason: None,
            completed_time_ms: None,
        })
    }
}

/// 時間制御をCECPの`level`コマンドへ変換する。
pub(super) fn cecp_level_text(time: TimeControl) -> String {
    let total_seconds = time.base_ms / 1_000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    let base = if seconds == 0 {
        minutes.to_string()
    } else {
        format!("{minutes}:{seconds:02}")
    };
    format!("level 0 {base} {}", time.increment_ms / 1_000)
}

/// 1手固定時間をCECPの`st`コマンドへ変換する。
/// HaChuでは`st`が固定時間モードを設定し、毎手の`time`が表す時間の0.4倍を目標、
/// 約0.98倍を強制中断の上限とするため、`time`にも1手分の時間を送る必要がある。
pub(super) fn cecp_fixed_time_text(time: TimeControl) -> String {
    format!("st {}", time.byoyomi_ms / 1_000)
}

/// CECPの着手・結果・拒否のいずれかを示す行かどうかを返す。
fn is_cecp_response_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("move ")
        || line.starts_with("Illegal move")
        || ["1-0", "0-1", "1/2-1/2", "resign"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
}

/// `pong`までのCECP応答行を着手または投了へ解釈する。
///
/// 応答行がない`pong`はプロトコル違反であり、クラッシュ相当に分類する。
fn interpret_cecp_response(lines: &[String]) -> Result<EngineResponse, EngineFailure> {
    if lines
        .iter()
        .any(|line| line.trim_start().starts_with("Illegal move"))
    {
        return Err(EngineFailure::RejectedMove);
    }

    let mut moves = lines
        .iter()
        .filter_map(|line| line.trim_start().strip_prefix("move ").map(str::trim));
    if let Some(first) = moves.next() {
        let mut text = first.to_owned();
        if text.ends_with(',')
            && let Some(second) = moves.next()
        {
            text.push_str(second);
        }
        return Ok(EngineResponse::Move(text));
    }

    if lines.iter().any(|line| {
        let line = line.trim_start();
        ["1-0", "0-1", "1/2-1/2", "resign"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
    }) {
        Ok(EngineResponse::Resigned)
    } else {
        Err(EngineFailure::Crash)
    }
}

/// CECP応答を最初の着手または結果から`pong`まで読み切る。
fn receive_cecp_response(
    lines: &Receiver<io::Result<String>>,
    timeout: Duration,
    pong_number: usize,
    start: Instant,
) -> Result<(EngineResponse, Duration), EngineFailure> {
    let expected_pong = format!("pong {pong_number}");
    let mut received = Vec::new();
    let mut response_elapsed = None;
    let first = receive_until(lines, timeout, |line| {
        received.push(line.to_owned());
        if is_cecp_response_line(line) {
            response_elapsed = Some(start.elapsed());
            true
        } else {
            line.trim() == expected_pong
        }
    })?;
    if first.trim() != expected_pong {
        receive_until(lines, timeout, |line| {
            received.push(line.to_owned());
            line.trim() == expected_pong
        })?;
    }
    let response = interpret_cecp_response(&received)?;
    let elapsed = response_elapsed.expect("a valid CECP response has a response line");
    Ok((response, elapsed))
}

/// 明示容量または既定容量を設定するCECPコマンドを返す。
pub(super) fn cecp_memory_text(hash_mb: Option<u64>) -> String {
    format!("memory {}", hash_mb.unwrap_or(CECP_MEMORY_MB))
}

/// CECPで表現できる思考制限かどうかを検証する。
pub(in crate::harness) fn validate_cecp_limit(limit: SearchLimit) -> io::Result<()> {
    match limit {
        SearchLimit::Fixed { nodes: Some(_), .. } => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CECP engine does not support a fixed node limit",
        )),
        SearchLimit::Fixed {
            depth: Some(_),
            nodes: None,
        } => Ok(()),
        SearchLimit::Fixed {
            depth: None,
            nodes: None,
        } => unreachable!("a validated fixed limit contains depth or nodes"),
        SearchLimit::Time(time) if time.byoyomi_ms > 0 => {
            if time.base_ms == 0 && time.increment_ms == 0 && time.byoyomi_ms % 1_000 == 0 {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "CECP engine supports byoyomi only as a fixed time per move: base 0, increment 0, whole seconds",
                ))
            }
        }
        SearchLimit::Time(time) if time.base_ms % 1_000 != 0 || time.increment_ms % 1_000 != 0 => {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "CECP time control requires base time and increment in whole seconds",
            ))
        }
        SearchLimit::Time(_) => Ok(()),
    }
}

/// 合法手集合から移動元の内部升番号が最小の正準じっとを選ぶ。
pub(in crate::harness) fn canonical_jitto(legal_moves: &[Move]) -> Option<Move> {
    legal_moves
        .iter()
        .copied()
        .filter(|mv| mv.mid.is_none() && mv.from == mv.to)
        .min_by_key(|mv| mv.from.raw_index())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Square;
    use crate::harness::limit::parse_search_limit;
    use crate::harness::player::{parse_player_spec, resolve_player};
    use std::sync::mpsc;

    // D8-HARN-20（match-harness.md「CECPセッション管理」）: `level`の持ち時間は
    // 60秒の倍数なら分だけ、それ以外は分:秒の2桁表記になり、加算は秒で送る。
    #[test]
    fn cecp_level_formats_minutes_seconds_and_increment() {
        assert_eq!(
            cecp_level_text(TimeControl {
                base_ms: 60_000,
                increment_ms: 1_000,
                byoyomi_ms: 0,
            }),
            "level 0 1 1"
        );
        assert_eq!(
            cecp_level_text(TimeControl {
                base_ms: 61_000,
                increment_ms: 2_000,
                byoyomi_ms: 0,
            }),
            "level 0 1:01 2"
        );
    }

    // 1手固定時間の仕様: 2000 msの秒読みを`st 2`として送る。
    #[test]
    fn cecp_fixed_time_formats_byoyomi_in_seconds() {
        assert_eq!(
            cecp_fixed_time_text(TimeControl {
                base_ms: 0,
                increment_ms: 0,
                byoyomi_ms: 2_000,
            }),
            "st 2"
        );
    }

    // CECPの`memory`コマンド: CECPには明示容量を送り、省略時だけ256 MBを使う。
    #[test]
    fn cecp_memory_command_uses_explicit_size_or_default() {
        for (hash_mb, expected) in [
            (None, "memory 256"),
            (Some(1), "memory 1"),
            (Some(512), "memory 512"),
        ] {
            let player = resolve_player(
                parse_player_spec("cecp:engine").unwrap(),
                parse_search_limit("depth=1").unwrap(),
                hash_mb,
                "R1",
                Vec::new(),
            )
            .unwrap();
            assert_eq!(cecp_memory_text(player.hash_mb), expected);
        }
    }

    // D8-HARN-20（match-harness.md「CECPセッション管理」、hachu.md第8節）:
    // `@@@@`は合法手中の正準じっとから移動元の内部升番号が最小の手へ一意に割り当てる。
    #[test]
    fn cecp_null_move_selects_the_lowest_origin_canonical_jitto() {
        let low = Move {
            from: Square::new(2, 3).unwrap(),
            mid: None,
            to: Square::new(2, 3).unwrap(),
            promote: false,
        };
        let high = Move {
            from: Square::new(9, 8).unwrap(),
            mid: None,
            to: Square::new(9, 8).unwrap(),
            promote: false,
        };
        let ordinary = Move {
            from: Square::new(0, 0).unwrap(),
            mid: None,
            to: Square::new(0, 1).unwrap(),
            promote: false,
        };
        assert_eq!(canonical_jitto(&[high, ordinary, low]), Some(low));
        assert_eq!(canonical_jitto(&[ordinary]), None);
    }

    // D8-HARN-06/20（match-harness.md「CECPセッション管理」「異常時裁定」）:
    // CECP応答はpongまで読み切り、分割レグを連結し、拒否を反則へ、結果行だけを投了へ
    // 分類する。着手後の結果行はHaChuの王駒捕獲出力なので着手を優先する。
    #[test]
    fn cecp_response_lines_are_interpreted_through_pong() {
        fn receive(lines_to_send: &[&str]) -> Result<EngineResponse, EngineFailure> {
            let (sender, lines) = mpsc::channel();
            for &line in lines_to_send {
                sender.send(Ok(line.to_owned())).unwrap();
            }
            sender.send(Ok("pong 12".to_owned())).unwrap();
            receive_cecp_response(&lines, Duration::from_secs(1), 12, Instant::now())
                .map(|(response, _)| response)
        }

        assert_eq!(
            receive(&["# debug", "move e7d8,", "move d8d7"]),
            Ok(EngineResponse::Move("e7d8,d8d7".to_owned()))
        );
        assert_eq!(
            receive(&["Illegal move (repetition): e7d8"]),
            Err(EngineFailure::RejectedMove)
        );
        assert_eq!(receive(&["0-1 {resign}"]), Ok(EngineResponse::Resigned));
        assert_eq!(
            receive(&["move c9i3+", "1-0 {royal capture}"]),
            Ok(EngineResponse::Move("c9i3+".to_owned()))
        );
    }

    // D8-HARN-20（match-harness.md「CECPセッション管理」）: 応答なしのpongは
    // プロトコル違反であり、クラッシュ相当に分類する。
    #[test]
    fn cecp_pong_without_a_move_or_result_is_a_crash() {
        let (sender, lines) = mpsc::channel();
        sender.send(Ok("pong 3".to_owned())).unwrap();
        assert_eq!(
            receive_cecp_response(&lines, Duration::from_secs(1), 3, Instant::now())
                .map(|(response, _)| response),
            Err(EngineFailure::Crash)
        );
    }
}

//! 現局面に対する1手の思考。

use crate::harness::clock::GameClocks;
use crate::harness::engine::channel::receive_until_deadline;
use crate::harness::engine::process::EngineProcess;
use crate::harness::engine::usi::{observe_usi_evaluation, observe_usi_search_data};
use crate::harness::engine::{EngineResponse, ThinkResult};
use crate::harness::failure::EngineFailure;
use crate::harness::limit::SearchLimit;
use crate::harness::player::Protocol;
use crate::{Color, Move};
use std::time::Instant;

impl EngineProcess {
    /// 現局面の応答を得る。外れた先読みの停止も同じ時計と期限に含める。
    pub(in crate::harness) fn bestmove(
        &mut self,
        usi_history: &[String],
        move_history: &[Move],
        clocks: &GameClocks,
        side: Color,
        limit: SearchLimit,
    ) -> Result<ThinkResult, EngineFailure> {
        if self.protocol == Protocol::Cecp {
            return self.bestmove_cecp(move_history, &clocks.think_request(side, limit));
        }
        let start;
        if let Some(predicted) = self.pondering.take() {
            start = Instant::now();
            if move_history.last() == Some(&predicted) {
                self.send("ponderhit")?;
            } else {
                self.send("stop")?;
                self.discard_bestmove(start)?;
                self.send_position(usi_history)?;
                let mut adjusted = *clocks;
                if let Some(clock) = adjusted.get_mut(side) {
                    clock.remaining = clock.remaining.saturating_sub(start.elapsed());
                }
                let request = adjusted.think_request(side, limit);
                self.send(&format!("go {}", request.go_text))?;
            }
        } else {
            self.send_position(usi_history)?;
            let request = clocks.think_request(side, limit);
            start = Instant::now();
            self.send(&format!("go {}", request.go_text))?;
        }
        self.receive_bestmove(start)
    }

    /// 初期局面からのUSI着手列を送る。
    pub(super) fn send_position(&mut self, history: &[String]) -> Result<(), EngineFailure> {
        if history.is_empty() {
            self.send("position startpos")
        } else {
            self.send(&format!("position startpos moves {}", history.join(" ")))
        }
    }

    /// 的中時には先読み中のinfoも同じ探索の情報として読む。
    fn receive_bestmove(&self, start: Instant) -> Result<ThinkResult, EngineFailure> {
        let mut evaluation = None;
        let mut stop_reason = None;
        let mut completed_time_ms = None;
        let mut observation_error = None;
        let line = receive_until_deadline(&self.lines, start, self.timeout, |line| {
            observe_usi_evaluation(&mut evaluation, line);
            if observation_error.is_none()
                && let Err(reason) =
                    observe_usi_search_data(&mut stop_reason, &mut completed_time_ms, line)
            {
                observation_error = Some(reason);
            }
            observation_error.is_some() || line.split_whitespace().next() == Some("bestmove")
        })?;
        if let Some(reason) = observation_error {
            return Err(reason);
        }
        let response = match line.split_whitespace().nth(1).unwrap_or_default() {
            "resign" => EngineResponse::Resigned,
            bestmove => EngineResponse::Move(bestmove.to_owned()),
        };
        let tokens = line.split_whitespace().collect::<Vec<_>>();
        let ponder = if tokens.get(2) == Some(&"ponder") {
            tokens.get(3).map(|text| (*text).to_owned())
        } else {
            None
        };
        let elapsed = start.elapsed();
        if elapsed > self.timeout {
            return Err(EngineFailure::Timeout);
        }
        Ok(ThinkResult {
            response,
            ponder,
            elapsed,
            evaluation,
            stop_reason,
            completed_time_ms,
        })
    }
}

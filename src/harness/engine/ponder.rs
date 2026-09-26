//! 相手番の先読みと停止。

use crate::harness::engine::ThinkRequest;
use crate::harness::engine::channel::receive_until_deadline;
use crate::harness::engine::process::EngineProcess;
use crate::harness::failure::EngineFailure;
use crate::harness::referee::ponder_move;
use crate::notation::usi;
use crate::{Game, MoveGenerator};
use std::time::Instant;

impl EngineProcess {
    /// 合法で終局しない予想手だけを使い、次の相手番の間に先読みする。
    pub(in crate::harness) fn start_ponder(
        &mut self,
        game: &Game,
        history: &[String],
        prediction: Option<&str>,
        request: &ThinkRequest,
    ) {
        let Some(Ok((predicted, true))) = prediction.map(|text| ponder_move(game, text)) else {
            return;
        };
        let mut history = history.to_vec();
        history.push(
            usi::text(
                game.position(),
                predicted,
                &MoveGenerator::new(game.rules().moves),
            )
            .expect("a legal prediction must be renderable"),
        );
        if self.send_position(&history).is_ok()
            && self.send(&format!("go ponder {}", request.go_text)).is_ok()
        {
            self.pondering = Some(predicted);
        }
    }

    /// 先読みの出力を探索情報も含めて捨てる。
    pub(super) fn discard_bestmove(&self, start: Instant) -> Result<(), EngineFailure> {
        receive_until_deadline(&self.lines, start, self.timeout, |line| {
            line.split_whitespace().next() == Some("bestmove")
        })
        .map(|_| ())
    }

    /// 終局時の停止は結果を変更せず、資源を読む前に同期する。
    pub(in crate::harness) fn stop_ponder(&mut self) {
        if self.pondering.take().is_some() {
            let start = Instant::now();
            if self.send("stop").is_ok() {
                let _ = self.discard_bestmove(start);
            }
        }
    }
}

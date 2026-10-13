//! 相手番の先読みの開始、的中、外れ、および終局時の停止。
//!
//! 返された予想手が審判層で合法かつ終局しない場合だけ、着手列に予想手を加えて`go ponder`を送る。
//! 送信失敗はこの場で勝敗へ反映せず、次の手番の通信で通常の異常として扱う。
//! 的中したら`ponderhit`を送り、先読み中の探索情報を同じ探索の情報として読む。
//! 外れたら`stop`の応答を捨て、実際の着手列と更新した時計で通常の`go`を送る。
//! 外れの停止から通常探索の応答までを1つの時計と応答期限で測る。
//! 終局時の処理は[`EngineProcess::stop_ponder`]に従う。

use crate::harness::engine::ThinkRequest;
use crate::harness::engine::channel::receive_until_deadline;
use crate::harness::engine::process::EngineProcess;
use crate::harness::failure::EngineFailure;
use crate::harness::referee::ponder_move;
use minase_core::notation::usi;
use minase_core::{Game, MoveGenerator};
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

    /// 終局時に先読みを停止し、結果を変えずに資源計測の前に同期する。
    ///
    /// `stop`後の`bestmove`と探索情報を捨てる。
    /// 送信失敗や応答期限切れは、すでに確定した終局結果や異常件数へ反映しない。
    pub(in crate::harness) fn stop_ponder(&mut self) {
        if self.pondering.take().is_some() {
            let start = Instant::now();
            if self.send("stop").is_ok() {
                let _ = self.discard_bestmove(start);
            }
        }
    }
}

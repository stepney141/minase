//! 外部通信方式と対局管理を分離するプロトコル層。

use std::io::{self, BufRead, Write};

pub mod cecp;
pub mod engine;
pub mod usi;

pub use cecp::CecpProtocol;
pub use engine::{Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason};
pub use usi::UsiProtocol;

/// 同期入出力でプロトコルセッションを処理する。
///
/// 各入力行を受信順に完了させる。reader threadと並行して探索イベントを
/// 扱う非同期実行は、[`UsiProtocol::run_channel`]が別に提供する。
pub trait Protocol {
    /// 入力の終端または終了コマンドまでセッションを処理する。
    fn run(
        &mut self,
        engine: &mut Engine,
        input: &mut dyn BufRead,
        output: &mut dyn Write,
    ) -> io::Result<()>;
}

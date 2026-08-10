//! 外部通信方式と対局管理を分離するプロトコル層。

use std::io::{self, BufRead, Write};

pub mod engine;
pub mod usi;

pub use engine::{Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason};
pub use usi::UsiProtocol;

/// 同期入出力でプロトコルセッションを処理する。
///
/// 探索機能がない段階では、スレッドとチャネルを設けず、各入力行を受信順に
/// 完了させる。探索導入時には`stop`を含む境界を改めて設計する。
pub trait Protocol {
    fn run(
        &mut self,
        engine: &mut Engine,
        input: &mut dyn BufRead,
        output: &mut dyn Write,
    ) -> io::Result<()>;
}

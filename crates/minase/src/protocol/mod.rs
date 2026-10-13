//! 通信方式に依存しない対局状態と、USIおよびCECPの入出力を分担する層。
//!
//! [`Engine`]は[`minase_core::Game`]、現局の規則、および次局の規則を保持する状態機械である。
//! アダプターは入力行の解析と出力文字列の生成を担い、指し手と局面の表記変換は
//! [`minase_core::notation`]へ委ねる。依存はプロトコルから表記の層への一方向である。
//! 探索の起動、停止、および結果受領は[`EngineCommand`]に入れず、各アダプターが[`crate::search`]を直接呼ぶ。
//! 理由は`docs/adr/call-search-from-protocol-modules.md`にある。
//!
//! 拒否されたコマンドが対局状態を変えない厳格な境界を[`Engine`]に置く。
//! 未知のオプション名の無視など、通信上の寛容な扱いはアダプター内に閉じる。

use std::io::{self, BufRead, Write};

pub mod cecp;
pub mod engine;
pub mod usi;

pub use cecp::CecpProtocol;
pub use engine::{Engine, EngineCommand, EngineLifecycle, EngineReply, RejectReason};
pub use usi::UsiProtocol;

/// 台本テスト用の逐次経路でプロトコルセッションを処理する。
///
/// [`Self::run`]は各入力行を受信順に処理する。
/// 実バイナリは標準入力の専用スレッドと[`UsiProtocol::run_channel`]または
/// [`CecpProtocol::run_channel`]を使い、探索中も入力を受ける。
/// 台本テストはこの並行処理を覆わないため、入力スレッドや`run_channel`を変えた場合は、
/// 実バイナリで時計つき`go`と`stop`、`go infinite`と`stop`、探索中の`quit`を確かめる。
/// 理由は`docs/adr/call-search-from-protocol-modules.md`にある。
pub trait Protocol {
    /// 入力の終端または終了コマンドまでセッションを処理する。
    fn run(
        &mut self,
        engine: &mut Engine,
        input: &mut dyn BufRead,
        output: &mut dyn Write,
    ) -> io::Result<()>;
}

//! `minase match run`と`minase spsa`が共有する対局実行部。
//!
//! 局ごとにエンジンを別プロセスとして起動し、返答を着手の提案として扱う。
//! 合法性と終局の判定は審判層の[`minase_core::Game`]が担う。
//! 比較対象の機能の内容は知らず、機能比較のスイッチを持たない。
//! 理由は`docs/adr/compare-commits-as-separate-engine-processes.md`と
//! `docs/adr/run-spsa-as-a-separate-command.md`にある。
//!
//! 実行ファイル向けの内部層として`#[doc(hidden)]`で公開し、安定した公開APIとは扱わない。
//! 子モジュールの項目は本モジュールの`pub use`で`minase::harness::X`という平らな名前へ公開する。
//! エンジンのプロセスを駆動する`engine`モジュール自体は非公開である。

mod clock;
mod commit;
mod engine;
mod environment;
mod failure;
mod game;
mod limit;
mod opening;
mod pair;
mod player;
mod ponder_stats;
mod records;
mod referee;
mod resume;
mod storage;

pub use clock::*;
pub use commit::*;
pub use engine::*;
pub use environment::*;
pub use failure::*;
pub use game::*;
pub use limit::*;
pub use opening::*;
pub use pair::*;
pub use player::*;
pub use ponder_stats::*;
pub use records::*;
pub use referee::*;
pub use resume::*;
pub use storage::*;

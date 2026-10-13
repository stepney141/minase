//! 中将棋の対局エンジンMinase。
//!
//! 盤、駒、合法手、裁定、および表記は[`minase_core`]が提供する。
//! 探索と評価の依存は[`search`]から[`eval`]、さらに`minase_core`への一方向である。
//! [`training`]はMNSD、MNRS、および来歴JSONの交換形式を持ち、プロジェクト内では`minase_core`だけに依存する。
//! `search`と`eval`は`training`を参照しない。
//! `datagen`はデータ生成と棋譜取り込みの共有処理を持ち、`training`と`minase_core`に依存し、探索と評価には依存しない。
//!
//! `harness`、`datagen`、`stats`は実行ファイル向けの内部層として`#[doc(hidden)]`で公開する。
//! 規則ライブラリの型は`minase_core::`から参照し、本crateから再公開しない。
//! 理由は`docs/adr/do-not-re-export-moved-items-at-old-paths.md`にある。

#[doc(hidden)]
pub mod datagen;
pub mod eval;
#[doc(hidden)]
pub mod harness;
pub mod protocol;
pub mod search;
#[doc(hidden)]
pub mod stats;
pub mod training;

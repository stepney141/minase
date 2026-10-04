//! 中将棋の対局エンジンMinase。
//!
//! 盤・駒・合法手・裁定と表記は`minase-core`が提供する。
//! 本crateはプロトコル、探索、評価、および学習データの生成を担う。

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

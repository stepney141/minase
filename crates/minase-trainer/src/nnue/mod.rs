//! NNUEの特徴構築、モデル、参照計算、CUDA処理と学習パイプライン。

pub mod data;
pub mod export;
pub mod fit_linear;
pub mod gpu;
pub mod handcrafted;
// 学習ループへ接続するまでの一時的な許可。
#[allow(dead_code)]
pub mod layerstack;
pub mod model;
pub mod options;
pub mod reference;
pub mod schedule;
pub mod train;

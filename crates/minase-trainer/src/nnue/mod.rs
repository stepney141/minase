//! NNUEの特徴構築、モデル、参照計算、CUDA処理と学習パイプライン。

pub mod data;
pub mod gpu;
pub mod handcrafted;
pub mod layerstack;
pub mod options;
pub mod reference;
pub mod schedule;
pub mod train;

pub mod simple;

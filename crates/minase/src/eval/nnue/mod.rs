//! tatara形式のLayerStackとSimpleの復号、差分更新、および整数評価。

mod file;
pub mod input;
pub mod network;
mod stats;
pub(crate) mod transformer;

pub use file::{NetworkFile, NetworkFileError, read_network_file};
pub(crate) use network::SearchEvaluator;
#[cfg(feature = "test-util")]
pub use network::test_util;
pub use network::{
    FormatError, LayerStackNetwork, LayerStackParameters, Network, SimpleNetwork, SimpleParameters,
};
pub use stats::AccumulatorStats;

/// 静的評価の絶対値の上限。単位はセンチポーンである。
/// [`crate::search::MATE_THRESHOLD`]未満に保ち、詰み帯と区別する。
pub(crate) const EVALUATION_LIMIT: i32 = 28_999;

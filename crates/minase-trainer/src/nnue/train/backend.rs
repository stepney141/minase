use super::{Progress, Result};
use crate::nnue::{
    data::{BatchSlot, FeatureSet},
    gpu::{self, HostBuffer, HostRing},
};
use minase_core::{Color, Position};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

/// 学習ループが集計する1更新の結果。損失は局面方向の和。
pub(crate) struct StepMetrics {
    pub(crate) loss_sum: f64,
    pub(crate) positions: usize,
    pub(crate) compute_seconds: f64,
}

/// 検証の加算可能な統計。構造固有の各欄もバッチ間で加算する。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Validation {
    pub(crate) loss_sum: f64,
    pub(crate) positions: usize,
    pub(crate) sums: BTreeMap<String, f64>,
}
impl Validation {
    pub(crate) fn loss(&self) -> f64 {
        self.loss_sum / self.positions as f64
    }
    pub(crate) fn append(&mut self, next: Self) {
        self.loss_sum += next.loss_sum;
        self.positions += next.positions;
        for (key, value) in next.sums {
            *self.sums.entry(key).or_insert(0.0) += value;
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.positions == 0
            || !self.loss_sum.is_finite()
            || self.loss_sum < 0.0
            || self.sums.values().any(|v| !v.is_finite())
        {
            return Err(super::Error::Invalid("invalid validation statistics").into());
        }
        Ok(())
    }
}

/// 供給スレッドが循環利用する面。構造固有の配列を持たない。
pub(crate) enum Buffer {
    Cuda(HostBuffer),
    Cpu(Option<BatchSlot>),
}
impl Buffer {
    pub(crate) fn fill(&mut self, batch: BatchSlot) -> gpu::Result<()> {
        match self {
            Self::Cuda(host) => host.fill(&batch),
            Self::Cpu(slot) => {
                *slot = Some(batch);
                Ok(())
            }
        }
    }
}
pub(crate) enum Queue {
    Cuda(Box<HostRing>),
    Cpu,
}

/// 構造ごとの学習状態を隠し、更新と副作用だけを学習ループへ公開する。
/// 初期化と再開は実装側で済ませてから渡す。未回収の結果は全境界でflushする。
pub(crate) trait TrainerBackend {
    fn name(&self) -> &str;
    fn feature_set(&self) -> FeatureSet;
    fn step(&self) -> u64;
    fn memory(&self) -> Result<Option<(usize, usize)>>;
    fn queue(&self, capacity: usize) -> Result<(Queue, [Buffer; 3])>;
    /// 今回の更新を投入し、あれば前の更新の結果を回収する。
    fn train_step(
        &mut self,
        queue: &mut Queue,
        buffer: &mut Buffer,
        targets: &[f32],
        multiplier: f64,
    ) -> Result<Option<StepMetrics>>;
    fn flush_pending_loss(&mut self) -> Result<Option<StepMetrics>>;
    fn validate_step(&mut self, batch: &BatchSlot, targets: &[f32]) -> Result<Validation>;
    /// 構造固有の統計をJSONへ変換する。検証しない境界でも重みの統計を返す。
    fn statistics(&mut self, validation: Option<&Validation>) -> Result<Value>;
    fn save_checkpoint(&mut self, path: &Path, progress: &Progress) -> Result<()>;
    fn export(&mut self, path: &Path, samples: &[(Position, Color)]) -> Result<Value>;
}

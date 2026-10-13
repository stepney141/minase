//! 学習、照合、計測で共有する係数と実行デバイス。

use crate::nn::AdamWGroup;
use crate::nnue::{
    model::{Error, Result},
    reference::AdamWConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 明示的に選択する実行デバイス。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    /// CUDAを使用する。使用できなければエラーにする。
    #[default]
    Cuda,
    /// 標準ライブラリのスレッドでCPU参照計算を実行する。
    Cpu,
}

/// 学習、照合、計測で共用する最適化と教師の係数。
#[derive(Clone, Copy, Debug, PartialEq, clap::Args, Serialize)]
pub struct TrainingOptions {
    /// 線形経路の学習率を正の有限値で指定する。
    #[arg(long, default_value_t = 0.03)]
    pub lr_linear: f64,
    /// 線形経路の重み減衰を非負の有限値で指定する。
    #[arg(long, default_value_t = 0.0)]
    pub weight_decay_linear: f64,
    /// 第1層と仮想行の学習率を正の有限値で指定する。
    #[arg(long, default_value_t = 1e-3)]
    pub lr_ft: f64,
    /// 第1層と仮想行の重み減衰を非負の有限値で指定する。
    #[arg(long, default_value_t = 1e-4)]
    pub weight_decay_ft: f64,
    /// 出力部の学習率を正の有限値で指定する。
    #[arg(long, default_value_t = 1e-3)]
    pub lr_head: f64,
    /// 出力部の重み減衰を非負の有限値で指定する。
    #[arg(long, default_value_t = 1e-4)]
    pub weight_decay_head: f64,
    /// 駒除去差分の補助損失係数を非負の有限値で指定する。
    #[arg(long, default_value_t = 1000.0)]
    pub removal_penalty: f64,
    /// 自己対局の来歴のλを0以上1以下で上書きし、人間対局には適用しない。
    /// 既定値1.0は探索値だけを教師とする。
    /// PST、FM、同時学習は省略時に来歴のλを使うが、NNUEはこの既定値で上書きする。
    /// 理由は`docs/adr/train-the-nnue-on-search-value-lookahead-teachers.md`にある。
    #[arg(long, default_value_t = 1.0)]
    pub lambda_override: f64,
}

impl Default for TrainingOptions {
    fn default() -> Self {
        let config = AdamWConfig::default();
        Self {
            lr_linear: config.linear.lr,
            weight_decay_linear: config.linear.weight_decay,
            lr_ft: config.ft.lr,
            weight_decay_ft: config.ft.weight_decay,
            lr_head: config.head.lr,
            weight_decay_head: config.head.weight_decay,
            removal_penalty: 1000.0,
            lambda_override: 1.0,
        }
    }
}

impl TrainingOptions {
    /// 全係数の有限性と範囲を検査する。
    pub fn validate(&self) -> Result<()> {
        self.optimizer().validate()?;
        if !self.removal_penalty.is_finite() || self.removal_penalty < 0.0 {
            return Err(Error::Invalid(
                "--removal-penalty must be finite and nonnegative",
            ));
        }
        if !self.lambda_override.is_finite() || !(0.0..=1.0).contains(&self.lambda_override) {
            return Err(Error::Invalid(
                "--lambda-override must be finite and in 0..1",
            ));
        }
        Ok(())
    }

    /// 線形、第1層、出力部の3群へ係数を割り当てる。
    pub fn optimizer(&self) -> AdamWConfig {
        AdamWConfig {
            linear: AdamWGroup {
                lr: self.lr_linear,
                weight_decay: self.weight_decay_linear,
            },
            ft: AdamWGroup {
                lr: self.lr_ft,
                weight_decay: self.weight_decay_ft,
            },
            head: AdamWGroup {
                lr: self.lr_head,
                weight_decay: self.weight_decay_head,
            },
        }
    }
}

/// 指定された並列度、または物理コア数−2を使う。最小値は1。
pub fn workers(threads: Option<usize>) -> crate::nnue::train::Result<usize> {
    if let Some(threads) = threads {
        return if threads == 0 {
            Err(crate::nnue::train::Error::Invalid("--threads must be positive").into())
        } else {
            Ok(threads)
        };
    }
    physical_workers(&std::fs::read_to_string("/proc/cpuinfo")?)
}

pub(crate) fn physical_workers(cpuinfo: &str) -> crate::nnue::train::Result<usize> {
    let mut cores = HashSet::new();
    for processor in cpuinfo.split("\n\n").filter(|s| !s.trim().is_empty()) {
        let field = |key: &str| {
            processor.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.trim() == key).then_some(value.trim())
            })
        };
        let package = field("physical id").ok_or(crate::nnue::train::Error::Invalid(
            "CPU topology has no physical id; set --threads explicitly",
        ))?;
        let core = field("core id").ok_or(crate::nnue::train::Error::Invalid(
            "CPU topology has no core id; set --threads explicitly",
        ))?;
        cores.insert((package, core));
    }
    if cores.is_empty() {
        return Err(crate::nnue::train::Error::Invalid(
            "CPU topology is empty; set --threads explicitly",
        )
        .into());
    }
    Ok(cores.len().saturating_sub(2).max(1))
}

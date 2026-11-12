//! 推論器と起動設定を用意する試験専用の補助。

use super::{Config, Engine};
use crate::nn::test_util::UniformEvaluator;
use crate::nn::{Evaluator, Network};
use crate::search::mcts::test_support::{CAPACITY, SCALE};
use std::path::PathBuf;

impl Engine {
    /// ONNX Runtimeを使わない試験でだけ推論器を注入する。
    pub(crate) fn with_evaluator(
        evaluator: Option<Box<dyn Evaluator + Send>>,
        config: Config,
    ) -> Self {
        let mut engine = Self::new(config);
        engine
            .set_option(
                "NnModel",
                crate::search::SearchOptionValue::String("test.onnx".to_owned()),
            )
            .unwrap();
        engine.network = Some(Network::from_bytes(Vec::new()));
        engine.evaluator = evaluator;
        engine
    }

    /// 起動設定の試験用に、推論器を準備しキャッシュ先を空にする。
    pub(crate) fn ready_for_test() -> Self {
        let mut engine = Self::with_evaluator(
            Some(Box::new(UniformEvaluator)),
            Config {
                capacity_bytes: CAPACITY,
                ..Config::new(SCALE)
            },
        );
        engine.cache_dir = PathBuf::new();
        engine.default_cache_dir = PathBuf::new();
        engine
    }

    /// 推論器を保持したまま、試験で比較するキャッシュ先を設定する。
    pub(crate) fn set_cache_dir_for_test(&mut self, cache_dir: PathBuf) {
        self.cache_dir = cache_dir;
    }

    /// 推論器の準備が必要な状態を作る。
    pub(crate) fn clear_evaluator_for_test(&mut self) {
        self.evaluator = None;
    }
}

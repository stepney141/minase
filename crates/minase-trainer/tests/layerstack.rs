//! LayerStackの教師混合式と既存の入力バッチを、新しいCPU実装の入口で照合する。
mod common;
use common::Fixture;
use minase_core::{Color, Position};
use minase_trainer::nnue::{
    data::{BatchSlot, FeatureSet, build_targets},
    layerstack::{
        AdamW, Features, OptimizerConfig, Parameter, Parameters, Scale, forward, train_batch,
    },
};

#[test]
fn cpu_batch_uses_existing_teacher_mixture_and_scale_ratio() {
    let scale = Scale::new(1072.65).unwrap();
    let fixture = Fixture::new(&[Position::initial(); 1]);
    let batch = fixture.batch(FeatureSet::A, &[0]);
    let mut weights = Parameters::zeros();
    weights[Parameter::L3Bias][0] = 0.25;
    let mut optimizer = AdamW::new(OptimizerConfig::default()).unwrap();
    let lambda = 0.7;
    let target = build_targets(&batch, &[1072.65], &[lambda]).unwrap()[0];
    // Fixtureの探索値は120、結果は勝ち。f32への丸めまで共通の教師構築と一致させる。
    let expected_target = (0.7 / (1.0 + (-120.0_f64 / 1072.65).exp()) + 0.3) as f32;
    assert_eq!(target, expected_target);
    let probability = 1.0 / (1.0 + (-1016.0_f64 * 0.25 / 1072.65).exp());
    let expected_loss = (probability - f64::from(expected_target)).powi(2);
    let actual = train_batch(&mut weights, &mut optimizer, scale, &batch, lambda, 0.5).unwrap();
    assert!((actual - expected_loss).abs() < 1e-15);
    assert_eq!(optimizer.step, 1);
    assert!(weights[Parameter::L3Bias][0] > 0.25);
    let checkpoint = minase_trainer::nnue::layerstack::Checkpoint {
        progress: minase_trainer::nnue::train::Progress {
            completed_steps: optimizer.step,
            ..progress()
        },
        parameters: weights,
        optimizer,
        settings: minase_trainer::nnue::layerstack::Settings {
            scale,
            rules: minase_core::rules::Rules::JCSA,
            lambda,
            seed: 0,
            optimizer: OptimizerConfig::default(),
        },
    };
    let path = fixture.root.join("layerstack.npz");
    checkpoint.write(&path).unwrap();
    let restored = minase_trainer::nnue::layerstack::Checkpoint::read(path).unwrap();
    assert_eq!(restored, checkpoint);
    let kt_only = build_targets(&batch, &[1072.65], &[1.0]).unwrap()[0];
    assert_eq!(kt_only, (1.0 / (1.0 + (-120.0_f64 / 1072.65).exp())) as f32);
}

#[test]
fn packed_a_batches_match_position_features() {
    let positions = minase_core::test_util::bench_positions();
    let weights = Parameters::initialize(12, Scale::new(679.03).unwrap());
    for side in Color::ALL {
        let batch = BatchSlot::from_evaluation_positions(FeatureSet::A, &positions, side).unwrap();
        let features = Features::from_batch(&batch).unwrap();
        assert_eq!(features.len(), positions.len());
        for (position, packed) in positions.iter().zip(features) {
            assert_eq!(
                forward(&weights, &packed).unwrap().output,
                forward(&weights, &Features::from_position(position, side))
                    .unwrap()
                    .output
            );
        }
    }
}

fn progress() -> minase_trainer::nnue::train::Progress {
    minase_trainer::nnue::train::Progress {
        completed_superbatches: 0,
        completed_steps: 0,
        elapsed_seconds: 0.0,
        config: minase_trainer::nn::checkpoint::Configuration {
            settings: serde_json::json!({}),
            inputs: serde_json::json!({}),
            preprocessing: serde_json::json!({}),
        },
    }
}

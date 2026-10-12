//! カーネル9の期待値は、BCEと統計量の定義から定める。
mod common;
use common::*;
use minase::training::records::{Outcome, Record};
use minase_trainer::{
    data::{BatchSlot, InputFeatureSet},
    model::Parameter,
    reference::{self, Forward},
};

#[test]
fn validation_excludes_draws_and_counts_activation_endpoints() {
    let positions = positions();
    let records: Vec<_> = positions
        .iter()
        .zip([Outcome::Loss, Outcome::Draw, Outcome::Win])
        .map(|(p, outcome)| Record::from_position(p, 0, outcome, 1, 0))
        .collect();
    for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
        let batch = BatchSlot::from_records(set, &records, &[0.0; 3], &[0; 3]).unwrap();
        let m = model(set, 2);
        let f = Forward {
            acc: vec![-0.1, 0.0, 1.0, 1.1, 0.2, 0.4, 0.6, 0.8, -1.0, 0.0, 0.9, 1.0],
            lin: vec![[0.0; 2]; 3],
        };
        let stats = reference::validation_forward(&m, &batch, &f, &[0.5; 3]).unwrap();
        assert_eq!((stats.correct, stats.decisive), (1, 2));
        assert_eq!(stats.sign_accuracy(), Some(0.5));
        assert!((stats.loss() - std::f64::consts::LN_2).abs() < 1e-7);
        assert_eq!((stats.lower, stats.upper, stats.activations), (4, 3, 12));
        assert_eq!(stats.saturation(), [1.0 / 3.0, 0.25]);
        assert_eq!(stats.nonlinear_std_cp(), 0.0);
    }
}

#[test]
fn validation_reports_population_standard_deviation_in_centipawns() {
    let position = &positions()[0];
    let records = [0, 1].map(|i| Record::from_position(position, 0, Outcome::Draw, 1, i));
    let batch =
        BatchSlot::from_records(InputFeatureSet::HalfKpHmB8, &records, &[0.0; 2], &[0; 2]).unwrap();
    let mut m = model(InputFeatureSet::HalfKpHmB8, 2);
    m.parameter_mut(Parameter::OutputWeights)[0] = 1.0;
    let f = Forward {
        acc: vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        lin: vec![[0.0; 2]; 2],
    };
    let stats = reference::validation_forward(&m, &batch, &f, &[0.5; 2]).unwrap();
    assert_eq!(stats.sign_accuracy(), None);
    assert_eq!(stats.nonlinear_sum, 1000.0);
    assert_eq!(stats.nonlinear_square_sum, 1_000_000.0);
    assert_eq!(stats.nonlinear_std_cp(), 500.0);
    assert!(reference::validation_forward(&m, &batch, &f, &[0.5]).is_err());
    assert!(reference::validation_forward(&m, &batch, &f, &[f32::NAN, 0.5]).is_err());
}

#[test]
fn record_batches_reject_mismatched_teachers_and_nonfinite_scores() {
    let record = Record::from_position(&positions()[0], 0, Outcome::Draw, 1, 0);
    assert!(BatchSlot::from_records(InputFeatureSet::P, &[], &[], &[]).is_err());
    assert!(
        BatchSlot::from_records(InputFeatureSet::P, std::slice::from_ref(&record), &[], &[0])
            .is_err()
    );
    assert!(BatchSlot::from_records(InputFeatureSet::P, &[record], &[f32::NAN], &[0]).is_err());
}

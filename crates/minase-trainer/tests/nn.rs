//! 汎用部品の契約を、数式、NumPyの出力、順序と保存の性質で検証する。
use minase_trainer::nn::{
    self, AdamCoefficients, AdamWGroup, Error, Result,
    npz::{self, Array},
};
use std::sync::{
    Barrier,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn adamw_matches_element_arithmetic_and_leaves_selection_to_the_caller() {
    // 46e03baのCoefficientsを同じ入力で3更新した重みと両モーメントのビット列。
    // 共通化後も比較先が同じ実装にならないよう、履歴から得た固定値を使う。
    let expected: [[[u32; 4]; 3]; 6] =
        serde_json::from_str(include_str!("fixtures/adam-46e03ba.json")).unwrap();
    for (decay_index, decay) in [0.0, 0.2].into_iter().enumerate() {
        let group = AdamWGroup {
            lr: 0.03,
            weight_decay: decay,
        };
        let mut actual = [4095.875_f32, -0.0, -4096.0, 3.125];
        let mut m = [0.0; 4];
        let mut v = [0.0; 4];
        for (step, gradient) in [[-0.7, 0.0, 0.31, 2.0], [0.0; 4], [0.9, -1.1, 0.0, -7.3]]
            .into_iter()
            .enumerate()
        {
            let step = step as u64 + 1;
            let coefficients = AdamCoefficients::new(step, group).unwrap();
            for i in 0..4 {
                coefficients.update(&mut actual[i], gradient[i], &mut m[i], &mut v[i]);
            }
            let [weights, first_moments, second_moments] =
                expected[decay_index * 3 + step as usize - 1];
            assert_eq!(actual.map(f32::to_bits), weights);
            assert_eq!(m.map(f32::to_bits), first_moments);
            assert_eq!(v.map(f32::to_bits), second_moments);
        }
        if decay == 0.0 {
            assert!(actual[0] > 4095.875);
        }
    }
    assert!(
        AdamCoefficients::new(
            0,
            AdamWGroup {
                lr: 0.1,
                weight_decay: 0.0
            }
        )
        .is_err()
    );
    for (lr, weight_decay) in [
        (0.0, 0.0),
        (f64::NAN, 0.0),
        (0.1, -1.0),
        (0.1, f64::INFINITY),
    ] {
        assert!(AdamCoefficients::new(1, AdamWGroup { lr, weight_decay }).is_err());
    }
}

#[test]
fn bce_and_its_gradient_match_probability_loss_and_extreme_limits() {
    for target in [0.0, 0.3, 1.0] {
        assert_eq!(nn::bce_with_logits(0.0, target), 2.0_f64.ln());
        assert_eq!(nn::bce_with_logits_gradient(0.0, target), 0.5 - target);
        for logit in [-9.0, -0.25, 0.5, 8.0] {
            let probability = 1.0 / (1.0 + f64::exp(-logit));
            let loss = -target * probability.ln() - (1.0 - target) * (1.0 - probability).ln();
            assert!((nn::bce_with_logits(logit, target) - loss).abs() < 1e-12);
            let h = 1e-5;
            let derivative = (nn::bce_with_logits(logit + h, target)
                - nn::bce_with_logits(logit - h, target))
                / (2.0 * h);
            assert!((nn::bce_with_logits_gradient(logit, target) - derivative).abs() < 1e-9);
        }
    }
    assert_eq!(nn::bce_with_logits(1000.0, 0.0), 1000.0);
    assert_eq!(nn::bce_with_logits(-1000.0, 1.0), 1000.0);
    assert_eq!(nn::bce_with_logits_gradient(1000.0, 0.0), 1.0);
    assert_eq!(nn::bce_with_logits_gradient(-1000.0, 1.0), -1.0);
}

#[test]
fn epoch_zero_ties_strict_improvements_and_patience_boundaries() {
    let mut best = None;
    assert!(!nn::update_best(&mut best, 0, 0.1).unwrap());
    assert_eq!(best, None);
    assert!(!nn::patience_reached(best, 100, 3));
    assert!(nn::update_best(&mut best, 1, 0.5).unwrap());
    assert!(!nn::update_best(&mut best, 2, 0.5).unwrap());
    assert!(!nn::patience_reached(best, 2, 2));
    assert!(nn::patience_reached(best, 3, 2));
    let smaller = f64::from_bits(0.5_f64.to_bits() - 1);
    assert!(nn::update_best(&mut best, 3, smaller).unwrap());
    assert_eq!(best.unwrap().epoch, 3);
    assert!(!nn::patience_reached(best, 3, 2));
    for loss in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(nn::update_best(&mut best, 4, loss).is_err());
    }
    assert_eq!(best.unwrap().epoch, 3);
}

#[test]
fn ordered_map_returns_input_order_and_joins_every_worker_on_failure() {
    let barrier = Barrier::new(4);
    let output: Result<_> = nn::parallel::map([5, 2, 9, 1], |item| {
        barrier.wait();
        Ok(item * item)
    });
    assert_eq!(output.unwrap(), [25, 4, 81, 1]);
    for panic in [false, true] {
        let completed = AtomicUsize::new(0);
        let barrier = Barrier::new(4);
        let output: Result<Vec<usize>> = nn::parallel::map(0..4, |item| {
            barrier.wait();
            if item == 0 {
                assert!(!panic, "injected worker panic");
                return Err(Error::Invalid("injected worker error"));
            }
            completed.fetch_add(1, Ordering::SeqCst);
            Ok(item)
        });
        assert!(output.is_err());
        assert_eq!(completed.load(Ordering::SeqCst), 3);
    }
    assert!(nn::parallel::ranges(1, 0).is_err());
    assert_eq!(nn::parallel::ranges(7, 3).unwrap(), [0..3, 3..5, 5..7]);
    assert!(nn::parallel::ranges(0, 3).unwrap().is_empty());
}

#[test]
fn npz_preserves_numpy_dtype_shape_signed_zero_nan_and_endianness() {
    // NumPy 2.5.3 np.savez; ZIP64 local headers, C-order arrays:
    // floats=np.array([0x80000000,0x7fc01234,0x7f800000,0xff800000,1,0x3f800000],
    // dtype='<u4').view('<f4').reshape(2,3); integers=np.array([-32768,12345,32767],
    // dtype='>i2'); empty=np.empty((2,0),dtype='?'); scalar=np.array(-0.0,dtype='<f8').
    let arrays = npz::decode(include_bytes!("fixtures/nn-arrays.npz")).unwrap();
    assert_eq!(arrays.len(), 4);
    assert_eq!(arrays["floats"].shape, [2, 3]);
    assert_eq!(arrays["floats"].dtype, "<f4");
    let bits: Vec<_> = arrays["floats"]
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&b| u32::from_le_bytes(b))
        .collect();
    assert_eq!(
        bits,
        [
            0x80000000, 0x7fc01234, 0x7f800000, 0xff800000, 1, 0x3f800000
        ]
    );
    assert_eq!(arrays["integers"].dtype, ">i2");
    assert_eq!(arrays["integers"].data, [0x80, 0, 0x30, 0x39, 0x7f, 0xff]);
    assert_eq!(arrays["empty"].shape, [2, 0]);
    assert!(arrays["empty"].data.is_empty());
    assert!(arrays["scalar"].shape.is_empty());
    assert_eq!(arrays["scalar"].data, (-0.0_f64).to_le_bytes());
    let values: Vec<_> = arrays
        .iter()
        .map(|(key, value)| (key.as_str(), value.clone()))
        .collect();
    assert_eq!(npz::decode(&npz::encode(&values).unwrap()).unwrap(), arrays);
}

#[test]
fn npz_rejects_corruption_duplicate_keys_and_mismatched_payloads() {
    let bytes = include_bytes!("fixtures/nn-arrays.npz");
    for cut in [0, 8, 100, bytes.len() - 1] {
        assert!(npz::decode(&bytes[..cut]).is_err());
    }
    let mut corrupt = bytes.to_vec();
    corrupt[100] ^= 1;
    assert!(npz::decode(&corrupt).is_err());
    let scalar = Array {
        dtype: "<f4".into(),
        shape: vec![],
        data: vec![0; 4],
    };
    assert!(npz::encode(&[("x", scalar.clone()), ("x", scalar.clone())]).is_err());
    for wrong in [
        Array {
            shape: vec![2],
            ..scalar.clone()
        },
        Array {
            dtype: "|O".into(),
            ..scalar.clone()
        },
        Array {
            shape: vec![usize::MAX, 2],
            ..scalar
        },
    ] {
        assert!(npz::encode(&[("x", wrong)]).is_err());
    }
}

#[test]
fn atomic_publication_replaces_the_whole_file_and_leaves_no_temporary_file() {
    let directory = std::env::temp_dir().join(format!("nn-atomic-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("state");
    nn::atomic_write(&path, b"first state").unwrap();
    nn::atomic_write(&path, b"next").unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"next");
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
    assert!(nn::atomic_write(&directory.join("absent/state"), b"error").is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

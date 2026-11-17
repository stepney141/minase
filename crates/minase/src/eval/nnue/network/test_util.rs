//! 学習結果に依存しない固定ネット。シードと生成順序も固定して再現可能にする。

use super::*;
use crate::eval::{
    features::{BOARD_FEATURE_COUNT, FEATURE_COUNT, PIECE_STATE_COUNT, mirror_feature},
    piece_values::piece_value_of_state,
};
use minase_core::rng::XorShift64;
use std::num::NonZeroU64;

/// P型、幅32、除数128の固定ネットを作る。
/// PSQTは駒割だけ、第1層の重みは−8〜8、バイアスは0〜64で、鏡映対を共有する。
/// L1の0〜14行は−16〜16、出力へ直接足す15行は−8〜8、L2は−48〜48、
/// 出力の重みは−64〜64とし、後段のバイアスは0にする。
/// 除数128での切り捨て後にも静かな着手の差を残し、駒割が支配する範囲として選んだ。
/// benchと固定ランダム対局の47標本で後段は最大24 cp、25通りとなり、
/// 初期局面の合法手後は12通りとなる。これらの性質はnetworkの試験で検査する。
pub fn fixed_layer_stack() -> LayerStackNetwork {
    LayerStackNetwork::new(layer_stack_parameters())
        .expect("fixed LayerStack satisfies all constraints")
}

/// 固定LayerStackの検証用パラメータを返す。
pub fn layer_stack_parameters() -> LayerStackParameters {
    let width = 32;
    let mut rng = XorShift64::new(NonZeroU64::new(0x7461_7461_7261_5031).unwrap());
    let mut ft_biases = vec![0; width];
    for j in 0..width / 2 {
        ft_biases[j] = (rng.next() % 65) as i16;
        ft_biases[j + width / 2] = ft_biases[j];
    }
    let mut ft_weights = vec![0; FEATURE_COUNT * width];
    for row in 0..FEATURE_COUNT {
        let mirror = mirror_feature(row);
        if row < mirror {
            for j in 0..width {
                let w = (rng.next() % 17) as i16 - 8;
                ft_weights[row * width + j] = w;
                ft_weights[mirror * width + (j + width / 2) % width] = w;
            }
        }
    }
    let mut psqt_weights = vec![0; FEATURE_COUNT];
    for (row, weight) in psqt_weights
        .iter_mut()
        .enumerate()
        .take(BOARD_FEATURE_COUNT)
    {
        let state = row / 144 % PIECE_STATE_COUNT;
        let sign = if row / (144 * PIECE_STATE_COUNT) == 0 {
            1
        } else {
            -1
        };
        *weight = 128 * piece_value_of_state(state) * sign;
    }
    let l1_weights: Vec<_> = (0..16 * width)
        .map(|i| {
            let range = if i / width == 15 { 8 } else { 16 };
            (rng.next() % (2 * range + 1)) as i8 - range as i8
        })
        .collect();
    LayerStackParameters {
        width,
        fv_scale: 128,
        ft_biases,
        ft_weights,
        psqt_bias: 0,
        psqt_weights,
        l1_biases: vec![0; 16],
        l1_weights,
        l2_biases: vec![0; 32],
        l2_weights: (0..32 * 30).map(|_| (rng.next() % 97) as i8 - 48).collect(),
        output_bias: 0,
        output_weights: (0..32)
            .map(|_| ((rng.next() % 129) as i16 - 64) as i8)
            .collect(),
    }
}

/// HalfKP_vm、幅16、除数128の固定ネットを作る。
/// 第1層の重みは−8〜8、バイアスは0〜64、後段の重みは各層とも−32〜32、バイアスは0。
/// 除数128での切り捨て後にも差を残すためにこの範囲を選んだ。
/// benchの最大絶対値は28 cp、初期局面の合法手後は11通りとなる。
/// 第1層の重みだけで30,855,168バイトを保持する。
pub fn fixed_simple() -> SimpleNetwork {
    SimpleNetwork::new(simple_parameters()).expect("fixed Simple satisfies all constraints")
}

pub(super) fn simple_parameters() -> SimpleParameters {
    let width = 16;
    let mut rng = XorShift64::new(NonZeroU64::new(0x7461_7461_7261_5331).unwrap());
    SimpleParameters {
        width,
        fv_scale: 128,
        ft_biases: (0..width).map(|_| (rng.next() % 65) as i16).collect(),
        ft_weights: (0..FeatureSet::HalfKpVm.spec().dimensions() * width)
            .map(|_| (rng.next() % 17) as i16 - 8)
            .collect(),
        l1_biases: vec![0; 32],
        l1_weights: (0..32 * 2 * width)
            .map(|_| (rng.next() % 65) as i8 - 32)
            .collect(),
        l2_biases: vec![0; 32],
        l2_weights: (0..32 * 32).map(|_| (rng.next() % 65) as i8 - 32).collect(),
        output_bias: 0,
        output_weights: (0..32).map(|_| (rng.next() % 65) as i8 - 32).collect(),
    }
}

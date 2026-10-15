use super::*;
use crate::pst::v0_piece_values;
use minase::eval::features::FEATURE_COUNT;
use minase_core::{Color, PieceCode, PieceKind, PositionBuilder, Square};

#[test]
fn signed_accumulators_phi_and_correction_follow_hand_calculated_pairs() {
    // 符号(+,-)、3行(2,1),(-3,4),(5,-2)のA=(4,3)、C=17、N=-10。
    // e=1では-10/8を0方向へ切り捨てて-1。Vを同じ値、a=(2,-3)とするとΦ=-4。
    let mut u = vec![[0; FM_RANK]; FEATURE_COUNT];
    u[0][..2].copy_from_slice(&[2, 1]);
    u[1][..2].copy_from_slice(&[-3, 4]);
    u[2][..2].copy_from_slice(&[5, -2]);
    let mut signs = [1; FM_RANK];
    signs[1] = -1;
    let fm = FmWeights {
        embeddings: &u,
        signs: &signs,
        exponent: 1,
    };
    let (sums, c) = accumulators(fm, &[0, 1, 2]);
    assert_eq!(&sums[..2], &[4, 3]);
    assert_eq!(c, 17);
    assert_eq!(numerator(fm, &[0, 1, 2]), -10);
    assert_eq!(correction(fm, &[0, 1, 2]), -1);
    assert_eq!(correction(fm, &[0, 2]), 3); // 対の内積は12、分母4。
    let v: Vec<_> = u.iter().map(|row| row.map(f64::from)).collect();
    let mut a = [0.0; FM_RANK];
    a[..2].copy_from_slice(&[2.0, -3.0]);
    assert_eq!(phi(&v, &a, &[0, 1, 2]), -4.0);
    assert_eq!(phi(&v, &a, &[2]), 0.0);
}

#[test]
fn tuning_scales_before_truncation_and_handles_large_denominators() {
    for sign in [-1, 1] {
        // trunc(12*768/(1024*8))=1だが、trunc(12/8)*768/1024は0。
        assert_eq!(tuning_correction(sign * 12, 768, 1).unwrap(), sign);
        assert_eq!(tuning_correction(sign * 7, 1024, 1).unwrap(), 0);
    }
    assert_eq!(tuning_correction(i64::MIN, 1024, 30).unwrap(), -4);
    assert_eq!(tuning_correction(i64::MAX, 1024, 30).unwrap(), 3);
    assert_eq!(tuning_correction(i64::MAX, 0, 30).unwrap(), 0);
    assert!(tuning_correction(1, -1, 0).is_err());
    assert!(tuning_correction(1, 1025, 0).is_err());
    assert!(tuning_correction(1, 1, 31).is_err());
}

#[test]
fn python_integer_definitions_match_engine_for_both_sides_lion_and_clipping() {
    for side in Color::ALL {
        for count in [2, 5, 144] {
            let mut builder = PositionBuilder::new(side);
            for i in 0..count {
                builder
                    .put(
                        Square::new((i % 12) as u8, (i / 12) as u8).unwrap(),
                        PieceCode::new(Color::ALL[i % 2], PieceKind::Pawn).unwrap(),
                    )
                    .unwrap();
            }
            let mut position = builder.finish().unwrap();
            position
                .set_lion_capture(Some(
                    Square::new(if side == Color::Black { 1 } else { 0 }, 0).unwrap(),
                ))
                .unwrap();
            let mut features = Vec::new();
            active_features(&position, |f| features.push(f));
            assert_eq!(features.len(), count + 1);
            for value in [i16::MIN, -7, 3, i16::MAX] {
                for sign in [-1, 1] {
                    let u = vec![[value; FM_RANK]; FEATURE_COUNT];
                    let signs = [sign; FM_RANK];
                    for exponent in [0, 1, 7, 30] {
                        let fm = FmWeights {
                            embeddings: &u,
                            signs: &signs,
                            exponent,
                        };
                        let (sums, c) = accumulators(fm, &features);
                        let n = count as i64 + 1;
                        assert_eq!(sums, [n * i64::from(value); FM_RANK]);
                        assert_eq!(c, n * 32 * i64::from(sign) * i64::from(value).pow(2));
                        // n個の特徴にn(n-1)/2対があり、各内積はsign*32*value²。
                        let pairs =
                            n * (n - 1) / 2 * 32 * i64::from(sign) * i64::from(value).pow(2);
                        let expected = pairs / (1_i64 << (2 * exponent));
                        assert_eq!(correction(fm, &features), expected);
                        let weights = vec![[8, -16]; FEATURE_COUNT];
                        let pst =
                            Pst::from_weights_with_fm(&weights, v0_piece_values(), 1.0, "test", fm)
                                .unwrap();
                        let baseline = i64::from(pst::evaluate_pst(&pst, &position));
                        assert_eq!(
                            i64::from(pst::evaluate(&pst, &position)),
                            (baseline + expected).clamp(-28999, 28999)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn numpy_f64_sum_preserves_the_documented_pairwise_order() {
    // A=2^54ではA+1と−A+1がともに偶数丸めで±Aになる。
    // 規定の木は(A−A)+(0+1)=1、前からの逐次加算は2、逆順の逐次加算は4。
    let block = [
        18_014_398_509_481_984.0,
        1.0,
        -18_014_398_509_481_984.0,
        1.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ];
    assert_eq!(numpy_sum(&block[..7]).to_bits(), 1.0_f64.to_bits());
    assert_eq!(numpy_sum(&block).to_bits(), 1.0_f64.to_bits());
    // 8本の部分和を2倍にすると木の和は2。末尾0.5は木の後に足す。
    assert_eq!(numpy_sum(&block.repeat(2)).to_bits(), 2.0_f64.to_bits());
    for count in [9, 129, 145] {
        let mut values = vec![0.0; count];
        values[..8].copy_from_slice(&block);
        values[count - 1] = 0.5;
        // 128超では先頭側の再帰和1と後半の再帰和0.5を足して1.5。
        assert_eq!(
            numpy_sum(&values).to_bits(),
            1.5_f64.to_bits(),
            "count={count}"
        );
    }
}

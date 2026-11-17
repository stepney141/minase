//! 期待値の根拠はフェーズ2a指示書の整数式、形式、および拒否条件。

use super::test_util::{
    fixed_layer_stack, fixed_simple, layer_stack_parameters, simple_parameters,
};
use super::*;
use crate::eval::features::{self, FEATURE_COUNT};
use crate::eval::nnue::input::{HalfKpVm, P, for_each_active_row};
use minase_core::rng::XorShift64;
use minase_core::test_util::{
    bench_positions, position_from_codes, reflect_ranks_and_swap_colors, sampled_random_positions,
    sq,
};
use minase_core::{Move, MoveGenerator, MoveRules, PieceCode, PieceKind, Square};
use std::num::NonZeroU64;

mod format;

#[test]
fn specified_integer_stages() {
    for (a, b, expected) in [
        (127, 100, 99),
        (-5, 100, 0),
        (127, 127, 126),
        (200, 100, 99),
    ] {
        assert_eq!(pair(a, b), expected);
    }
    for (y, expected) in [(3000, 17), (-3000, 17), (5000, 47), (40000, 127)] {
        assert_eq!(square_activation(y), expected);
    }
    for (y, expected) in [(3000, 46), (-1, 0), (8200, 127)] {
        assert_eq!(activation(y), expected);
    }
    assert_eq!(score(-17, 8), -2);
    assert_eq!(psqt_half(0, 7), -3);
    assert_eq!(score(40_000 * 8, 8), 28_999);
    assert_eq!(score(-40_000 * 8, 8), -28_999);
}

#[test]
fn architecture_and_hashes_match_tatara_formula() {
    assert_eq!(
        arch(FeatureSet::P, 256, 12),
        "Features=P(Friend)[13680->256x2],PSQT=1,Network=AffineTransform[1<-32](ClippedReLU[32](AffineTransform[32<-30](SqrClippedReLU[30](AffineTransform[16<-512](InputSlice[512(0:512)]))))),fv_scale=12"
    );
    assert_eq!(
        arch(FeatureSet::HalfKpVm, 256, 12),
        "Features=HalfKPVm(Friend)[964224->256x2],Network=AffineTransform[1<-32](ClippedReLU[32](AffineTransform[32<-32](ClippedReLU[32](AffineTransformSparseInput[32<-512](InputSlice[512(0:512)]))))),fv_scale=12"
    );
    for (set, name, first) in [
        (FeatureSet::P, b"P".as_slice(), 256),
        (FeatureSet::HalfKpVm, b"HalfKPVm".as_slice(), 32),
    ] {
        let feature = name.iter().fold(0x811C_9DC5_u32, |h, &b| {
            (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
        });
        let ft = feature ^ 512;
        // 指示書の各層の式を、実装の関数を使わず3層分展開する。
        let input = 0xEC42_E90D_u32 ^ 512;
        let l1 = ((0xCC03_DAE4_u32.wrapping_add(first)) ^ (input >> 1) ^ (input << 31))
            .wrapping_add(0x538D_24C7);
        let l2 =
            ((0xCC03_DAE4_u32.wrapping_add(32)) ^ (l1 >> 1) ^ (l1 << 31)).wrapping_add(0x538D_24C7);
        let fc = (0xCC03_DAE4_u32.wrapping_add(1)) ^ (l2 >> 1) ^ (l2 << 31);
        assert_eq!(ft_hash(set, 256), ft);
        assert_eq!(fc_hash(set, 256), fc);
        assert_eq!(&header(set, 256, 12)[4..8], &(fc ^ ft).to_le_bytes());
    }
}

fn specimens() -> Vec<Position> {
    bench_positions()
        .into_iter()
        .chain(sampled_random_positions(MoveRules::standard()))
        .collect()
}

/// 固定ネットは静かな局面を区別し、駒割との差を50 cp以内に保つ。
#[test]
fn fixed_layer_stack_dense_contribution_is_visible_and_bounded() {
    let net = fixed_layer_stack();
    let mut values = std::collections::BTreeSet::new();
    let mut max_difference = 0;
    for p in specimens() {
        let b = net.breakdown(&p);
        values.insert(b.dense / b.fv_scale);
        let material: i32 = Square::all()
            .filter_map(|s| p.piece_at(s))
            .map(|piece| {
                let sign = if piece.color().unwrap() == p.side_to_move() {
                    1
                } else {
                    -1
                };
                sign * crate::eval::handcrafted::piece_value(piece.kind().unwrap())
            })
            .sum();
        max_difference = max_difference.max((net.evaluate(&p) - material).abs());
    }
    let maximum = values.iter().map(|v| v.abs()).max().unwrap();
    eprintln!(
        "LayerStack: dense max={maximum} cp, distinct={}, material difference={max_difference} cp",
        values.len()
    );
    assert!((15..=50).contains(&maximum));
    assert!(values.len() >= 20);
    assert!(max_difference <= 50);
}

#[test]
fn fixed_networks_distinguish_initial_legal_moves() {
    let root = Position::initial();
    for net in [
        Network::LayerStack(fixed_layer_stack()),
        Network::Simple(fixed_simple()),
    ] {
        let values: std::collections::BTreeSet<_> = legal_moves(&root)
            .into_iter()
            .map(|mv| {
                let mut child = root.clone();
                child.make_move_unchecked(mv, MoveRules::standard());
                net.evaluate(&child).unwrap()
            })
            .collect();
        eprintln!(
            "{:?}: initial child distinct={}",
            net.feature_set(),
            values.len()
        );
        assert!(values.len() >= 10);
    }
}

#[test]
fn fixed_simple_bench_scores_are_visible() {
    let net = fixed_simple();
    let maximum = bench_positions()
        .iter()
        .map(|p| net.evaluate(p).unwrap().abs())
        .max()
        .unwrap();
    eprintln!("Simple: bench max={maximum} cp");
    assert!(maximum >= 15);
}

/// 第1層もi64で加算し、剰余演算や製品の活性化関数を用いない参照。
fn reference_ft<I: Indexer>(
    p: &Position,
    side: Color,
    width: usize,
    bias: &[i16],
    weights: &[i16],
) -> Vec<i64> {
    let context = I::context(p, side).unwrap();
    let mut rows = Vec::new();
    for_each_active_row::<I>(context, side, p, |row| rows.push(row));
    (0..width)
        .map(|j| {
            i64::from(bias[j])
                + rows
                    .iter()
                    .map(|r| i64::from(weights[r * width + j]))
                    .sum::<i64>()
        })
        .collect()
}
fn reference_layer(p: &Position, side: Color, n: &LayerStackParameters) -> i32 {
    let mut x = Vec::new();
    let mut sums = [0_i64; 2];
    for (part, perspective) in [side, side.opposite()].into_iter().enumerate() {
        let acc = reference_ft::<P>(p, perspective, n.width, &n.ft_biases, &n.ft_weights);
        for i in 0..n.width / 2 {
            x.push((acc[i].clamp(0, 127) * acc[i + n.width / 2].clamp(0, 127)) >> 7);
        }
        features::active_features_for(perspective, p, |r| {
            sums[part] += i64::from(n.psqt_weights[r])
        });
    }
    let mut y = [0_i64; 16];
    for (j, y) in y.iter_mut().enumerate() {
        *y = i64::from(n.l1_biases[j]);
        for (i, &x) in x.iter().enumerate() {
            *y += i64::from(n.l1_weights[j * n.width + i]) * x;
        }
    }
    let mut z = [0_i64; 30];
    for i in 0..15 {
        z[i] = ((y[i] * y[i]) >> 19).clamp(0, 127);
        z[15 + i] = (y[i] >> 6).clamp(0, 127);
    }
    let mut out = i64::from(n.output_bias);
    for k in 0..32 {
        let mut u = i64::from(n.l2_biases[k]);
        for (i, &z) in z.iter().enumerate() {
            u += i64::from(n.l2_weights[k * 30 + i]) * z;
        }
        out += i64::from(n.output_weights[k]) * (u >> 6).clamp(0, 127);
    }
    ((out + y[15] + (sums[0] - sums[1]) / 2) / i64::from(n.fv_scale)).clamp(-28_999, 28_999) as i32
}
fn reference_simple(p: &Position, side: Color, n: &SimpleParameters) -> i32 {
    let mut x = Vec::new();
    for perspective in [side, side.opposite()] {
        let acc = reference_ft::<HalfKpVm>(p, perspective, n.width, &n.ft_biases, &n.ft_weights);
        for a in acc {
            x.push(a.clamp(0, 127));
        }
    }
    let mut h1 = [0_i64; 32];
    for (j, h) in h1.iter_mut().enumerate() {
        *h = i64::from(n.l1_biases[j]);
        for (i, &x) in x.iter().enumerate() {
            *h += i64::from(n.l1_weights[j * 2 * n.width + i]) * x;
        }
        *h = (*h >> 6).clamp(0, 127);
    }
    let mut h2 = [0_i64; 32];
    for (k, h) in h2.iter_mut().enumerate() {
        *h = i64::from(n.l2_biases[k]);
        for (j, &h1) in h1.iter().enumerate() {
            *h += i64::from(n.l2_weights[k * 32 + j]) * h1;
        }
        *h = (*h >> 6).clamp(0, 127);
    }
    let mut out = i64::from(n.output_bias);
    for (k, &h) in h2.iter().enumerate() {
        out += i64::from(n.output_weights[k]) * h;
    }
    (out / i64::from(n.fv_scale)).clamp(-28_999, 28_999) as i32
}
fn mirrored(p: &Position) -> Position {
    let pieces: Vec<_> = p
        .occupied()
        .into_iter()
        .map(|s| (sq(11 - s.file(), s.rank()), p.piece_at(s).unwrap()))
        .collect();
    let mut result = position_from_codes(p.side_to_move(), &pieces);
    result
        .set_lion_capture(
            p.lion_taken_by_non_lion()
                .map(|t| sq(11 - t.square.file(), t.square.rank())),
        )
        .unwrap();
    result
}
fn large_biases(rng: &mut XorShift64, values: &mut [i32]) {
    for value in values {
        let sign = if rng.next().is_multiple_of(2) { 1 } else { -1 };
        *value = sign * ((1 << 24) - (rng.next() % 128) as i32);
    }
}
fn large_layer() -> LayerStackParameters {
    let mut p = layer_stack_parameters();
    p.fv_scale = 4096;
    for v in &mut p.ft_weights {
        *v *= 28;
    }
    for v in &mut p.ft_biases {
        *v *= 3;
    }
    let mut rng = XorShift64::new(NonZeroU64::new(0x9911).unwrap());
    large_biases(&mut rng, &mut p.l1_biases);
    large_biases(&mut rng, &mut p.l2_biases);
    large_biases(&mut rng, std::slice::from_mut(&mut p.output_bias));
    for v in p
        .l1_weights
        .iter_mut()
        .chain(&mut p.l2_weights)
        .chain(&mut p.output_weights)
    {
        *v = rng.next() as i8;
    }
    for row in 0..FEATURE_COUNT {
        if row < features::mirror_feature(row) {
            let v = (rng.next() % (1 << 23)) as i32 - (1 << 22);
            p.psqt_weights[row] = v;
            p.psqt_weights[features::mirror_feature(row)] = v;
        }
    }
    p
}
fn large_simple() -> SimpleParameters {
    let mut p = simple_parameters();
    p.fv_scale = 4096;
    for v in &mut p.ft_weights {
        *v *= 28;
    }
    for v in &mut p.ft_biases {
        *v *= 3;
    }
    let mut rng = XorShift64::new(NonZeroU64::new(0x9922).unwrap());
    large_biases(&mut rng, &mut p.l1_biases);
    large_biases(&mut rng, &mut p.l2_biases);
    large_biases(&mut rng, std::slice::from_mut(&mut p.output_bias));
    for v in p
        .l1_weights
        .iter_mut()
        .chain(&mut p.l2_weights)
        .chain(&mut p.output_weights)
    {
        *v = rng.next() as i8;
    }
    p
}

// 製品の型を試験都合で変更せず、参照用パラメータとネットを同じ生成手順から別々に作る。
#[test]
fn layer_stack_matches_i64_reference_incrementally_and_under_symmetries() {
    for build in [layer_stack_parameters, interior_layer, large_layer] {
        let oracle = build();
        let net = LayerStackNetwork::new(build()).unwrap();
        for mut p in specimens() {
            for side in Color::ALL {
                assert_eq!(
                    net.evaluate_accumulator(&net.refresh(&p), side),
                    reference_layer(&p, side, &oracle)
                );
            }
            assert_eq!(net.evaluate(&p), net.evaluate(&mirrored(&p)));
            assert_eq!(
                net.evaluate(&p),
                net.evaluate(&reflect_ranks_and_swap_colors(&p))
            );
            let before = net.refresh(&p);
            let mut after = net.new_accumulator();
            let moves = legal_moves(&p);
            if let Some(&mv) = moves.first() {
                let undo = p.make_move_unchecked(mv, MoveRules::standard());
                net.update_after_move(&before, &mut after, &p, &undo);
                assert_eq!(after, net.refresh(&p));
                assert_eq!(
                    net.evaluate_accumulator(&after, p.side_to_move()),
                    reference_layer(&p, p.side_to_move(), &oracle)
                );
            }
        }
        for seed in [0x1234, 0x5678, 0x9abc, 0xdef0] {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut p = Position::initial();
            let mut acc = net.refresh(&p);
            let mut next = net.new_accumulator();
            for ply in 0..128 {
                assert_eq!(
                    net.evaluate(&p),
                    reference_layer(&p, p.side_to_move(), &oracle),
                    "seed={seed} ply={ply}"
                );
                assert_eq!(
                    net.evaluate_accumulator(&acc, p.side_to_move()),
                    net.evaluate(&p)
                );
                assert_eq!(net.evaluate(&p), net.evaluate(&mirrored(&p)));
                assert_eq!(
                    net.evaluate(&p),
                    net.evaluate(&reflect_ranks_and_swap_colors(&p))
                );
                if !has_royals(&p) {
                    break;
                }
                let moves = legal_moves(&p);
                if moves.is_empty() {
                    break;
                }
                let mv = moves[rng.next() as usize % moves.len()];
                let undo = p.make_move_unchecked(mv, MoveRules::standard());
                net.update_after_move(&acc, &mut next, &p, &undo);
                std::mem::swap(&mut acc, &mut next);
                assert_eq!(acc, net.refresh(&p));
                let expected = reference_layer(&p, p.side_to_move(), &oracle);
                assert_eq!(net.evaluate_accumulator(&acc, p.side_to_move()), expected);
                assert_eq!(net.evaluate(&p), expected);
                let mut nulled = p.clone();
                let lion = p.lion_taken_by_non_lion().map(|t| t.square);
                nulled.make_null_move();
                net.update_after_null(&acc, &mut next, lion);
                assert_eq!(next, net.refresh(&nulled));
                assert_eq!(
                    net.evaluate_accumulator(&next, nulled.side_to_move()),
                    reference_layer(&nulled, nulled.side_to_move(), &oracle)
                );
            }
        }
    }
}
fn legal_moves(p: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    MoveGenerator::new(MoveRules::standard()).generate_moves(p, &mut moves);
    moves
}
fn has_royals(p: &Position) -> bool {
    Color::ALL
        .into_iter()
        .all(|s| !p.royal_pieces(s).is_empty())
}

#[test]
fn simple_matches_i64_reference_incrementally_and_under_symmetries() {
    for build in [simple_parameters, interior_simple, large_simple] {
        let oracle = build();
        let net = SimpleNetwork::new(build()).unwrap();
        for mut p in specimens() {
            if !has_royals(&p) {
                assert_missing_royal(&net, &p);
                continue;
            }
            for side in Color::ALL {
                assert_eq!(
                    net.evaluate_for(&p, side).unwrap(),
                    reference_simple(&p, side, &oracle)
                );
            }
            assert_eq!(net.evaluate(&p), net.evaluate(&mirrored(&p)));
            assert_eq!(
                net.evaluate(&p),
                net.evaluate(&reflect_ranks_and_swap_colors(&p))
            );
            let before = net.refresh(&p);
            let mut after = net.new_accumulator();
            if let Some(&mv) = legal_moves(&p).first() {
                let undo = p.make_move_unchecked(mv, MoveRules::standard());
                net.update_after_move(&before, &mut after, &p, &undo);
                assert_eq!(after, net.refresh(&p));
                if has_royals(&p) {
                    assert_eq!(
                        net.evaluate_accumulator(&after, p.side_to_move()),
                        reference_simple(&p, p.side_to_move(), &oracle)
                    );
                }
            }
        }
        for seed in [0x1234, 0x5678, 0x9abc, 0xdef0] {
            let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
            let mut p = Position::initial();
            let mut acc = net.refresh(&p);
            let mut next = net.new_accumulator();
            for ply in 0..128 {
                if !has_royals(&p) {
                    assert!(net.evaluate(&p).is_err());
                    break;
                }
                assert_eq!(
                    net.evaluate(&p).unwrap(),
                    reference_simple(&p, p.side_to_move(), &oracle),
                    "seed={seed} ply={ply}"
                );
                assert_eq!(
                    net.evaluate_accumulator(&acc, p.side_to_move()),
                    net.evaluate(&p).unwrap()
                );
                assert_eq!(net.evaluate(&p), net.evaluate(&mirrored(&p)));
                assert_eq!(
                    net.evaluate(&p),
                    net.evaluate(&reflect_ranks_and_swap_colors(&p))
                );
                let moves = legal_moves(&p);
                if moves.is_empty() {
                    break;
                }
                let undo = p.make_move_unchecked(
                    moves[rng.next() as usize % moves.len()],
                    MoveRules::standard(),
                );
                net.update_after_move(&acc, &mut next, &p, &undo);
                std::mem::swap(&mut acc, &mut next);
                assert_eq!(acc, net.refresh(&p));
                if has_royals(&p) {
                    let expected = reference_simple(&p, p.side_to_move(), &oracle);
                    assert_eq!(net.evaluate_accumulator(&acc, p.side_to_move()), expected);
                    assert_eq!(net.evaluate(&p).unwrap(), expected);
                } else {
                    assert_missing_royal(&net, &p);
                }
                let mut nulled = p.clone();
                let lion = p.lion_taken_by_non_lion().map(|t| t.square);
                nulled.make_null_move();
                net.update_after_null(&acc, &mut next, lion);
                assert_eq!(next, net.refresh(&nulled));
                if has_royals(&nulled) {
                    assert_eq!(
                        net.evaluate_accumulator(&next, nulled.side_to_move()),
                        reference_simple(&nulled, nulled.side_to_move(), &oracle)
                    );
                }
            }
        }
    }
}

#[test]
fn psqt_reproduces_material_at_all_specified_scales() {
    for scale in [1, 8, 12, 128] {
        let mut p = layer_stack_parameters();
        p.fv_scale = scale;
        p.ft_biases.fill(0);
        p.ft_weights.fill(0);
        p.l1_weights.fill(0);
        p.l2_weights.fill(0);
        p.output_weights.fill(0);
        for w in &mut p.psqt_weights {
            *w = *w / 128 * scale;
        }
        let net = LayerStackNetwork::new(p).unwrap();
        for position in specimens() {
            let expected: i32 = position
                .occupied()
                .into_iter()
                .map(|s| {
                    let piece = position.piece_at(s).unwrap();
                    let sign = if piece.color().unwrap() == position.side_to_move() {
                        1
                    } else {
                        -1
                    };
                    sign * crate::eval::handcrafted::piece_value(piece.kind().unwrap())
                })
                .sum();
            assert_eq!(net.evaluate(&position), expected);
            let b = net.breakdown(&position);
            assert_eq!(b.value, expected);
            assert_eq!(b.fv_scale, scale);
            assert_eq!(b.dense, 0);
            assert_eq!(b.psqt, b.squares.iter().sum::<i32>() / 2);
        }
    }
}

#[test]
fn quiet_delta_and_breakdowns_obey_perspective_and_promotion() {
    let net = fixed_layer_stack();
    for side in Color::ALL {
        for promote in [false, true] {
            let black = PieceCode::new(Color::Black, PieceKind::King).unwrap();
            let white = PieceCode::new(Color::White, PieceKind::King).unwrap();
            let pawn = PieceCode::new(side, PieceKind::Pawn).unwrap();
            let from = sq(5, if side == Color::Black { 7 } else { 4 });
            let to = sq(5, if side == Color::Black { 8 } else { 3 });
            let mut p = position_from_codes(
                side,
                &[(sq(0, 0), black), (sq(11, 11), white), (from, pawn)],
            );
            let mv = Move {
                from,
                to,
                mid: None,
                promote,
            };
            assert!(legal_moves(&p).contains(&mv));
            let before = net.breakdown(&p);
            let delta = net.quiet_move_psqt_delta(&p, mv);
            p.make_move_unchecked(mv, MoveRules::standard());
            let after = net.breakdown(&p);
            assert_eq!(before.lion, 0);
            assert_eq!(after.lion, 0);
            // 着手後のbreakdownは手番が逆なので、前の手番の視点に戻す。
            assert_eq!(
                delta,
                -after.squares.iter().map(|&v| i64::from(v)).sum::<i64>()
                    - before.squares.iter().map(|&v| i64::from(v)).sum::<i64>()
            );
        }
    }
    let simple = fixed_simple();
    for p in specimens() {
        if !has_royals(&p) {
            assert_missing_royal(&simple, &p);
            assert!(simple.breakdown(&p).is_err());
            continue;
        }
        let b = simple.breakdown(&p).unwrap();
        assert_eq!(b.value, simple.evaluate(&p).unwrap());
        assert_eq!(b.fv_scale, 128);
        assert_eq!(
            b.value,
            (i64::from(b.out) / 128).clamp(-28_999, 28_999) as i32
        );
    }
}

#[test]
fn fixed_layer_stack_file_matches_generator() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),"/nets/test-layer-stack.bin")).expect("regenerate with: nice -n 19 cargo test -j 8 -p minase --lib eval::nnue::network::tests::regenerate_layer_stack_fixture -- --ignored --exact");
    assert_eq!(
        bytes,
        fixed_layer_stack().encode(),
        "regenerate with: nice -n 19 cargo test -j 8 -p minase --lib eval::nnue::network::tests::regenerate_layer_stack_fixture -- --ignored --exact"
    );
}
#[test]
#[ignore = "固定ネットを書き直すときだけ明示的に実行する"]
fn regenerate_layer_stack_fixture() {
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/nets/test-layer-stack.bin"),
        fixed_layer_stack().encode(),
    )
    .unwrap();
}

fn assert_missing_royal(net: &SimpleNetwork, p: &Position) {
    let color = Color::ALL
        .into_iter()
        .find(|&s| p.royal_pieces(s).is_empty())
        .unwrap();
    assert_eq!(net.evaluate(p), Err(AnchorError::MissingRoyal { color }));
    for side in Color::ALL {
        assert_eq!(
            net.evaluate_for(p, side),
            Err(AnchorError::MissingRoyal { color })
        );
    }
}

#[test]
fn lion_capture_expiration_and_null_update_nonzero_psqt() {
    let oracle = large_layer();
    let net = LayerStackNetwork::new(large_layer()).unwrap();
    let simple_oracle = large_simple();
    let simple = SimpleNetwork::new(large_simple()).unwrap();
    let piece = |side, kind| PieceCode::new(side, kind).unwrap();
    let original = position_from_codes(
        Color::Black,
        &[
            (sq(0, 11), piece(Color::Black, PieceKind::King)),
            (sq(11, 0), piece(Color::White, PieceKind::King)),
            (sq(5, 5), piece(Color::Black, PieceKind::Rook)),
            (sq(5, 3), piece(Color::White, PieceKind::Lion)),
            (sq(9, 3), piece(Color::White, PieceKind::Pawn)),
        ],
    );
    for swapped in [false, true] {
        let mut p = if swapped {
            reflect_ranks_and_swap_colors(&original)
        } else {
            original.clone()
        };
        let reflect = |s: minase_core::Square| {
            if swapped {
                sq(s.file(), 11 - s.rank())
            } else {
                s
            }
        };
        let mut before = net.refresh(&p);
        let mut after = net.new_accumulator();
        let mut simple_before = simple.refresh(&p);
        let mut simple_after = simple.new_accumulator();
        for (from, to) in [(sq(5, 5), sq(5, 3)), (sq(9, 3), sq(9, 2))] {
            let mv = Move {
                from: reflect(from),
                to: reflect(to),
                mid: None,
                promote: false,
            };
            assert!(legal_moves(&p).contains(&mv));
            let undo = p.make_move_unchecked(mv, MoveRules::standard());
            net.update_after_move(&before, &mut after, &p, &undo);
            simple.update_after_move(&simple_before, &mut simple_after, &p, &undo);
            assert_eq!(after, net.refresh(&p));
            assert_eq!(simple_after, simple.refresh(&p));
            assert_eq!(
                net.evaluate_accumulator(&after, p.side_to_move()),
                reference_layer(&p, p.side_to_move(), &oracle)
            );
            assert_eq!(
                simple.evaluate_accumulator(&simple_after, p.side_to_move()),
                reference_simple(&p, p.side_to_move(), &simple_oracle)
            );
            let b = net.breakdown(&p);
            let expected_lion = match p.lion_taken_by_non_lion() {
                Some(t) => {
                    oracle.psqt_weights[features::lion_feature_index(p.side_to_move(), t.square)]
                        - oracle.psqt_weights
                            [features::lion_feature_index(p.side_to_move().opposite(), t.square)]
                }
                None => 0,
            };
            assert_eq!(b.lion, expected_lion);
            assert_eq!(b.psqt, (b.squares.iter().sum::<i32>() + expected_lion) / 2);
            let mut nulled = p.clone();
            let lion = p.lion_taken_by_non_lion().map(|t| t.square);
            nulled.make_null_move();
            let mut null_acc = net.new_accumulator();
            net.update_after_null(&after, &mut null_acc, lion);
            assert_eq!(null_acc, net.refresh(&nulled));
            assert_eq!(
                net.evaluate_accumulator(&null_acc, nulled.side_to_move()),
                reference_layer(&nulled, nulled.side_to_move(), &oracle)
            );
            let mut simple_null = simple.new_accumulator();
            simple.update_after_null(&simple_after, &mut simple_null, lion);
            assert_eq!(simple_null, simple.refresh(&nulled));
            assert_eq!(
                simple.evaluate_accumulator(&simple_null, nulled.side_to_move()),
                reference_simple(&nulled, nulled.side_to_move(), &simple_oracle)
            );
            std::mem::swap(&mut before, &mut after);
            std::mem::swap(&mut simple_before, &mut simple_after);
        }
    }
}

#[test]
fn full_forward_has_specified_skip_psqt_and_truncation_order() {
    let mut p = layer_stack_parameters();
    p.ft_weights.fill(0);
    p.ft_biases.fill(127);
    p.l1_weights.fill(0);
    p.l2_weights.fill(0);
    p.output_weights.fill(0);
    p.psqt_weights.fill(0);
    p.fv_scale = 8;
    // L1のy0=-3000は平方側17、通常側0。L2のu0=17*64、出力は17*2。
    // skip=-51、PSQT=-3を足して-20、8で割って-2となる。
    p.l1_biases[0] = -3000;
    p.l1_biases[15] = -51;
    p.l2_weights[0] = 64;
    p.output_weights[0] = 2;
    let pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let square = sq(5, 5);
    let row = features::feature_index(Color::White, pawn, square);
    p.psqt_weights[row] = 7;
    p.psqt_weights[features::mirror_feature(row)] = 7;
    let net = LayerStackNetwork::new(p).unwrap();
    let position = position_from_codes(Color::Black, &[(square, pawn)]);
    let b = net.breakdown(&position);
    assert_eq!(b.dense, -17);
    assert_eq!(b.psqt, -3);
    assert_eq!(b.value, -2);
    assert_eq!(net.evaluate(&position), -2);

    let mut p = simple_parameters();
    p.ft_weights.fill(0);
    p.ft_biases.fill(100);
    p.l1_weights.fill(0);
    p.l2_weights.fill(0);
    p.output_weights.fill(0);
    p.fv_scale = 8;
    // x0=100、L1=(100*64-3200)>>6=50、L2=(50*64)>>6=50。
    // 出力は50*2-117=-17で、除算の結果は-2。
    p.l1_weights[0] = 64;
    p.l1_biases[0] = -3200;
    p.l2_weights[0] = 64;
    p.output_weights[0] = 2;
    p.output_bias = -117;
    let net = SimpleNetwork::new(p).unwrap();
    let b = net.breakdown(&Position::initial()).unwrap();
    assert_eq!(b.out, -17);
    assert_eq!(b.value, -2);
}

#[test]
fn simple_rejects_missing_or_ambiguous_anchors_without_panicking() {
    let net = Network::Simple(fixed_simple());
    let king = |side| PieceCode::new(side, PieceKind::King).unwrap();
    let prince = PieceCode::new_promoted(Color::Black, PieceKind::CrownPrince).unwrap();
    for (pieces, expected) in [
        (
            vec![(sq(11, 11), king(Color::White))],
            AnchorError::MissingRoyal {
                color: Color::Black,
            },
        ),
        (
            vec![
                (sq(11, 11), king(Color::White)),
                (sq(1, 1), king(Color::Black)),
                (sq(2, 2), king(Color::Black)),
            ],
            AnchorError::MultipleKings {
                color: Color::Black,
            },
        ),
        (
            vec![
                (sq(11, 11), king(Color::White)),
                (sq(1, 1), prince),
                (sq(2, 2), prince),
            ],
            AnchorError::MultiplePrinces {
                color: Color::Black,
            },
        ),
    ] {
        let p = position_from_codes(Color::Black, &pieces);
        assert_eq!(net.evaluate(&p), Err(expected));
        for side in Color::ALL {
            assert_eq!(net.evaluate_for(&p, side), Err(expected));
        }
    }
}

// 上限近くのバイアスだけでは活性化が飽和し続けるため、中間の範囲も照合する。
fn interior_biases(values: &mut [i32]) {
    for value in values {
        *value %= 4001;
    }
}
fn interior_layer() -> LayerStackParameters {
    let mut p = large_layer();
    p.fv_scale = 128;
    interior_biases(&mut p.l1_biases);
    interior_biases(&mut p.l2_biases);
    p.output_bias %= 4001;
    // 大きなPSQTだけで最終値が飽和することを避ける。
    for q in &mut p.psqt_weights {
        *q %= 1001;
    }
    p
}
fn interior_simple() -> SimpleParameters {
    let mut p = large_simple();
    p.fv_scale = 128;
    interior_biases(&mut p.l1_biases);
    interior_biases(&mut p.l2_biases);
    p.output_bias %= 4001;
    p
}

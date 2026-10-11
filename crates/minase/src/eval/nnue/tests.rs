//! 共通土台の形式、整数推論、差分更新および鏡映不変性の契約を検査する。

use super::accumulator::Accumulator;
use super::feature_set::{HalfKpContext, HalfKpHmB8, Mirror};
use super::format::HEADER_LENGTH;
use super::*;
use crate::eval::features::mirror_feature;
use minase_core::test_util::{position_from_codes, sq};
use minase_core::{Move, MoveGenerator, MoveRules};
use sha2::{Digest, Sha256};

/// 同じ仕様の検査を両入力特徴集合へ具体化する。
macro_rules! both_feature_sets {
    ($name:ident) => {
        mod $name {
            #[test]
            fn p() {
                super::$name::<super::P>();
            }
            #[test]
            fn half_kp() {
                super::$name::<super::HalfKpHmB8>();
            }
        }
    };
}

fn set_architecture<F: FeatureSet>(bytes: &mut [u8], width: usize) {
    let text = format!(
        "features={};ft={width};linear=tapered2;head=direct-screlu",
        F::NAME
    );
    bytes[8..104].fill(0);
    bytes[8..8 + text.len()].copy_from_slice(text.as_bytes());
}

/// P型の各行を各区分の同じ基底特徴へ複写し、出力までのパラメータを保つ。
fn copy_p_rows<F: FeatureSet>(source: &[u8]) -> Vec<u8> {
    let width = u32::from_le_bytes(source[104..108].try_into().unwrap()) as usize;
    let hidden = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let end = hidden + FEATURE_COUNT * width * 2;
    let mut bytes = source[..hidden].to_vec();
    set_architecture::<F>(&mut bytes, width);
    for _ in 0..F::DIMENSIONS / FEATURE_COUNT {
        bytes.extend_from_slice(&source[hidden..end]);
    }
    bytes.extend_from_slice(&source[end..]);
    refresh_checksum(&mut bytes);
    bytes
}

fn refresh_checksum(bytes: &mut [u8]) {
    let checksum = Sha256::digest(&bytes[HEADER_LENGTH..]);
    bytes[144..176].copy_from_slice(&checksum);
}

fn next(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state
}

/// 任意の許容幅で、鏡映共有された非零の重みを独立に符号化する。
fn random_bytes<F: FeatureSet>(width: usize) -> Vec<u8> {
    let mut state = 17;
    let mut linear = vec![[0_i16; 2]; FEATURE_COUNT];
    let mut hidden = vec![0_i16; F::DIMENSIONS * width];
    for f in 0..FEATURE_COUNT {
        let partner = mirror_feature(f);
        if f > partner {
            continue;
        }
        let pair = std::array::from_fn(|_| (next(&mut state) % 1601) as i16 - 800);
        linear[f] = pair;
        linear[partner] = pair;
        for j in 0..width {
            let value = (next(&mut state) % 25) as i16 - 12;
            hidden[f * width + j] = value;
            hidden[partner * width + (j + width / 2) % width] = value;
        }
    }
    if F::MIRROR == Mirror::Normalized {
        for value in &mut hidden {
            *value = (next(&mut state) % 25) as i16 - 12;
        }
    }
    let mut bytes = EMBEDDED[..HEADER_LENGTH].to_vec();
    set_architecture::<F>(&mut bytes, width);
    bytes[104..108].copy_from_slice(&(width as u32).to_le_bytes());
    for pair in linear {
        for value in pair {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(
        &EMBEDDED[HEADER_LENGTH + FEATURE_COUNT * 4
            ..HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4],
    );
    for value in hidden {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let mut biases: Vec<i32> = (0..width)
        .map(|_| (next(&mut state) % 101) as i32)
        .collect();
    if F::MIRROR == Mirror::Shared {
        for j in 0..width / 2 {
            biases[j + width / 2] = biases[j];
        }
    }
    for value in biases {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for _ in 0..2 {
        let mut row: Vec<i16> = (0..width)
            .map(|_| (next(&mut state) % 8193) as i16 - 4096)
            .collect();
        if F::MIRROR == Mirror::Shared {
            for j in 0..width / 2 {
                row[j + width / 2] = row[j];
            }
        }
        for value in row {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&(-19_713_i32).to_le_bytes());
    refresh_checksum(&mut bytes);
    bytes
}

fn networks<F: FeatureSet>() -> [Network<F, DirectScrelu>; 2] {
    [
        Network::decode(&copy_p_rows::<F>(EMBEDDED)).unwrap(),
        Network::decode(&random_bytes::<F>(4)).unwrap(),
    ]
}

/// 着手前後の差分累算値が完全再計算と一致し、親累算値を変更しないことを検査する。
fn assert_move_accumulator<F: FeatureSet>(
    nnue: &Network<F, DirectScrelu>,
    position: &mut Position,
    mv: Move,
) {
    let before = nnue.refresh_accumulator(position);
    let undo = position.make_move_unchecked(mv, MoveRules::standard());
    let mut after = Accumulator::new(nnue.width());
    nnue.update_accumulator_after_move(&before, &mut after, position, &undo);
    assert_eq!(after, nnue.refresh_accumulator(position));
    assert_eq!(
        nnue.evaluate_accumulator(&after, position.side_to_move()),
        evaluate(nnue, position)
    );
    position.unmake_move(undo);
    assert_eq!(before, nnue.refresh_accumulator(position));
}

both_feature_sets!(accumulator_updates_match_full_refresh_across_move_shapes);
/// 差分累算値が通常手、特殊移動、先獅子状態、およびnull moveで完全再計算と一致する。
fn accumulator_updates_match_full_refresh_across_move_shapes<F: FeatureSet>() {
    for nnue in &networks::<F>() {
        let black_king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let white_king = PieceCode::new(Color::White, PieceKind::King).unwrap();
        let black_pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
        let white_pawn = PieceCode::new(Color::White, PieceKind::Pawn).unwrap();
        let black_lion = PieceCode::new(Color::Black, PieceKind::Lion).unwrap();

        let mut promotion = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 3), black_pawn),
                (sq(5, 2), white_pawn),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut promotion,
            Move {
                from: sq(5, 3),
                mid: None,
                to: sq(5, 2),
                promote: true,
            },
        );

        let mut lion = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_lion),
                (sq(5, 4), white_pawn),
                (sq(5, 3), white_pawn),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut lion,
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 4)),
                to: sq(5, 3),
                promote: false,
            },
        );

        let mut igui = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_lion),
                (sq(5, 4), white_pawn),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut igui,
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 4)),
                to: sq(5, 5),
                promote: false,
            },
        );

        let mut jitto = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_lion),
            ],
        );
        assert_move_accumulator(
            nnue,
            &mut jitto,
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 4)),
                to: sq(5, 5),
                promote: false,
            },
        );

        let black_rook = PieceCode::new(Color::Black, PieceKind::Rook).unwrap();
        let white_lion = PieceCode::new(Color::White, PieceKind::Lion).unwrap();
        let mut lion_capture = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), black_king),
                (sq(11, 0), white_king),
                (sq(5, 5), black_rook),
                (sq(5, 3), white_lion),
            ],
        );
        let before = nnue.refresh_accumulator(&lion_capture);
        let undo = lion_capture.make_move_unchecked(
            Move {
                from: sq(5, 5),
                mid: None,
                to: sq(5, 3),
                promote: false,
            },
            MoveRules::standard(),
        );
        let mut after = Accumulator::new(nnue.width());
        nnue.update_accumulator_after_move(&before, &mut after, &lion_capture, &undo);
        assert_eq!(after, nnue.refresh_accumulator(&lion_capture));
        assert_eq!(after.piece_count, 3);
        assert!(lion_capture.lion_taken_by_non_lion().is_some());
        for side in Color::ALL {
            assert_eq!(
                evaluate_for_diagnostics(nnue, &lion_capture, side),
                evaluate_for_diagnostics(nnue, &mirror(&lion_capture), side)
            );
        }
        assert_eq!(
            nnue.evaluate_accumulator(&after, lion_capture.side_to_move()),
            evaluate(nnue, &lion_capture)
        );

        let mut normal_response = lion_capture.clone();
        assert_move_accumulator(
            nnue,
            &mut normal_response,
            Move {
                from: sq(11, 0),
                mid: None,
                to: sq(10, 0),
                promote: false,
            },
        );

        let lion_before = lion_capture
            .lion_taken_by_non_lion()
            .map(|trigger| trigger.square);
        let null_undo = lion_capture.make_null_move();
        let mut after_null = Accumulator::new(nnue.width());
        nnue.update_accumulator_after_null(&after, &mut after_null, lion_before);
        assert_eq!(after_null, nnue.refresh_accumulator(&lion_capture));
        assert_eq!(after_null.piece_count, 3);
        assert_eq!(
            nnue.evaluate_accumulator(&after_null, lion_capture.side_to_move()),
            evaluate(nnue, &lion_capture)
        );
        lion_capture.unmake_null_move(null_undo);
        assert_eq!(after, nnue.refresh_accumulator(&lion_capture));
        assert_eq!(after.piece_count, 3);
        assert!(lion_capture.lion_taken_by_non_lion().is_some());
        for side in Color::ALL {
            assert_eq!(
                evaluate_for_diagnostics(nnue, &lion_capture, side),
                evaluate_for_diagnostics(nnue, &mirror(&lion_capture), side)
            );
        }
        assert_eq!(
            nnue.evaluate_accumulator(&after, lion_capture.side_to_move()),
            evaluate(nnue, &lion_capture)
        );
        lion_capture.unmake_move(undo);
        assert_eq!(before, nnue.refresh_accumulator(&lion_capture));
    }
}

both_feature_sets!(royal_promotion_and_captures_update_all_features);
/// 太子への成りと王・太子の捕獲でも、累算値と取消が全再計算に一致する。
fn royal_promotion_and_captures_update_all_features<F: FeatureSet>() {
    for nnue in networks::<F>() {
        let king = PieceCode::new(Color::Black, PieceKind::King).unwrap();
        let enemy_king = PieceCode::new(Color::White, PieceKind::King).unwrap();
        let elephant = PieceCode::new(Color::Black, PieceKind::DrunkElephant).unwrap();
        let mut position = position_from_codes(
            Color::Black,
            &[
                (sq(0, 11), king),
                (sq(11, 0), enemy_king),
                (sq(5, 7), elephant),
            ],
        );
        assert_move_accumulator(
            &nnue,
            &mut position,
            Move {
                from: sq(5, 7),
                mid: None,
                to: sq(5, 8),
                promote: true,
            },
        );
        for victim in [
            enemy_king,
            PieceCode::new_promoted(Color::White, PieceKind::CrownPrince).unwrap(),
        ] {
            let rook = PieceCode::new(Color::Black, PieceKind::Rook).unwrap();
            let mut position = position_from_codes(
                Color::Black,
                &[(sq(0, 11), king), (sq(5, 7), rook), (sq(5, 8), victim)],
            );
            assert_move_accumulator(
                &nnue,
                &mut position,
                Move {
                    from: sq(5, 7),
                    mid: None,
                    to: sq(5, 8),
                    promote: false,
                },
            );
        }
    }
}

fn mirror(position: &Position) -> Position {
    let pieces: Vec<_> = position
        .occupied()
        .into_iter()
        .map(|square| {
            (
                sq(11 - square.file(), square.rank()),
                position.piece_at(square).unwrap(),
            )
        })
        .collect();
    let mut reflected = position_from_codes(position.side_to_move(), &pieces);
    reflected
        .set_lion_capture(
            position
                .lion_taken_by_non_lion()
                .map(|trigger| sq(11 - trigger.square.file(), trigger.square.rank())),
        )
        .unwrap();
    reflected
}

both_feature_sets!(random_games_preserve_accumulators_symmetry_and_epoch_zero_evaluation);
/// 4,096局面以上で差分、null move、取消、左右鏡映とエポック0のPST一致を検査する。
fn random_games_preserve_accumulators_symmetry_and_epoch_zero_evaluation<F: FeatureSet>() {
    let nets = networks::<F>();
    let pst =
        crate::eval::pst::Pst::decode(include_bytes!("../../../nets/nnue-epoch0-pst.bin")).unwrap();
    let generator = MoveGenerator::standard();
    let mut visited = 0;
    for seed in 1..=16 {
        let mut state = seed;
        let mut position = Position::initial();
        let mut accumulators = nets
            .each_ref()
            .map(|net| net.refresh_accumulator(&position));
        let mut children = nets.each_ref().map(|net| Accumulator::new(net.width()));
        let mut nulls = nets.each_ref().map(|net| Accumulator::new(net.width()));
        let mut history = Vec::new();
        for _ in 0..256 {
            let reflected = mirror(&position);
            for (i, net) in nets.iter().enumerate() {
                assert_eq!(accumulators[i], net.refresh_accumulator(&position));
                for side in Color::ALL {
                    assert_eq!(
                        net.evaluate_accumulator(&accumulators[i], side),
                        evaluate_for_diagnostics(net, &reflected, side)
                    );
                }
            }
            assert_eq!(
                evaluate(&nets[0], &position),
                crate::eval::pst::evaluate(&pst, &position)
            );
            visited += 1;
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            // 王駒の消失は専用フィクスチャで検査し、ここでは長い対局列を保つ。
            moves.retain(|&mv| {
                position
                    .captured_squares(mv)
                    .into_iter()
                    .flatten()
                    .all(|square| {
                        !matches!(
                            position.piece_at(square).unwrap().kind(),
                            Some(PieceKind::King | PieceKind::CrownPrince)
                        )
                    })
            });
            assert!(!moves.is_empty());
            let parents = accumulators.clone();
            let undo = position.make_move_unchecked(
                moves[next(&mut state) as usize % moves.len()],
                MoveRules::standard(),
            );
            for (i, net) in nets.iter().enumerate() {
                net.update_accumulator_after_move(
                    &accumulators[i],
                    &mut children[i],
                    &position,
                    &undo,
                );
                assert_eq!(children[i], net.refresh_accumulator(&position));
                std::mem::swap(&mut accumulators[i], &mut children[i]);
            }
            history.push((undo, parents));
            let lion = position
                .lion_taken_by_non_lion()
                .map(|trigger| trigger.square);
            let undo = position.make_null_move();
            for (i, net) in nets.iter().enumerate() {
                net.update_accumulator_after_null(&accumulators[i], &mut nulls[i], lion);
                assert_eq!(nulls[i], net.refresh_accumulator(&position));
                assert_eq!(
                    net.evaluate_accumulator(&nulls[i], position.side_to_move()),
                    evaluate(net, &position)
                );
            }
            position.unmake_null_move(undo);
        }
        for (undo, parents) in history.into_iter().rev() {
            position.unmake_move(undo);
            for (net, parent) in nets.iter().zip(&parents) {
                assert_eq!(*parent, net.refresh_accumulator(&position));
            }
            assert_eq!(
                evaluate(&nets[0], &position),
                crate::eval::pst::evaluate(&pst, &position)
            );
        }
        assert_eq!(position, Position::initial());
    }
    assert_eq!(visited, 4096);
}

#[test]
fn embedded_metadata_and_search_values_match_baseline() {
    let nnue = Nnue::decode(EMBEDDED).unwrap();
    let pst = crate::eval::pst::Pst::decode(include_bytes!("../../../nets/pst.bin")).unwrap();
    assert_eq!(nnue.pawn_value(), pst.pawn_value());
    assert_eq!(nnue.max_promotion_gain(), pst.max_promotion_gain());
    assert_eq!(nnue.k(), pst.k());
    for state in 0..PIECE_STATE_COUNT {
        assert_eq!(
            nnue.piece_value_of_state(state),
            pst.piece_value_of_state(state)
        );
    }
    assert_eq!(
        nnue.checksum(),
        &<[u8; 32]>::from(Sha256::digest(&EMBEDDED[HEADER_LENGTH..]))
    );
    let codes = minase_core::rules::parse_rule_set(nnue.rule_set()).unwrap();
    assert_eq!(
        minase_core::Rules::from_codes(&codes).unwrap(),
        minase_core::Rules::ENGINE_DEFAULT
    );
}

both_feature_sets!(header_validation_and_runtime_lengths);
fn header_validation_and_runtime_lengths<F: FeatureSet>() {
    let a = random_bytes::<F>(2);
    let b = random_bytes::<F>(4);
    assert_eq!(b.len() - a.len(), F::DIMENSIONS * 4 + 16);
    for width in [2, 4, 512] {
        let bytes = random_bytes::<F>(width);
        assert_eq!(
            Network::<F, DirectScrelu>::decode(&bytes).unwrap().width(),
            width
        );
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&extra),
            Err(Error::InvalidLength { .. })
        ));
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bytes[..bytes.len() - 1]),
            Err(Error::InvalidLength { .. })
        ));
        let mut bad = bytes.clone();
        bad[HEADER_LENGTH] ^= 1;
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::ChecksumMismatch)
        ));
    }
    for length in 0..HEADER_LENGTH {
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&a[..length]),
            Err(Error::InvalidLength { .. })
        ));
    }
    for version in [0, 1, 2, 3, 4, 6, u32::MAX] {
        let mut bad = a.clone();
        bad[4..8].copy_from_slice(&version.to_le_bytes());
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::UnsupportedVersion { .. })
        ));
    }
    for width in [0, 1, 3, 513, 514, u32::MAX] {
        let mut bad = a.clone();
        bad[104..108].copy_from_slice(&width.to_le_bytes());
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::InvalidWidth { .. })
        ));
    }
    for k in [
        0.0,
        -1.0,
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        100_001.0,
    ] {
        let mut bad = a.clone();
        bad[108..112].copy_from_slice(&k.to_le_bytes());
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::InvalidK { .. })
        ));
    }
    for k in [f32::MIN_POSITIVE, 100_000.0] {
        let mut bytes = a.clone();
        bytes[108..112].copy_from_slice(&k.to_le_bytes());
        assert!(Network::<F, DirectScrelu>::decode(&bytes).is_ok());
    }
    let mut bad = a.clone();
    bad[0] = 0;
    assert!(matches!(
        Network::<F, DirectScrelu>::decode(&bad),
        Err(Error::InvalidMagic { .. })
    ));
    bad = a.clone();
    bad[8] = b'X';
    assert!(matches!(
        Network::<F, DirectScrelu>::decode(&bad),
        Err(Error::UnexpectedArchitecture { .. })
    ));
    for offset in 176..192 {
        bad = a.clone();
        bad[offset] = 1;
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::NonZeroReserved)
        ));
    }
    for (offset, value) in [(112, 255), (143, 65)] {
        bad = a.clone();
        bad[offset] = value;
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::InvalidRuleSet)
        ));
    }
}

both_feature_sets!(parameter_ranges_and_mirror_sharing_are_validated);
fn parameter_ranges_and_mirror_sharing_are_validated<F: FeatureSet>() {
    let width = 4;
    let bytes = random_bytes::<F>(width);
    let hidden = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let bias = hidden + F::DIMENSIONS * width * 2;
    let output = bias + width * 4;
    for (offset, values) in [
        (hidden, vec![-1017_i16, 1017, i16::MIN]),
        (output, vec![-8193, 8193, i16::MIN]),
    ] {
        for value in values {
            let mut bad = bytes.clone();
            bad[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            refresh_checksum(&mut bad);
            assert!(matches!(
                Network::<F, DirectScrelu>::decode(&bad),
                Err(Error::ParameterOutOfRange { .. })
            ));
        }
    }
    for (offset, limit) in [(bias, 1016_i32), (output + width * 4, 4_161_536)] {
        for value in [-limit - 1, limit + 1, i32::MIN] {
            let mut bad = bytes.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            refresh_checksum(&mut bad);
            assert!(matches!(
                Network::<F, DirectScrelu>::decode(&bad),
                Err(Error::ParameterOutOfRange { .. })
            ));
        }
    }
    for (offset, parameter) in [
        (HEADER_LENGTH, "linear"),
        (hidden, "weights"),
        (bias, "biases"),
        (output, "output_weights"),
        (output + width * 2, "output_weights"),
    ] {
        let mut bad = bytes.clone();
        bad[offset] ^= 1;
        refresh_checksum(&mut bad);
        if parameter == "linear" || F::MIRROR == Mirror::Shared {
            assert!(
                matches!(Network::<F, DirectScrelu>::decode(&bad), Err(Error::MirrorMismatch { parameter: found, .. }) if found == parameter)
            );
        } else {
            assert!(Network::<F, DirectScrelu>::decode(&bad).is_ok());
        }
    }
    for value in [-1_i32, 29_000, i32::MAX] {
        let mut bad = bytes.clone();
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4;
        bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        refresh_checksum(&mut bad);
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::PieceValueOutOfRange { .. })
        ));
    }
    for kind in [PieceKind::King, PieceKind::CrownPrince] {
        let mut bad = bytes.clone();
        let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + kind.index() * 4;
        bad[offset..offset + 4].copy_from_slice(&0_i32.to_le_bytes());
        refresh_checksum(&mut bad);
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::InconsistentRoyalValue { .. })
        ));
    }
    let mut bad = bytes.clone();
    let offset = HEADER_LENGTH + FEATURE_COUNT * 4 + 29 * 4;
    bad[offset..offset + 4].copy_from_slice(&0_i32.to_le_bytes());
    refresh_checksum(&mut bad);
    assert!(matches!(
        Network::<F, DirectScrelu>::decode(&bad),
        Err(Error::NonPositivePieceValue { .. })
    ));
}

/// 数式の境界を手計算できる、一定の非線形入力を持つネット。
fn constant_network<F: FeatureSet>(
    width: usize,
    activation_input: i32,
    output_weight: i16,
    bias: i32,
    k: f32,
    linear: i16,
) -> Network<F, DirectScrelu> {
    let mut bytes = random_bytes::<F>(width);
    bytes[108..112].copy_from_slice(&k.to_le_bytes());
    for pair in bytes[HEADER_LENGTH..HEADER_LENGTH + FEATURE_COUNT * 4]
        .as_chunks_mut::<2>()
        .0
    {
        pair.copy_from_slice(&linear.to_le_bytes());
    }
    let hidden = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4;
    let biases = hidden + F::DIMENSIONS * width * 2;
    let output = biases + width * 4;
    bytes[hidden..biases].fill(0);
    for chunk in bytes[biases..output].as_chunks_mut::<4>().0 {
        chunk.copy_from_slice(&activation_input.to_le_bytes());
    }
    for chunk in bytes[output..output + width * 4].as_chunks_mut::<2>().0 {
        chunk.copy_from_slice(&output_weight.to_le_bytes());
    }
    bytes[output + width * 4..].copy_from_slice(&bias.to_le_bytes());
    refresh_checksum(&mut bytes);
    Network::<F, DirectScrelu>::decode(&bytes).unwrap()
}

both_feature_sets!(integer_activation_and_signed_rounding_follow_the_specification);
fn integer_activation_and_signed_rounding_follow_the_specification<F: FeatureSet>() {
    let position = Position::initial();
    // K=127/16によりscale=1。出力重み4096を4成分足すとa/4 cp。
    for (input, expected) in [
        (-1016, 0),
        (0, 0),
        (1, 0),
        (63, 8),
        (64, 8),
        (126, 31),
        (127, 32),
        (1016, 32),
    ] {
        let net = constant_network::<F>(2, input, 4096, 0, 7.9375, 0);
        assert_eq!(evaluate(&net, &position), expected);
    }
    for (bias, expected) in [(32767, 0), (32768, 1), (-32768, 0), (-32769, -1)] {
        let net = constant_network::<F>(2, 0, 0, bias, 7.9375, 0);
        assert_eq!(evaluate(&net, &position), expected);
    }
    // 0.5と1.5を偶数側に丸め、ヘッダのf32を先に倍精度へ広げる。
    for (k, expected_scale) in [(3.96875, 0), (11.90625, 2)] {
        let net = constant_network::<F>(2, 0, 0, 65536, k, 0);
        assert_eq!(net.scale, expected_scale);
        assert_eq!(evaluate(&net, &position), expected_scale as i32);
    }
}

both_feature_sets!(final_sum_is_clamped_once_and_linear_division_truncates_toward_zero);
fn final_sum_is_clamped_once_and_linear_division_truncates_toward_zero<F: FeatureSet>() {
    let pieces: Vec<_> = minase_core::Square::all()
        .take(10)
        .enumerate()
        .map(|(i, square)| {
            let (side, kind) = match i {
                0 => (Color::Black, PieceKind::King),
                1 => (Color::White, PieceKind::King),
                _ => (Color::Black, PieceKind::Pawn),
            };
            (square, PieceCode::new(side, kind).unwrap())
        })
        .collect();
    let position = position_from_codes(Color::Black, &pieces);
    // 線形は40958cp、非線形は-16128cp。線形を先に制限すると一致しない。
    let net = constant_network::<F>(512, 127, -8192, 0, 7.9375, 32767);
    assert_eq!(evaluate(&net, &position), 24_830);
    let net = constant_network::<F>(512, 127, 8192, 0, 7.9375, -32768);
    assert_eq!(evaluate(&net, &position), -24_832);
    for (linear, expected) in [(32767, 28_999), (-32768, -28_999), (-1, -1), (1, 1)] {
        let net = constant_network::<F>(2, 0, 0, 0, 7.9375, linear);
        assert_eq!(evaluate(&net, &position), expected);
    }
}

#[cfg(feature = "invariants")]
#[test]
fn accumulator_invariant_reports_a_mismatch() {
    let net = Nnue::decode(EMBEDDED).unwrap();
    let position = Position::initial();
    let mut accumulator = net.refresh_accumulator(&position);
    net.assert_accumulator(&position, &accumulator, 7);
    accumulator.sums[0][0] += 1;
    assert!(
        std::panic::catch_unwind(|| net.assert_accumulator(&position, &accumulator, 7)).is_err()
    );
}

/// 手番による出力行の交換と先獅子の保持を、独立した6cp/8cpの期待値で固定する。
#[test]
fn diagnostic_views_select_both_output_rows_without_removing_lion() {
    use crate::eval::features::lion_feature_index;
    let mut net = constant_network::<P>(2, 0, 0, 0, 7.9375, 0);
    let mut position = position_from_codes(
        Color::Black,
        &[
            (
                sq(0, 11),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            ),
            (
                sq(11, 0),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            ),
        ],
    );
    position.set_lion_capture(Some(sq(5, 3))).unwrap();
    // 黒視点の先獅子だけで各ユニット64になり、活性化は32。
    let feature = lion_feature_index(Color::Black, sq(5, 3));
    for f in [feature, mirror_feature(feature)] {
        net.weights[f * 2..f * 2 + 2].fill(64);
    }
    net.head.weights.copy_from_slice(&[4096, 4096, 8192, 8192]);
    // 黒視点の線形成分は8/16。駒数2なので終盤の16/8=2cp。
    for f in [feature, mirror_feature(feature)] {
        net.linear[f] = [8, 16];
    }
    assert_eq!(evaluate_for_diagnostics(&net, &position, Color::Black), 6);
    assert_eq!(evaluate_for_diagnostics(&net, &position, Color::White), 8);
    assert_eq!(position.lion_taken_by_non_lion().unwrap().square, sq(5, 3));
}

/// 絶対升の番号、基準の王駒の優先順位と例外を仕様の値で固定する。
#[test]
fn half_kp_anchor_selection_and_dense_indices_follow_the_contract() {
    use minase_core::Square;
    for square in Square::all() {
        assert_eq!(
            square.dense_index(),
            usize::from(square.rank()) * 12 + usize::from(square.file())
        );
    }
    for side in Color::ALL {
        let king = PieceCode::new(side, PieceKind::King).unwrap();
        let prince = PieceCode::new_promoted(side, PieceKind::CrownPrince).unwrap();
        // 後手でも絶対段の小さい王駒が優先される。
        let cases = [
            (
                vec![(sq(10, 0), king), (sq(1, 1), prince)],
                [(1, false), (7, false)],
            ),
            (vec![(sq(1, 1), prince)], [(6, true), (7, true)]),
            (
                vec![(sq(10, 0), king), (sq(1, 1), king)],
                [(1, false), (7, false)],
            ),
            (
                vec![(sq(10, 0), prince), (sq(1, 1), prince)],
                [(1, false), (7, false)],
            ),
            (
                vec![(sq(3, 0), king), (sq(10, 0), king)],
                [(3, true), (7, true)],
            ),
            (vec![], [(0, false), (0, false)]),
        ];
        for (pieces, expected) in cases {
            let position = position_from_codes(side, &pieces);
            let (bucket, mirrored) = expected[side.index()];
            assert_eq!(
                HalfKpHmB8::context(&position, side),
                HalfKpContext { bucket, mirrored }
            );
            assert_eq!(
                HalfKpHmB8::context(&position, side.opposite()),
                HalfKpContext {
                    bucket: 0,
                    mirrored: false
                }
            );
        }
    }
    assert_eq!(std::mem::size_of::<<P as FeatureSet>::Context>(), 0);
}

/// 8区分と中央の鏡映境界を、両視点の手作り局面で検査する。
#[test]
fn half_kp_buckets_and_feature_indices_follow_the_contract() {
    let first_rank = [0, 1, 2, 3, 4, 5, 5, 4, 3, 2, 1, 0];
    for side in Color::ALL {
        for (relative_rank, buckets) in [(0, first_rank), (1, [6; 12]), (2, [7; 12]), (11, [7; 12])]
        {
            for (file, bucket) in buckets.into_iter().enumerate() {
                let rank = if side == Color::Black {
                    relative_rank
                } else {
                    11 - relative_rank
                };
                let position = position_from_codes(
                    side,
                    &[(
                        sq(file as u8, rank),
                        PieceCode::new(side, PieceKind::King).unwrap(),
                    )],
                );
                let context = HalfKpHmB8::context(&position, side);
                assert_eq!(
                    context,
                    HalfKpContext {
                        bucket,
                        mirrored: file < 6
                    }
                );
            }
        }
    }
    // 先獅子も同じ鏡映で写す。基底特徴13536は相対升(0,0)。
    for (context, base, expected) in [
        (
            HalfKpContext {
                bucket: 0,
                mirrored: false,
            },
            0,
            0,
        ),
        (
            HalfKpContext {
                bucket: 5,
                mirrored: true,
            },
            0,
            68_411,
        ),
        (
            HalfKpContext {
                bucket: 6,
                mirrored: true,
            },
            13_536,
            95_627,
        ),
        (
            HalfKpContext {
                bucket: 7,
                mirrored: false,
            },
            13_679,
            109_439,
        ),
    ] {
        assert_eq!(HalfKpHmB8::index(context, base), expected);
    }
    for base in 0..13_680 {
        assert_eq!(P::index((), base), base);
        assert_eq!(P::base_index(base), base);
        for bucket in 0..8 {
            assert_eq!(HalfKpHmB8::base_index(bucket * 13_680 + base), base);
        }
    }
}

both_feature_sets!(royal_context_changes_preserve_incremental_evaluation);
fn royal_context_changes_preserve_incremental_evaluation<F: FeatureSet>() {
    let net = Network::<F, DirectScrelu>::decode(&random_bytes::<F>(4)).unwrap();
    for side in Color::ALL {
        let king = PieceCode::new(side, PieceKind::King).unwrap();
        let enemy_king = PieceCode::new(side.opposite(), PieceKind::King).unwrap();
        let rank = |r| if side == Color::Black { r } else { 11 - r };
        for (from, to) in [
            (sq(5, rank(0)), sq(6, rank(0))), // 鏡映だけが変わる。
            (sq(6, rank(0)), sq(7, rank(0))), // 最下段の区分が変わる。
            (sq(6, rank(0)), sq(6, rank(1))),
            (sq(6, rank(1)), sq(6, rank(2))),
            (sq(6, rank(2)), sq(7, rank(3))), // 王が動いても文脈は不変。
        ] {
            let mut position =
                position_from_codes(side, &[(from, king), (sq(11, rank(11)), enemy_king)]);
            assert_move_accumulator(
                &net,
                &mut position,
                Move {
                    from,
                    to,
                    mid: None,
                    promote: false,
                },
            );
        }
        let prince = PieceCode::new_promoted(side.opposite(), PieceKind::CrownPrince).unwrap();
        let rook = PieceCode::new(side, PieceKind::Rook).unwrap();
        // 王将を失った側の太子が基準になる場合と、王駒がなくなる場合。
        for survivor in [None, Some((sq(6, rank(11)), prince))] {
            let mut pieces = vec![
                (sq(0, rank(0)), king),
                (sq(5, rank(7)), rook),
                (sq(5, rank(8)), enemy_king),
            ];
            pieces.extend(survivor);
            let mut position = position_from_codes(side, &pieces);
            assert_move_accumulator(
                &net,
                &mut position,
                Move {
                    from: sq(5, rank(7)),
                    to: sq(5, rank(8)),
                    mid: None,
                    promote: false,
                },
            );
        }
        // 王将が存在しない側に太子が生じ、初めて文脈を持つ。
        let elephant = PieceCode::new(side, PieceKind::DrunkElephant).unwrap();
        let mut position = position_from_codes(
            side,
            &[(sq(5, rank(7)), elephant), (sq(11, rank(11)), enemy_king)],
        );
        assert_move_accumulator(
            &net,
            &mut position,
            Move {
                from: sq(5, rank(7)),
                to: sq(5, rank(8)),
                mid: None,
                promote: true,
            },
        );
        // 太子だけを持つ側では、その移動も鏡映を変える。
        let prince = PieceCode::new_promoted(side, PieceKind::CrownPrince).unwrap();
        let mut position = position_from_codes(
            side,
            &[(sq(5, rank(0)), prince), (sq(11, rank(11)), enemy_king)],
        );
        assert_move_accumulator(
            &net,
            &mut position,
            Move {
                from: sq(5, rank(0)),
                to: sq(6, rank(0)),
                mid: None,
                promote: false,
            },
        );
    }
}

/// 区分へのP型の行の複写は、鏡映のユニット交換も含めて整数出力を保存する。
#[test]
fn copied_p_rows_produce_identical_half_kp_evaluations() {
    let bytes = random_bytes::<P>(8);
    let p = Network::<P, DirectScrelu>::decode(&bytes).unwrap();
    let half_kp =
        Network::<HalfKpHmB8, DirectScrelu>::decode(&copy_p_rows::<HalfKpHmB8>(&bytes)).unwrap();
    let mut state = 917;
    for rank in 0..12 {
        for file in 0..12 {
            let mut pieces = vec![
                (
                    sq(file, rank),
                    PieceCode::new(Color::Black, PieceKind::King).unwrap(),
                ),
                (
                    sq(11 - file, 11 - rank),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
            ];
            for square in minase_core::Square::all() {
                if pieces.iter().any(|&(s, _)| s == square) || next(&mut state).is_multiple_of(3) {
                    continue;
                }
                let side = Color::ALL[next(&mut state) as usize % 2];
                let kind = [PieceKind::Pawn, PieceKind::Rook, PieceKind::Lion]
                    [next(&mut state) as usize % 3];
                pieces.push((square, PieceCode::new(side, kind).unwrap()));
            }
            let mut position = position_from_codes(Color::Black, &pieces);
            position
                .set_lion_capture(Some(sq(11 - file, 11 - rank)))
                .unwrap();
            for side in Color::ALL {
                assert_eq!(
                    evaluate_for_diagnostics(&p, &position, side),
                    evaluate_for_diagnostics(&half_kp, &position, side)
                );
            }
        }
    }
}

both_feature_sets!(architecture_must_match_type_and_header_width);
fn architecture_must_match_type_and_header_width<F: FeatureSet>() {
    let bytes = random_bytes::<F>(4);
    for text in [
        format!(
            "features={};ft=2;linear=tapered2;head=direct-screlu",
            F::NAME
        ),
        format!("features={};ft=4;linear=tapered2;head=unknown", F::NAME),
        format!(
            "features={};ft=4;linear=unknown;head=direct-screlu",
            F::NAME
        ),
        "features=unknown;ft=4;linear=tapered2;head=direct-screlu".to_owned(),
        String::new(),
    ] {
        let mut bad = bytes.clone();
        bad[8..104].fill(0);
        bad[8..8 + text.len()].copy_from_slice(text.as_bytes());
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::UnexpectedArchitecture { .. })
        ));
    }
    let mut bad = bytes.clone();
    bad[104..108].copy_from_slice(&2_u32.to_le_bytes());
    assert!(matches!(
        Network::<F, DirectScrelu>::decode(&bad),
        Err(Error::UnexpectedArchitecture { .. })
    ));
    for offset in [8, 103] {
        let mut bad = bytes.clone();
        bad[offset] = 255;
        assert!(matches!(
            Network::<F, DirectScrelu>::decode(&bad),
            Err(Error::InvalidArchitecture)
        ));
    }
    let mut bad = bytes.clone();
    bad[103] = b'x';
    assert!(matches!(
        Network::<F, DirectScrelu>::decode(&bad),
        Err(Error::InvalidArchitecture)
    ));
    // 他方の型を明示して復号しても受理しない。
    if F::NAME == "P" {
        assert!(matches!(
            Network::<HalfKpHmB8, DirectScrelu>::decode(&bytes),
            Err(Error::UnexpectedArchitecture { .. })
        ));
    } else {
        assert!(matches!(
            Nnue::decode(&bytes),
            Err(Error::UnexpectedArchitecture { .. })
        ));
    }
}

#[cfg(feature = "invariants")]
#[test]
fn accumulator_invariant_includes_contexts() {
    let net = Network::<HalfKpHmB8, DirectScrelu>::decode(&random_bytes::<HalfKpHmB8>(4)).unwrap();
    let position = Position::initial();
    let mut accumulator = net.refresh_accumulator(&position);
    net.assert_accumulator(&position, &accumulator, 0);
    accumulator.contexts[0].mirrored = !accumulator.contexts[0].mirrored;
    assert!(
        std::panic::catch_unwind(|| net.assert_accumulator(&position, &accumulator, 0)).is_err()
    );
}

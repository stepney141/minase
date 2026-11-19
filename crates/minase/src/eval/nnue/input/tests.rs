use super::*;
use crate::eval::features::{FEATURE_COUNT, feature_index};
use Color::{Black, White};
use PieceKind::*;
use minase_core::test_util::{
    bench_positions, position_from_codes, reflect_ranks_and_swap_colors, sampled_random_positions,
    sq,
};
use minase_core::{MoveRules, PieceCode, PieceKind};

fn piece(color: Color, kind: PieceKind) -> PieceCode {
    if kind == CrownPrince {
        PieceCode::new_promoted(color, kind).unwrap()
    } else {
        PieceCode::new(color, kind).unwrap()
    }
}

fn rows<I: Indexer>(position: &Position, perspective: Color) -> Result<Vec<usize>, AnchorError> {
    let context = I::context(position, perspective)?;
    let mut rows = Vec::new();
    for_each_active_row::<I>(context, perspective, position, |row| rows.push(row));
    rows.sort_unstable();
    Ok(rows)
}

#[test]
fn specifications_match_phase_one_contract() {
    assert_eq!(FeatureSet::ALL, [FeatureSet::P, FeatureSet::HalfKpVm]);
    for (set, name, buckets, inputs, dimensions, hash, mirror, factorized) in [
        (
            FeatureSet::P,
            "P",
            1,
            13_680,
            13_680,
            0xD50C118F,
            Mirror::HalfWidthRotation,
            false,
        ),
        (
            FeatureSet::HalfKpVm,
            "HalfKPVm",
            72,
            13_392,
            964_224,
            0x0AE9857A,
            Mirror::KingFile,
            true,
        ),
    ] {
        let spec = set.spec();
        assert_eq!(spec.feature_set(), set);
        assert_eq!(spec.arch_name(), name);
        assert_eq!(spec.king_buckets(), buckets);
        assert_eq!(spec.piece_inputs(), inputs);
        assert_eq!(spec.dimensions(), dimensions);
        assert_eq!(spec.max_active(), 145);
        assert_eq!(spec.feature_hash(), hash);
        assert_eq!(spec.mirror(), mirror);
        assert_eq!(spec.factorized(), factorized);
        assert_eq!(FeatureSet::from_arch_name(name), Some(set));
    }
    for name in ["p", "HalfKP", ""] {
        assert_eq!(FeatureSet::from_arch_name(name), None);
    }
}

#[test]
fn p_preserves_every_base_feature_without_an_anchor() {
    let empty = position_from_codes(Black, &[]);
    assert_eq!(P::check_search_root(&empty), Ok(()));
    for perspective in Color::ALL {
        assert_eq!(P::context(&empty, perspective), Ok(()));
    }
    for base in 0..FEATURE_COUNT {
        assert_eq!(P::row((), base), Some(base));
    }
}

#[test]
#[should_panic(expected = "base feature out of range")]
fn p_rejects_out_of_range_base() {
    P::row((), 13_680);
}

#[test]
#[should_panic(expected = "base feature out of range")]
fn half_kp_vm_rejects_out_of_range_base() {
    HalfKpVm::row(
        HalfKpVm::context(&Position::initial(), Black).unwrap(),
        13_680,
    );
}

#[test]
fn hand_calculated_positions_with_king_prince_and_lion_trigger() {
    // 指示書6.2の独立に計算した行番号をそのまま固定する。
    let mut a = position_from_codes(
        Black,
        &[
            (sq(3, 0), piece(Black, King)),
            (sq(4, 1), piece(Black, CrownPrince)),
            (sq(2, 3), piece(Black, Pawn)),
            (sq(9, 10), piece(White, King)),
            (sq(7, 9), piece(White, GoldGeneral)),
        ],
    );
    a.set_lion_capture(Some(sq(5, 5))).unwrap();
    let b = position_from_codes(
        Black,
        &[
            (sq(6, 2), piece(Black, CrownPrince)),
            (sq(2, 3), piece(Black, Pawn)),
            (sq(9, 10), piece(White, King)),
            (sq(7, 9), piece(White, GoldGeneral)),
        ],
    );
    for (position, perspective, bucket, mirrored, expected) in [
        (&a, Black, 3, true, vec![43075, 44253, 53104, 53490]),
        (&a, White, 8, false, vec![113359, 116764, 117890, 120461]),
        (&b, Black, 17, false, vec![230574, 231734, 240595]),
        (&b, White, 8, false, vec![113359, 116754, 117890]),
    ] {
        let context = HalfKpVm::context(position, perspective).unwrap();
        assert_eq!((context.bucket(), context.mirrored()), (bucket, mirrored));
        assert_eq!(rows::<HalfKpVm>(position, perspective).unwrap(), expected);
    }
}

#[test]
fn central_boundary_keeps_bucket_and_swaps_piece_rows() {
    for (king_file, mirrored, pawn_rows) in
        [(5, true, [392445, 392438]), (6, false, [392438, 392445])]
    {
        let position = position_from_codes(
            Black,
            &[
                (sq(king_file, 4), piece(Black, King)),
                (sq(9, 10), piece(White, King)),
                (sq(2, 3), piece(Black, Pawn)),
                (sq(9, 3), piece(Black, Pawn)),
            ],
        );
        let context = HalfKpVm::context(&position, Black).unwrap();
        assert_eq!((context.bucket(), context.mirrored()), (29, mirrored));
        for (file, expected) in [2, 9].into_iter().zip(pawn_rows) {
            assert_eq!(
                HalfKpVm::row(
                    context,
                    feature_index(Black, piece(Black, Pawn), sq(file, 3))
                ),
                Some(expected)
            );
        }
        assert_eq!(
            rows::<HalfKpVm>(&position, Black).unwrap(),
            [392438, 392445]
        );
        let white = HalfKpVm::context(&position, White).unwrap();
        assert_eq!((white.bucket(), white.mirrored()), (8, false));
        assert_eq!(
            rows::<HalfKpVm>(&position, White).unwrap(),
            [117890, 117897]
        );
    }
}

#[test]
fn hand_calculated_promoted_and_unpromotable_piece_rows() {
    let position = position_from_codes(
        Black,
        &[
            (sq(3, 0), piece(Black, King)),
            (sq(9, 10), piece(White, King)),
        ],
    );
    for (perspective, code, square, expected) in [
        (
            Black,
            PieceCode::new_promoted(Black, GoldGeneral).unwrap(),
            sq(0, 0),
            42491,
        ),
        (
            Black,
            PieceCode::new_promoted(White, Lion).unwrap(),
            sq(11, 11),
            49668,
        ),
        (Black, piece(White, FreeKing), sq(6, 6), 48317),
        (White, piece(Black, Pawn), sq(0, 11), 117792),
    ] {
        let context = HalfKpVm::context(&position, perspective).unwrap();
        assert_eq!(
            HalfKpVm::row(context, feature_index(perspective, code, square)),
            Some(expected)
        );
    }
}

#[test]
fn article_20_anchor_errors_and_root_candidate_checks() {
    assert_eq!(HalfKpVm::check_search_root(&Position::initial()), Ok(()));
    // 両陣営で欠落・重複を検査する。王将がある間の太子重複は文脈だけなら一意。
    for color in Color::ALL {
        let other = (sq(11, 11), piece(color.opposite(), King));
        for (own, context_error, root_error) in [
            (
                vec![],
                AnchorError::MissingRoyal { color },
                AnchorError::MissingRoyal { color },
            ),
            (
                vec![
                    (sq(0, 0), piece(color, King)),
                    (sq(1, 0), piece(color, King)),
                ],
                AnchorError::MultipleKings { color },
                AnchorError::MultipleKings { color },
            ),
            (
                vec![
                    (sq(0, 0), piece(color, CrownPrince)),
                    (sq(1, 0), piece(color, CrownPrince)),
                ],
                AnchorError::MultiplePrinces { color },
                AnchorError::PrinceCandidates { color },
            ),
        ] {
            let mut pieces = own;
            pieces.push(other);
            let position = position_from_codes(Black, &pieces);
            assert_eq!(HalfKpVm::context(&position, color), Err(context_error));
            assert_eq!(HalfKpVm::check_search_root(&position), Err(root_error));
        }
        for promoted in [false, true] {
            let elephant = if promoted {
                PieceCode::new_promoted(color, DrunkElephant).unwrap()
            } else {
                piece(color, DrunkElephant)
            };
            let position = position_from_codes(
                Black,
                &[
                    other,
                    (sq(0, 0), piece(color, King)),
                    (sq(1, 0), piece(color, CrownPrince)),
                    (sq(2, 0), elephant),
                ],
            );
            assert!(HalfKpVm::context(&position, color).is_ok());
            assert_eq!(
                HalfKpVm::check_search_root(&position),
                if promoted {
                    Ok(())
                } else {
                    Err(AnchorError::PrinceCandidates { color })
                }
            );
        }
        let position = position_from_codes(
            Black,
            &[
                other,
                (sq(0, 0), piece(color, King)),
                (sq(1, 0), piece(color, CrownPrince)),
                (sq(2, 0), piece(color, CrownPrince)),
            ],
        );
        assert!(HalfKpVm::context(&position, color).is_ok());
        assert_eq!(
            HalfKpVm::check_search_root(&position),
            Err(AnchorError::PrinceCandidates { color })
        );
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

fn swap_error_color(error: AnchorError) -> AnchorError {
    match error {
        AnchorError::MissingRoyal { color } => AnchorError::MissingRoyal {
            color: color.opposite(),
        },
        AnchorError::MultipleKings { color } => AnchorError::MultipleKings {
            color: color.opposite(),
        },
        AnchorError::MultiplePrinces { color } => AnchorError::MultiplePrinces {
            color: color.opposite(),
        },
        AnchorError::PrinceCandidates { color } => AnchorError::PrinceCandidates {
            color: color.opposite(),
        },
    }
}

fn assert_rank_symmetry<I: Indexer>(position: &Position) {
    let reflected = reflect_ranks_and_swap_colors(position);
    assert_eq!(
        rows::<I>(&reflected, Black),
        rows::<I>(position, White).map_err(swap_error_color)
    );
}

#[test]
fn feature_symmetries_hold_for_bench_random_and_missing_royal_positions() {
    let mut positions = bench_positions();
    positions.extend(sampled_random_positions(MoveRules::standard()));
    positions.push(position_from_codes(
        Black,
        &[(sq(0, 0), piece(Black, King))],
    ));
    positions.push(position_from_codes(
        White,
        &[(sq(11, 11), piece(White, CrownPrince))],
    ));
    for position in positions {
        let reflected = mirror(&position);
        for perspective in Color::ALL {
            assert_eq!(
                rows::<HalfKpVm>(&position, perspective),
                rows::<HalfKpVm>(&reflected, perspective)
            );
        }
        assert_rank_symmetry::<P>(&position);
        assert_rank_symmetry::<HalfKpVm>(&position);
    }
}

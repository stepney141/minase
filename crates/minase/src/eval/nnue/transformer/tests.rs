use super::*;
use crate::eval::nnue::input::{A, HalfKpVm};
use Color::{Black, White};
use FinnyRefresh::{Hit, Miss};
use PieceKind::*;
use minase_core::rng::XorShift64;
use minase_core::test_util::{position_from_codes, reflect_ranks_and_swap_colors, sq};
use minase_core::{Move, MoveGenerator, MoveRules, PieceCode, PieceKind};
use std::num::NonZeroU64;

fn piece(color: Color, kind: PieceKind) -> PieceCode {
    if kind == CrownPrince {
        PieceCode::new_promoted(color, kind).unwrap()
    } else {
        PieceCode::new(color, kind).unwrap()
    }
}

fn transformer<I: Indexer>(width: usize) -> FeatureTransformer<I> {
    let mut rng = XorShift64::new(NonZeroU64::new(0x7461_7461_7261).unwrap());
    let mut value = || (rng.next() % 129) as i16 - 64;
    let biases = (0..width).map(|_| value()).collect();
    let weights = (0..I::FEATURE_SET.spec().dimensions() * width)
        .map(|_| value())
        .collect();
    FeatureTransformer::new(width, biases, weights).unwrap()
}

fn mv(from: (u8, u8), to: (u8, u8)) -> Move {
    Move {
        from: sq(from.0, from.1),
        to: sq(to.0, to.1),
        mid: None,
        promote: false,
    }
}

fn assert_null<I: Indexer + PartialEq + fmt::Debug>(
    net: &FeatureTransformer<I>,
    position: &mut Position,
    before: &FtAccumulator<I>,
) {
    let lion_before = position
        .lion_taken_by_non_lion()
        .map(|trigger| trigger.square);
    let undo = position.make_null_move();
    // 使用済み領域へ書き、古い値を残さないことも検査する。
    let mut after = net.refresh(&Position::initial());
    let pointers = after.rows.each_ref().map(|row| row.as_ptr());
    net.update_after_null(before, &mut after, lion_before);
    assert_eq!(after, net.refresh(position));
    assert_eq!(pointers, after.rows.each_ref().map(|row| row.as_ptr()));
    position.unmake_null_move(undo);
    assert_eq!(*before, net.refresh(position));
}

/// 合法手であることを確かめてから指す。
fn make_legal_move(position: &mut Position, mv: Move) -> Undo {
    let rules = MoveRules::standard();
    let mut moves = Vec::new();
    MoveGenerator::new(rules).generate_moves(position, &mut moves);
    assert!(moves.contains(&mv), "fixture move must be legal: {mv:?}");
    position.make_move_unchecked(mv, rules)
}

fn assert_move<I: Indexer + PartialEq + fmt::Debug>(
    net: &FeatureTransformer<I>,
    position: &mut Position,
    mv: Move,
    expected_refresh: bool,
) {
    let before = net.refresh(position);
    let undo = make_legal_move(position, mv);
    let mut after = net.refresh(&Position::initial());
    let pointers = after.rows.each_ref().map(|row| row.as_ptr());
    assert_eq!(
        net.update_after_move(&before, &mut after, position, &undo),
        expected_refresh
    );
    assert_eq!(after, net.refresh(position));
    assert_eq!(pointers, after.rows.each_ref().map(|row| row.as_ptr()));
    for side in Color::ALL {
        assert_eq!(after.context(side), I::context(position, side).ok());
        if after.context(side).is_none() {
            assert_eq!(after.row(side), None);
            assert!(after.rows[side.index()].iter().all(|&value| value == 0));
        } else {
            assert_eq!(after.row(side), Some(after.rows[side.index()].as_ref()));
        }
    }
    assert_null(net, position, &after);
}

fn move_shapes<I: Indexer + PartialEq + fmt::Debug>(
    net: &FeatureTransformer<I>,
    anchor_refresh: bool,
) {
    let kings = [
        (sq(0, 11), piece(Black, King)),
        (sq(11, 0), piece(White, King)),
    ];
    let mut cases = vec![
        (
            "article_9_pawn",
            vec![(sq(5, 5), piece(Black, Pawn))],
            mv((5, 5), (5, 6)),
            false,
        ),
        (
            "article_18_promotion",
            vec![
                (sq(5, 7), piece(Black, Pawn)),
                (sq(5, 8), piece(White, Pawn)),
            ],
            Move {
                promote: true,
                ..mv((5, 7), (5, 8))
            },
            false,
        ),
        (
            "article_12_double_capture",
            vec![
                (sq(5, 5), piece(Black, Lion)),
                (sq(5, 4), piece(White, Pawn)),
                (sq(5, 3), piece(White, Pawn)),
            ],
            Move {
                mid: Some(sq(5, 4)),
                ..mv((5, 5), (5, 3))
            },
            false,
        ),
        (
            "article_12_igui",
            vec![
                (sq(5, 5), piece(Black, Lion)),
                (sq(5, 4), piece(White, Pawn)),
            ],
            Move {
                mid: Some(sq(5, 4)),
                ..mv((5, 5), (5, 5))
            },
            false,
        ),
        (
            "article_12_jitto",
            vec![(sq(5, 5), piece(Black, Lion))],
            Move {
                mid: None,
                ..mv((5, 5), (5, 5))
            },
            false,
        ),
        (
            "article_9_king",
            vec![],
            mv((0, 11), (1, 10)),
            anchor_refresh,
        ),
        (
            "article_10_prince_with_king",
            vec![(sq(5, 5), piece(Black, CrownPrince))],
            mv((5, 5), (5, 4)),
            false,
        ),
        (
            "article_18_elephant_with_king",
            vec![(sq(5, 7), piece(Black, DrunkElephant))],
            Move {
                promote: true,
                ..mv((5, 7), (5, 8))
            },
            false,
        ),
    ];
    for (_, pieces, _, _) in &mut cases {
        pieces.extend(kings);
    }
    cases.push((
        "article_9_central_boundary",
        vec![
            (sq(5, 4), piece(Black, King)),
            (sq(11, 0), piece(White, King)),
            (sq(2, 3), piece(Black, Pawn)),
            (sq(9, 3), piece(Black, Pawn)),
        ],
        mv((5, 4), (6, 4)),
        anchor_refresh,
    ));
    for has_prince in [false, true] {
        let mut pieces = vec![
            (sq(0, 11), piece(Black, King)),
            (sq(5, 5), piece(Black, Rook)),
            (sq(5, 3), piece(White, King)),
        ];
        if has_prince {
            pieces.push((sq(9, 0), piece(White, CrownPrince)));
        }
        cases.push((
            "article_20_capture_king",
            pieces,
            mv((5, 5), (5, 3)),
            has_prince && anchor_refresh,
        ));
    }
    for (name, pieces, mv, refresh) in cases {
        let original = position_from_codes(Black, &pieces);
        // 両陣営の文脈遷移を同じ契約で検査する。
        for reflected in [false, true] {
            let mut position = if reflected {
                reflect_ranks_and_swap_colors(&original)
            } else {
                original.clone()
            };
            let reflect = |square: Square| sq(square.file(), 11 - square.rank());
            let mv = if reflected {
                Move {
                    from: reflect(mv.from),
                    to: reflect(mv.to),
                    mid: mv.mid.map(reflect),
                    promote: mv.promote,
                }
            } else {
                mv
            };
            assert!(I::check_search_root(&position).is_ok(), "{name}");
            assert_move(net, &mut position, mv, refresh);
        }
    }
    let mut position = position_from_codes(
        Black,
        &[
            kings[0],
            kings[1],
            (sq(5, 5), piece(Black, Rook)),
            (sq(5, 3), piece(White, Lion)),
            (sq(9, 3), piece(White, Pawn)),
        ],
    );
    let before = net.refresh(&position);
    assert_null(net, &mut position, &before);
    assert_move(net, &mut position, mv((5, 5), (5, 3)), false);
    assert!(position.lion_taken_by_non_lion().is_some());
    // 応手は王駒を動かさず、先獅子の消滅だけを差分で扱う。
    assert_move(net, &mut position, mv((9, 3), (9, 2)), false);
    assert!(position.lion_taken_by_non_lion().is_none());
}

fn random_games<I: Indexer + PartialEq + fmt::Debug>(net: &FeatureTransformer<I>) {
    let rules = MoveRules::standard();
    let generator = MoveGenerator::new(rules);
    for seed in [0x1234, 0x5678, 0x9abc, 0xdef0] {
        let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
        let mut position = Position::initial();
        let mut before = net.refresh(&position);
        let mut after = FtAccumulator::new(net.width);
        let mut moves = Vec::new();
        for ply in 0..256 {
            if Color::ALL
                .into_iter()
                .any(|color| position.royal_pieces(color).is_empty())
            {
                break;
            }
            moves.clear();
            generator.generate_moves(&position, &mut moves);
            if moves.is_empty() {
                break;
            }
            // 棄却法で剰余による偏りを除き、合法手から一様に選ぶ。
            let count = moves.len() as u64;
            let threshold = count.wrapping_neg() % count;
            let choice = loop {
                let value = rng.next();
                if value >= threshold {
                    break (value % count) as usize;
                }
            };
            let undo = position.make_move_unchecked(moves[choice], rules);
            net.update_after_move(&before, &mut after, &position, &undo);
            assert_eq!(after, net.refresh(&position), "seed={seed}, ply={ply}");
            if (ply + 1) % 7 == 0 {
                assert_null(net, &mut position, &after);
            }
            std::mem::swap(&mut before, &mut after);
        }
    }
}

#[test]
fn a_updates_match_refresh_for_move_shapes_and_four_random_games() {
    let net = transformer::<A>(32);
    move_shapes(&net, false);
    random_games(&net);
}

#[test]
fn half_kp_vm_updates_match_refresh_for_move_shapes_and_four_random_games() {
    let net = transformer::<HalfKpVm>(16);
    move_shapes(&net, true);
    random_games(&net);
}

/// 根の全再計算の結果を保存した表を作る。
fn rooted_finny(net: &FeatureTransformer<HalfKpVm>, root: &Position) -> FinnyTable {
    let mut table = FinnyTable::new(net.width());
    table.save_root(root, &net.refresh(root));
    table
}

/// 表を使う着手後の両視点の文脈と全成分、および表で足し引きした行数を照合する。
fn assert_finny_move(
    net: &FeatureTransformer<HalfKpVm>,
    table: &mut FinnyTable,
    position: &mut Position,
    mv: Move,
    expected: [Option<FinnyRefresh>; 2],
) -> FtAccumulator<HalfKpVm> {
    let before = net.refresh(position);
    let mut after = before.clone();
    let undo = make_legal_move(position, mv);
    assert_eq!(
        net.update_after_move_cached(&before, &mut after, position, &undo, table),
        expected.iter().any(Option::is_some),
        "{mv:?}"
    );
    assert_eq!(after, net.refresh(position), "{mv:?}");
    assert_eq!(table.last_refresh, expected, "{mv:?}");
    after
}

/// 探索の枝を行き来し、同じ項目へ初訪問、別の配置、同じ配置または元の配置の順で入る。
fn assert_finny_revisits(
    net: &FeatureTransformer<HalfKpVm>,
    root: &Position,
    visits: [(&Position, Move); 2],
    changed_rows: [Option<usize>; 2],
) {
    for (third, third_rows) in [
        (visits[1], changed_rows.map(|rows| rows.map(|_| 0))),
        (visits[0], changed_rows),
    ] {
        let mut table = rooted_finny(net, root);
        for ((parent, mv), expected) in [
            (visits[0], changed_rows.map(|rows| rows.map(|_| Miss))),
            (visits[1], changed_rows.map(|rows| rows.map(Hit))),
            (third, third_rows.map(|rows| rows.map(Hit))),
        ] {
            assert_finny_move(net, &mut table, &mut parent.clone(), mv, expected);
        }
    }
}

/// 両視点で区分変更と鏡映だけの変更を区別し、根の項目も再利用する。
#[test]
fn finny_keys_keep_buckets_mirrors_and_perspectives_separate() {
    let net = transformer::<HalfKpVm>(4);
    let root = position_from_codes(
        Black,
        &[
            (sq(5, 2), piece(Black, King)),
            (sq(5, 9), piece(White, King)),
            (sq(2, 4), piece(Black, Pawn)),
            (sq(9, 7), piece(White, Pawn)),
        ],
    );
    let mut changed = root.clone();
    make_legal_move(&mut changed, mv((2, 4), (2, 5)));
    changed.make_null_move();
    for (root, changed, rank) in [
        (root.clone(), changed.clone(), 2),
        (
            reflect_ranks_and_swap_colors(&root),
            reflect_ranks_and_swap_colors(&changed),
            9,
        ),
    ] {
        let side = root.side_to_move();
        assert_eq!(
            HalfKpVm::context(&root, Black),
            HalfKpVm::context(&root, White)
        );
        for file in [4, 6] {
            let step = mv((5, rank), (file, rank));
            let mut rows = [None; 2];
            rows[side.index()] = Some(2);
            assert_finny_revisits(&net, &root, [(&root, step), (&changed, step)], rows);
        }

        let mut table = rooted_finny(&net, &root);
        let mut position = root;
        let mut expected = [None; 2];
        expected[side.index()] = Some(Miss);
        assert_finny_move(
            &net,
            &mut table,
            &mut position,
            mv((5, rank), (6, rank)),
            expected,
        );
        expected = [None; 2];
        expected[side.opposite().index()] = Some(Miss);
        assert_finny_move(
            &net,
            &mut table,
            &mut position,
            mv((5, 11 - rank), (6, 11 - rank)),
            expected,
        );
        // 相手の王将だけが根と異なり、足し引きする行はない。
        expected = [None; 2];
        expected[side.index()] = Some(Hit(0));
        assert_finny_move(
            &net,
            &mut table,
            &mut position,
            mv((6, rank), (5, rank)),
            expected,
        );
    }
}

/// 占有升が同じでも、成りと捕獲による駒コードの違いを更新する。
#[test]
fn finny_detects_promotions_and_captures_without_occupancy_changes() {
    let net = transformer::<HalfKpVm>(4);
    let kings = [
        (sq(5, 1), piece(Black, King)),
        (sq(10, 10), piece(White, King)),
    ];
    let king_step = mv((10, 10), (9, 10));
    let root = position_from_codes(
        Black,
        &[
            kings[0],
            kings[1],
            (sq(2, 7), piece(Black, DrunkElephant)),
            (sq(2, 8), piece(White, Pawn)),
        ],
    );
    let mut table = rooted_finny(&net, &root);
    let step = mv((2, 7), (2, 8));
    let mut plain = root.clone();
    let mut promoted = root.clone();
    assert_finny_move(&net, &mut table, &mut plain, step, [None; 2]);
    // 王将が残るので太子が現れても着手時には表を使わない。
    assert_finny_move(
        &net,
        &mut table,
        &mut promoted,
        Move {
            promote: true,
            ..step
        },
        [None; 2],
    );
    assert_eq!(plain.occupied(), promoted.occupied());
    assert_finny_revisits(
        &net,
        &root,
        [(&plain, king_step), (&promoted, king_step)],
        [None, Some(2)],
    );

    let root = position_from_codes(
        Black,
        &[
            kings[0],
            kings[1],
            (sq(2, 2), piece(Black, Rook)),
            (sq(3, 3), piece(Black, GoldGeneral)),
            (sq(2, 3), piece(White, Pawn)),
        ],
    );
    let mut table = rooted_finny(&net, &root);
    let mut by_rook = root.clone();
    let mut by_gold = root.clone();
    for (position, capture, quiet) in [
        (&mut by_rook, mv((2, 2), (2, 3)), mv((3, 3), (3, 2))),
        (&mut by_gold, mv((3, 3), (2, 3)), mv((2, 2), (3, 2))),
    ] {
        assert_finny_move(&net, &mut table, position, capture, [None; 2]);
        position.make_null_move();
        assert_finny_move(&net, &mut table, position, quiet, [None; 2]);
    }
    // どちらが歩兵を取ったかで、2升の駒コードだけが入れ替わる。
    assert_eq!(by_rook.occupied(), by_gold.occupied());
    assert_finny_revisits(
        &net,
        &root,
        [(&by_rook, king_step), (&by_gold, king_step)],
        [None, Some(4)],
    );
}

/// 盤面が同じでも先獅子の消滅と発生を更新し、null moveでは表を更新しない。
#[test]
fn finny_detects_lion_trigger_alone_and_ignores_null_updates() {
    let net = transformer::<HalfKpVm>(4);
    let root = position_from_codes(
        Black,
        &[
            (sq(5, 4), piece(Black, King)),
            (sq(11, 0), piece(White, King)),
            (sq(6, 4), piece(White, Lion)),
            (sq(9, 3), piece(White, Pawn)),
        ],
    );
    let step = mv((5, 4), (6, 4));
    for third_has_trigger in [false, true] {
        let mut table = rooted_finny(&net, &root);
        let mut captured = root.clone();
        let before_null =
            assert_finny_move(&net, &mut table, &mut captured, step, [Some(Miss), None]);
        let lion = captured.lion_capture_square();
        assert_eq!(lion, Some(sq(6, 4)));
        assert_null(&net, &mut captured, &before_null);

        let mut quiet_parent = captured.clone();
        quiet_parent.make_null_move();
        assert_finny_move(
            &net,
            &mut table,
            &mut quiet_parent,
            mv((6, 4), (5, 4)),
            [Some(Hit(1)), None],
        );
        quiet_parent.make_null_move();
        let mut quiet = quiet_parent.clone();
        assert_finny_move(&net, &mut table, &mut quiet, step, [Some(Hit(1)), None]);
        assert_eq!(quiet.lion_capture_square(), None);
        assert!(Square::all().all(|square| quiet.piece_at(square) == captured.piece_at(square)));

        let mut third_parent = if third_has_trigger {
            root.clone()
        } else {
            quiet_parent
        };
        assert_finny_move(
            &net,
            &mut table,
            &mut third_parent,
            step,
            [Some(Hit(usize::from(third_has_trigger))), None],
        );
    }
}

/// 獅子の2回動きの後は、移動した獅子と両方の捕獲駒の行を足し引きする。
#[test]
fn finny_accounts_for_both_captures_of_a_lion_move() {
    let net = transformer::<HalfKpVm>(4);
    let root = position_from_codes(
        Black,
        &[
            (sq(0, 11), piece(Black, King)),
            (sq(10, 0), piece(White, King)),
            (sq(5, 5), piece(Black, Lion)),
            (sq(5, 4), piece(White, Pawn)),
            (sq(5, 3), piece(White, Pawn)),
        ],
    );
    let mut plain = root.clone();
    plain.make_null_move();
    let mut captured = root.clone();
    let mut table = rooted_finny(&net, &root);
    assert_finny_move(
        &net,
        &mut table,
        &mut captured,
        Move {
            mid: Some(sq(5, 4)),
            ..mv((5, 5), (5, 3))
        },
        [None; 2],
    );
    let step = mv((10, 0), (9, 0));
    assert_finny_revisits(
        &net,
        &root,
        [(&plain, step), (&captured, step)],
        [None, Some(4)],
    );
}

/// 王将を取った後の太子への切り替え、両視点の同時更新、および文脈の消滅を照合する。
#[test]
fn finny_handles_royal_capture_with_or_without_a_prince() {
    let net = transformer::<HalfKpVm>(4);
    for has_prince in [true, false] {
        let mut pieces = vec![
            (sq(6, 3), piece(Black, King)),
            (sq(5, 3), piece(White, King)),
            (sq(5, 5), piece(Black, Rook)),
            (sq(2, 4), piece(Black, Pawn)),
        ];
        if has_prince {
            pieces.push((sq(9, 8), piece(White, CrownPrince)));
        }
        let root = position_from_codes(Black, &pieces);
        let mut changed = root.clone();
        make_legal_move(&mut changed, mv((2, 4), (2, 5)));
        changed.make_null_move();
        let white_rows = has_prince.then_some(2);
        for (capture, rows) in [
            (mv((6, 3), (5, 3)), [Some(2), white_rows]),
            (mv((5, 5), (5, 3)), [None, white_rows]),
        ] {
            if rows == [None; 2] {
                // 新しい文脈がないだけなら全再計算に数えない。
                let mut table = rooted_finny(&net, &root);
                assert_finny_move(&net, &mut table, &mut root.clone(), capture, [None; 2]);
            } else {
                assert_finny_revisits(&net, &root, [(&root, capture), (&changed, capture)], rows);
            }
        }
    }
}

#[test]
fn constructor_rejects_invalid_dimensions() {
    for width in [0, 1, 3] {
        assert!(
            matches!(FeatureTransformer::<A>::new(width, Box::new([]), Box::new([])), Err(TransformerError::Width { width: actual }) if actual == width)
        );
    }
    assert!(matches!(
        FeatureTransformer::<A>::new(2, Box::new([0]), Box::new([])),
        Err(TransformerError::BiasLength {
            expected: 2,
            actual: 1
        })
    ));
    assert!(matches!(
        FeatureTransformer::<A>::new(2, Box::new([0, 0]), Box::new([])),
        Err(TransformerError::WeightLength {
            expected: Some(27_360),
            actual: 0
        })
    ));
    assert!(matches!(
        FeatureTransformer::<A>::new(usize::MAX - 1, Box::new([]), Box::new([])),
        Err(TransformerError::WeightLength {
            expected: None,
            actual: 0
        })
    ));
}

#[test]
fn invalid_contexts_are_cleared_and_new_contexts_are_refreshed() {
    let net = transformer::<HalfKpVm>(16);
    let mut position = position_from_codes(
        Black,
        &[
            (sq(0, 11), piece(Black, King)),
            (sq(11, 0), piece(White, King)),
            (sq(5, 5), piece(Black, Pawn)),
        ],
    );
    let mut before = FtAccumulator::new(net.width);
    assert_eq!(before.context(Black), None);
    assert_eq!(before.row(Black), None);
    assert!(
        before
            .rows
            .iter()
            .all(|row| row.iter().all(|&value| value == 0))
    );
    let undo = position.make_move_unchecked(mv((5, 5), (5, 6)), MoveRules::standard());
    let mut after = net.refresh(&Position::initial());
    assert!(net.update_after_move(&before, &mut after, &position, &undo));
    assert_eq!(after, net.refresh(&position));
    for pieces in [
        vec![],
        vec![
            (sq(0, 0), piece(Black, King)),
            (sq(1, 0), piece(Black, King)),
        ],
        vec![
            (sq(0, 0), piece(Black, CrownPrince)),
            (sq(1, 0), piece(Black, CrownPrince)),
        ],
    ] {
        let mut invalid = position_from_codes(Black, &pieces);
        before = net.refresh(&invalid);
        for side in Color::ALL {
            assert_eq!(before.context(side), None);
            assert!(before.rows[side.index()].iter().all(|&value| value == 0));
        }
        assert_null(&net, &mut invalid, &before);
    }
}

#[test]
fn intermediate_i16_overflow_preserves_final_sum_and_difference() {
    let width = 2;
    let mut weights = vec![0; 13_680 * width];
    // 最終値は±30,000だが、自歩兵の行を引いた直後は±40,000になる。
    for (color, square, value) in [
        (Black, sq(5, 5), -10_000),
        (Black, sq(5, 6), -10_000),
        (White, sq(9, 3), 10_000),
    ] {
        for perspective in Color::ALL {
            let row = crate::eval::features::feature_index(perspective, piece(color, Pawn), square);
            weights[row * width..(row + 1) * width].copy_from_slice(&[value, -value]);
        }
    }
    let net = FeatureTransformer::<A>::new(
        width,
        Box::new([30_000, -30_000]),
        weights.into_boxed_slice(),
    )
    .unwrap();
    let mut position = position_from_codes(
        Black,
        &[
            (sq(0, 11), piece(Black, King)),
            (sq(11, 0), piece(White, King)),
            (sq(5, 5), piece(Black, Pawn)),
            (sq(9, 3), piece(White, Pawn)),
        ],
    );
    let before = net.refresh(&position);
    for side in Color::ALL {
        assert_eq!(before.row(side).unwrap(), [30_000, -30_000]);
    }
    assert_move(&net, &mut position, mv((5, 5), (5, 6)), false);
}

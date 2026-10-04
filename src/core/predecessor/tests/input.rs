use super::*;

#[test]
fn material_overflow_reports_origin_and_owner() {
    // 設計書「駒在庫」、第4・5条: 歩兵12枚を超える入力は拒否する。
    for color in Color::ALL {
        let pawns: Vec<_> = (0..13)
            .map(|i| (sq(i % 12, i / 12), color, PieceKind::Pawn))
            .collect();
        let target = position(color, &pawns);
        assert_eq!(
            PredecessorGenerator::standard().generate_predecessors(&target),
            Err(PredecessorError::MaterialExceedsInitial {
                color,
                origin: PieceKind::Pawn,
                found: 13,
                maximum: 12
            })
        );
    }
}

#[test]
fn promoted_material_has_its_own_origin() {
    // 設計書「駒在庫」、第9条: 成獅子は麒麟、成金は歩兵の在庫を使う。
    let target = position_from_codes(
        Color::Black,
        &[
            (
                sq(0, 0),
                PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
            ),
            (
                sq(1, 0),
                PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap(),
            ),
        ],
    );
    assert!(checked(MoveRules::standard(), &target).is_empty());
    let mut pieces: Vec<_> = (0..12)
        .map(|file| {
            (
                sq(file, 0),
                PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
            )
        })
        .collect();
    pieces.extend((0..2).map(|file| {
        (
            sq(file, 1),
            PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap(),
        )
    }));
    let target = position_from_codes(Color::Black, &pieces);
    assert!(checked(MoveRules::standard(), &target).is_empty());
    pieces.push((
        sq(2, 1),
        PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
    ));
    assert_eq!(
        PredecessorGenerator::standard()
            .generate_predecessors(&position_from_codes(Color::Black, &pieces)),
        Err(PredecessorError::MaterialExceedsInitial {
            color: Color::Black,
            origin: PieceKind::Pawn,
            found: 13,
            maximum: 12
        })
    );
}

#[test]
fn deferred_state_must_match_promotion_rules() {
    // 設計書「直前局面の定義」、第30条: P0は保留なし、P0+P5は歩兵のみ、P2は各側高々1個。
    for color in Color::ALL {
        let rank = if color == Color::Black { 9 } else { 2 };
        let squares = [sq(4, rank), sq(5, rank)];
        let pieces = squares.map(|s| (s, PieceCode::new(color, PieceKind::GoldGeneral).unwrap()));
        let target = deferred_position(color, &pieces, &squares);
        for rules in [
            MoveRules::standard(),
            MoveRules {
                p5: true,
                ..MoveRules::standard()
            },
            MoveRules {
                promotion: PromotionRule::P2,
                ..MoveRules::standard()
            },
        ] {
            assert_eq!(
                PredecessorGenerator::new(rules).generate_predecessors(&target),
                Err(PredecessorError::RuleStateMismatch)
            );
        }
        assert!(
            checked(
                MoveRules {
                    promotion: PromotionRule::P1,
                    ..MoveRules::standard()
                },
                &target
            )
            .is_empty()
        );
        let pawns = squares.map(|s| (s, PieceCode::new(color, PieceKind::Pawn).unwrap()));
        let target = deferred_position(color, &pawns, &squares);
        for promotion in [PromotionRule::P0, PromotionRule::P1, PromotionRule::P2] {
            assert!(
                checked(
                    MoveRules {
                        promotion,
                        p5: true,
                        ..MoveRules::standard()
                    },
                    &target
                )
                .is_empty()
            );
        }
        assert_eq!(
            PredecessorGenerator::new(MoveRules {
                promotion: PromotionRule::P2,
                ..MoveRules::standard()
            })
            .generate_predecessors(&target),
            Err(PredecessorError::RuleStateMismatch)
        );
    }
}

#[test]
fn lion_record_rejects_unpromoted_lion_full_stock_and_unexplained_empty_square() {
    // 設計書「直前局面の定義」: 記録升の駒、手番側の欠損、空升での獅子力を検査する。
    let fixtures = [
        vec![(sq(5, 5), Color::Black, PieceKind::Lion)],
        vec![
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(0, 0), Color::White, PieceKind::Lion),
            (sq(1, 0), Color::White, PieceKind::Kirin),
        ],
        vec![(sq(4, 5), Color::Black, PieceKind::Rook)],
    ];
    for pieces in fixtures {
        let mut target = position(Color::White, &pieces);
        target.set_lion_capture(Some(sq(5, 5))).unwrap();
        assert_eq!(target.validate(), Ok(()));
        assert_eq!(
            PredecessorGenerator::standard().generate_predecessors(&target),
            Err(PredecessorError::InvalidLionState)
        );
    }
}

#[test]
fn lion_record_accepts_empty_square_with_falcon_and_promoted_lion() {
    // 設計書「直前局面の定義」: 麒麟由来の成獅子と角鷹の経由升捕獲は記録を持てる。
    for kind in [
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
        PieceKind::Lion,
    ] {
        let square = if kind == PieceKind::Lion {
            sq(5, 5)
        } else {
            sq(4, 5)
        };
        let mut target = position_from_codes(
            Color::White,
            &[(square, PieceCode::new_promoted(Color::Black, kind).unwrap())],
        );
        target.set_lion_capture(Some(sq(5, 5))).unwrap();
        assert!(
            PredecessorGenerator::standard()
                .generate_predecessors(&target)
                .is_ok()
        );
    }
}

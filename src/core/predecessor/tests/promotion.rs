use super::*;

#[test]
fn pawn_outside_zone_has_only_unpromoted_predecessor() {
    // 第17・18条: 敵陣外の歩兵の移動で成駒が歩兵に戻ることはない。
    let from = sq(5, 4);
    let to = sq(5, 5);
    let predecessor = position(Color::Black, &[(from, Color::Black, PieceKind::Pawn)]);
    let result = round_trip(MoveRules::standard(), &predecessor, mv(from, to, false));
    let promoted = position_from_codes(
        Color::Black,
        &[(
            from,
            PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
        )],
    );
    assert!(!result.contains(&promoted));
    assert!(
        result
            .iter()
            .all(|p| p.piece_at(from)
                == Some(PieceCode::new(Color::Black, PieceKind::Pawn).unwrap()))
    );
}

#[test]
fn entering_zone_preserves_both_promotion_choices() {
    // 第18条1・5項: 敵陣へ入る同じ局面から成り・不成の各結果を逆にたどれる。
    let from = sq(5, 7);
    let to = sq(5, 8);
    let predecessor = position(Color::Black, &[(from, Color::Black, PieceKind::Pawn)]);
    let wrong = position(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]);
    for promote in [false, true] {
        let result = round_trip(MoveRules::standard(), &predecessor, mv(from, to, promote));
        assert!(!result.contains(&wrong));
    }
}

#[test]
fn newly_promoted_and_already_promoted_are_distinct_predecessors() {
    // 第17・18条、設計書「逆向き候補の構成」: 不成からの成りと既成駒の移動は別局面。
    let from = sq(5, 7);
    let to = sq(5, 8);
    let pawn = position(Color::Black, &[(from, Color::Black, PieceKind::Pawn)]);
    let gold = position_from_codes(
        Color::Black,
        &[(
            from,
            PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
        )],
    );
    let result = round_trip(MoveRules::standard(), &pawn, mv(from, to, true));
    assert_eq!(result.iter().filter(|p| **p == pawn).count(), 1);
    assert_eq!(result.iter().filter(|p| **p == gold).count(), 1);
    round_trip(MoveRules::standard(), &gold, mv(from, to, false));
    let native_gold = position(
        Color::Black,
        &[(from, Color::Black, PieceKind::GoldGeneral)],
    );
    assert!(!result.contains(&native_gold));
}

#[test]
fn promoted_pieces_cannot_promote_twice() {
    // 第17条4項: 成金(歩兵由来)から飛車への二重成りを逆生成しない。
    let from = sq(5, 7);
    let to = sq(5, 8);
    let gold = position(
        Color::Black,
        &[(from, Color::Black, PieceKind::GoldGeneral)],
    );
    let result = round_trip(MoveRules::standard(), &gold, mv(from, to, true));
    let promoted_gold = position_from_codes(
        Color::Black,
        &[(
            from,
            PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
        )],
    );
    assert!(!result.contains(&promoted_gold));
    let mut invalid = promoted_gold.clone();
    assert!(
        invalid
            .try_make_move(mv(from, to, true), &MoveGenerator::standard())
            .is_err()
    );
    assert_eq!(invalid, promoted_gold);
}

#[test]
fn p6_rejects_unpromoted_pawn_on_last_rank() {
    // 第19条・第30条P6: 標準では最奥段で不成にできるが、P6では強制成りとなる。
    for color in Color::ALL {
        let (from, to) = if color == Color::Black {
            (sq(5, 10), sq(5, 11))
        } else {
            (sq(5, 1), sq(5, 0))
        };
        let predecessor = position(color, &[(from, color, PieceKind::Pawn)]);
        let mut target = predecessor.clone();
        target
            .try_make_move(mv(from, to, false), &MoveGenerator::standard())
            .unwrap();
        assert!(checked(MoveRules::standard(), &target).contains(&predecessor));
        assert!(
            !checked(
                MoveRules {
                    p6: true,
                    ..MoveRules::standard()
                },
                &target
            )
            .contains(&predecessor)
        );
    }
}

#[test]
fn p1_restores_mover_and_captured_deferred_bits_independently() {
    // 設計書「一時状態の逆生成」、第30条P1: 移動元と捕獲升の保留を独立に復元する。
    // 両側の敵陣は重ならないため、相手の保留駒は敵陣を出る走りで捕獲する。
    let rules = MoveRules {
        promotion: PromotionRule::P1,
        ..MoveRules::standard()
    };
    let from = sq(5, 9);
    let to = sq(5, 2);
    let pieces = [
        (from, PieceCode::new(Color::Black, PieceKind::Rook).unwrap()),
        (
            to,
            PieceCode::new(Color::White, PieceKind::SilverGeneral).unwrap(),
        ),
        (
            sq(1, 9),
            PieceCode::new(Color::Black, PieceKind::Pawn).unwrap(),
        ),
    ];
    let mut targets = Vec::new();
    for mover_deferred in [false, true] {
        for captured_deferred in [false, true] {
            let mut deferred = vec![sq(1, 9)];
            if mover_deferred {
                deferred.push(from);
            }
            if captured_deferred {
                deferred.push(to);
            }
            let predecessor = deferred_position(Color::Black, &pieces, &deferred);
            let result = round_trip(rules, &predecessor, mv(from, to, false));
            let wrong = deferred_position(Color::Black, &pieces, &[]);
            assert!(!result.contains(&wrong));
            let mut target = predecessor;
            target
                .try_make_move(mv(from, to, false), &MoveGenerator::new(rules))
                .unwrap();
            targets.push(target);
        }
    }
    assert!(targets.iter().all(|target| *target == targets[0]));
}

#[test]
fn p5_restores_only_pawn_deferral_and_keeps_unrelated_bits() {
    // 第30条P5、設計書「一時状態の逆生成」: 移動歩兵の保留を復元し、無関係な保留を維持する。
    let rules = MoveRules {
        p5: true,
        ..MoveRules::standard()
    };
    let from = sq(5, 8);
    let to = sq(5, 9);
    let pawn = PieceCode::new(Color::Black, PieceKind::Pawn).unwrap();
    let pieces = [(from, pawn), (sq(1, 8), pawn)];
    let predecessor = deferred_position(Color::Black, &pieces, &[from, sq(1, 8)]);
    let result = round_trip(rules, &predecessor, mv(from, to, false));
    let wrong = deferred_position(Color::Black, &pieces, &[from]);
    assert!(!result.contains(&wrong));
    let entry = position(Color::Black, &[(sq(5, 7), Color::Black, PieceKind::Pawn)]);
    round_trip(rules, &entry, mv(sq(5, 7), sq(5, 8), false));
}

#[test]
fn p2_rejects_candidates_with_two_enemy_waiting_bits() {
    // 設計書「直前局面の定義」「一時状態の逆生成」: 捕獲で消える不正な待機も候補検査で拒否する。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        ..MoveRules::standard()
    };
    let from = sq(5, 3);
    let to = sq(5, 2);
    let pieces = [
        (
            from,
            PieceCode::new(Color::Black, PieceKind::GoldGeneral).unwrap(),
        ),
        (
            to,
            PieceCode::new(Color::White, PieceKind::SilverGeneral).unwrap(),
        ),
        (
            sq(1, 2),
            PieceCode::new(Color::White, PieceKind::SilverGeneral).unwrap(),
        ),
    ];
    let predecessor = deferred_position(Color::Black, &pieces, &[sq(1, 2)]);
    let result = round_trip(rules, &predecessor, mv(from, to, false));
    let invalid = deferred_position(Color::Black, &pieces, &[to, sq(1, 2)]);
    assert!(!result.contains(&invalid));
}

use super::*;

#[test]
fn entering_zone_without_promotion_has_its_predecessor() {
    // 第18条1・5項: 敵陣入りで不成を選んだ結果から直前局面を復元できる。
    let from = sq(5, 7);
    let to = sq(5, 8);
    let predecessor = position(Color::Black, &[(from, Color::Black, PieceKind::Pawn)]);
    round_trip(MoveRules::standard(), &predecessor, mv(from, to, false));
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
fn p2_capture_with_another_enemy_piece_waiting_has_its_predecessor() {
    // 設計書「直前局面の定義」「一時状態の逆生成」: 捕獲駒以外の相手駒に待機がある直前局面を復元する。
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
    round_trip(rules, &predecessor, mv(from, to, false));
}

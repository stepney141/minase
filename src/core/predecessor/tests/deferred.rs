//! 成り権保留の新設、満了、恒久保留、および捕獲で消える状態を検査する。

use super::*;

/// 到達升で新設される保留を移動元へコピーしない。
#[test]
fn p1_new_destination_bit_is_not_inherited() {
    // 設計書「一時状態の逆生成」、第30条P1: 敵陣へ入る直前には保留できない。
    let rules = MoveRules {
        promotion: PromotionRule::P1,
        ..MoveRules::standard()
    };
    let p = position(Color::Black, &[(sq(5, 7), Color::Black, PieceKind::Pawn)]);
    let mut q = p.clone();
    q.try_make_move(mv(sq(5, 7), sq(5, 8), false), &MoveGenerator::new(rules))
        .unwrap();
    assert!(q.promotion_deferred().contains(sq(5, 8)));
    let result = checked(rules, &q);
    assert!(result.contains(&p));
    assert!(
        result
            .iter()
            .all(|p| !p.promotion_deferred().contains(sq(5, 8)))
    );
}

/// P2は非移動駒を含む着手側の待機を高々1個だけ復元する。
#[test]
fn p2_restores_expired_waiting_and_new_entry() {
    // 設計書「一時状態の逆生成」、第30条P2: 新設と非移動駒の満了を同時に検査する。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        ..MoveRules::standard()
    };
    let from = sq(5, 7);
    let to = sq(5, 8);
    let a = sq(1, 9);
    let b = sq(2, 9);
    let base = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Pawn),
            (a, Color::Black, PieceKind::SilverGeneral),
            (b, Color::Black, PieceKind::SilverGeneral),
        ],
    );
    let mut q = base.clone();
    q.try_make_move(mv(from, to, false), &MoveGenerator::new(rules))
        .unwrap();
    assert!(q.promotion_deferred().contains(to));
    let result = checked(rules, &q);
    for deferred in [vec![], vec![a], vec![b]] {
        assert!(result.contains(&with_state(&base, &deferred, None)));
    }
    assert!(!result.contains(&with_state(&base, &[a, b], None)));
    assert!(
        result
            .iter()
            .all(|p| p.promotion_deferred().popcount() <= 1)
    );
}

/// P2の移動駒自身にあった待機も復元する。
#[test]
fn p2_restores_waiting_on_moving_piece() {
    // 第30条P2: 敵陣内の不成移動は新たな待機を生まず、既存の待機を満了させる。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        ..MoveRules::standard()
    };
    let from = sq(5, 9);
    let base = position(
        Color::Black,
        &[(from, Color::Black, PieceKind::SilverGeneral)],
    );
    let p = with_state(&base, &[from], None);
    let result = round_trip(rules, &p, mv(from, sq(5, 10), false));
    assert!(result.contains(&base));
}

/// P2とP5の併用では複数の歩兵保留と1個の待機を同時に復元する。
#[test]
fn p2_p5_preserves_pawns_and_restores_one_waiting_piece() {
    // 第30条P2・P5: 歩兵の恒久保留は全体の待機満了から除外する。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        p5: true,
        ..MoveRules::standard()
    };
    let from = sq(5, 8);
    let pawn = sq(1, 8);
    let waiting = sq(2, 9);
    let base = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Pawn),
            (pawn, Color::Black, PieceKind::Pawn),
            (waiting, Color::Black, PieceKind::SilverGeneral),
        ],
    );
    let p = with_state(&base, &[from, pawn, waiting], None);
    let result = round_trip(rules, &p, mv(from, sq(5, 9), false));
    assert!(result.contains(&with_state(&base, &[from, pawn], None)));
    assert!(!result.contains(&with_state(&base, &[from], None)));
}

/// P5の最奥段での強制成りを標準成り規則とP2の双方で検査する。
#[test]
fn p5_forces_deferred_pawn_promotion_at_last_rank() {
    // 第30条P5: P0では捕獲時、P2では非捕獲時も、保留歩兵の最奥段の成りは強制。
    for promotion in [PromotionRule::P0, PromotionRule::P2] {
        let rules = MoveRules {
            promotion,
            p5: true,
            ..MoveRules::standard()
        };
        let from = sq(5, 10);
        let to = sq(5, 11);
        let mut pieces = vec![(from, piece(Color::Black, PieceKind::Pawn))];
        if promotion == PromotionRule::P0 {
            pieces.push((to, piece(Color::White, PieceKind::SilverGeneral)));
        }
        let p = deferred_position(Color::Black, &pieces, &[from]);
        round_trip(rules, &p, mv(from, to, true));
        assert!(
            p.clone()
                .try_make_move(mv(from, to, false), &MoveGenerator::new(rules))
                .is_err()
        );
        let qpieces = vec![(to, piece(Color::Black, PieceKind::Pawn))];
        // 成れない不成の到達局面を与えても、このpは直前局面にはならない。
        let q = deferred_position(Color::White, &qpieces, &[to]);
        assert!(!checked(rules, &q).contains(&p));
    }
}

/// P2とP6の併用で香車の最奥段強制成りと待機満了を検査する。
#[test]
fn p2_p6_forces_lance_promotion_and_expires_other_waiting() {
    // 第30条P2・P6: 敵陣内から非捕獲で進む香車も最奥段では必ず成る。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        p6: true,
        ..MoveRules::standard()
    };
    let from = sq(5, 9);
    let to = sq(5, 11);
    let waiting = sq(1, 9);
    let base = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Lance),
            (waiting, Color::Black, PieceKind::SilverGeneral),
        ],
    );
    let p = with_state(&base, &[waiting], None);
    round_trip(rules, &p, mv(from, to, true));
    let without_p6 = MoveRules { p6: false, ..rules };
    let mut q = p.clone();
    q.try_make_move(mv(from, to, false), &MoveGenerator::new(without_p6))
        .unwrap();
    assert!(!checked(rules, &q).contains(&p));
}

/// 2枚の復元捕獲駒の待機は独立に列挙した上で集合Aにより絞る。
#[test]
fn p2_rejects_two_restored_enemy_waiting_bits() {
    // 設計書「一時状態の逆生成」: 同時に消える待機2個は再適用だけでは排除できない。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        ..MoveRules::standard()
    };
    let from = sq(5, 3);
    let mid = sq(5, 2);
    let to = sq(6, 2);
    let base = stocked_position(
        Color::Black,
        &[
            (from, piece(Color::Black, PieceKind::Lion)),
            (sq(11, 6), piece(Color::Black, PieceKind::Kirin)),
            (mid, piece(Color::White, PieceKind::SilverGeneral)),
            (to, piece(Color::White, PieceKind::SilverGeneral)),
        ],
    );
    let capture = Move {
        from,
        mid: Some(mid),
        to,
        promote: false,
    };
    let result = round_trip(rules, &base, capture);
    for deferred in [vec![mid], vec![to]] {
        assert!(result.contains(&with_state(&base, &deferred, None)));
    }
    let invalid = with_state(&base, &[mid, to], None);
    let mut replayed = invalid.clone();
    replayed
        .try_make_move(capture, &MoveGenerator::new(rules))
        .unwrap();
    assert!(!result.contains(&invalid));
}

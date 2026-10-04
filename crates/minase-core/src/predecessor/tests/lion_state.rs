//! 先獅子の記録升と獅子のローカルルールを検査する。

use super::*;
use crate::LionRule;

/// 非獅子の捕獲で先獅子記録が新設されても、以前の記録を復元できる。
#[test]
fn lion_capture_restores_previous_records() {
    // 設計書「一時状態の逆生成」、第15条: 新しい記録升と以前の記録升は独立。
    let from = sq(5, 5);
    let to = sq(5, 6);
    let base = position(
        Color::Black,
        &[
            (from, Color::Black, PieceKind::Rook),
            (to, Color::White, PieceKind::Lion),
            (sq(0, 0), Color::White, PieceKind::Pawn),
            (sq(11, 0), Color::White, PieceKind::Pawn),
        ],
    );
    let mut q = base.clone();
    q.try_make_move(mv(from, to, false), &MoveGenerator::standard())
        .unwrap();
    assert_eq!(q.lion_capture_square(), Some(to));
    let result = checked(MoveRules::standard(), &q);
    for record in [None, Some(sq(0, 0)), Some(sq(11, 0))] {
        let p = with_state(&base, &[], record);
        let mut replayed = p.clone();
        replayed
            .try_make_move(mv(from, to, false), &MoveGenerator::standard())
            .unwrap();
        assert_eq!(replayed, q);
        assert!(result.contains(&p));
    }
    assert!(!result.contains(&with_state(&base, &[], Some(to))));
}

/// 空升の記録には相手の角鷹または飛鷲を必要とする。
#[test]
fn empty_record_requires_enemy_falcon_or_eagle() {
    // 設計書「直前局面の定義」先獅子の局所条件3: 経由升捕獲の可能性を要求する。
    for kind in [
        PieceKind::Rook,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ] {
        let base = position_from_codes(
            Color::Black,
            &[
                (sq(5, 5), piece(Color::Black, PieceKind::Pawn)),
                (sq(0, 0), piece(Color::White, kind)),
            ],
        );
        let result = round_trip(MoveRules::standard(), &base, mv(sq(5, 5), sq(5, 6), false));
        let with_record = with_state(&base, &[], Some(sq(8, 8)));
        if kind != PieceKind::Rook {
            assert!(result.contains(&with_record));
        }
        for p in &result {
            if let Some(s) = p.lion_capture_square() {
                assert!(
                    p.piece_at(s).is_some()
                        || !p
                            .pieces_of_kind(Color::White, PieceKind::HornedFalcon)
                            .is_empty()
                        || !p
                            .pieces_of_kind(Color::White, PieceKind::SoaringEagle)
                            .is_empty()
                );
            }
        }
    }
}

/// L1とL2の併用で、麒麟が獅子を取りながら成った升への取り返しを復元できる。
#[test]
fn l2_allows_recapture_of_just_promoted_kirin() {
    // RULES.md第15条7項・第29条L2: 実際の麒麟成りから作った記録を復元する。
    let start = position(
        Color::White,
        &[
            (sq(5, 3), Color::White, PieceKind::Kirin),
            (sq(5, 1), Color::Black, PieceKind::Lion),
            (sq(6, 1), Color::Black, PieceKind::Rook),
        ],
    );
    let l1_rules = MoveRules {
        lion: LionRule::L1,
        ..MoveRules::standard()
    };
    let mut p = start;
    p.try_make_move(mv(sq(5, 3), sq(5, 1), true), &MoveGenerator::new(l1_rules))
        .unwrap();
    assert_eq!(p.lion_capture_square(), Some(sq(5, 1)));
    assert!(p.piece_at(sq(5, 1)).unwrap().is_promoted());
    round_trip(
        MoveRules {
            l2: true,
            ..l1_rules
        },
        &p,
        mv(sq(6, 1), sq(5, 1), false),
    );
}

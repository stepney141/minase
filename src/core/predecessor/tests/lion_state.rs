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

/// 獅子と麒麟が揃った着手側には記録升ありの直前局面を生成しない。
#[test]
fn full_lion_origins_exclude_all_prior_records() {
    // 設計書「直前局面の定義」先獅子の局所条件2: 着手側の在庫は移動で変わらない。
    let p = stocked_position(
        Color::Black,
        &[
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(11, 6), piece(Color::Black, PieceKind::Kirin)),
        ],
    );
    let result = round_trip(MoveRules::standard(), &p, mv(sq(5, 5), sq(6, 5), false));
    assert!(result.iter().all(|p| p.lion_capture_square().is_none()));
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
        assert_eq!(result.contains(&with_record), kind != PieceKind::Rook);
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

/// 1つの局面と着手について、規則の違いが順方向と逆方向で一致することを確認する。
fn rule_difference(p: &Position, mv: Move, allowed: MoveRules, denied: MoveRules) {
    let mut q = p.clone();
    q.try_make_move(mv, &MoveGenerator::new(allowed)).unwrap();
    assert!(
        p.clone()
            .try_make_move(mv, &MoveGenerator::new(denied))
            .is_err()
    );
    assert!(checked(allowed, &q).contains(p));
    assert!(!checked(denied, &q).contains(p));
}

/// L1の足条件なしの禁止を逆生成にも反映する。
#[test]
fn l1_prohibits_undefended_lion_recapture() {
    // RULES.md第29条L1: 足のない獅子の非獅子による取り返しはL0だけが認める。
    let base = position(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::Rook),
            (sq(5, 6), Color::White, PieceKind::Lion),
            (sq(0, 0), Color::White, PieceKind::Pawn),
        ],
    );
    let p = with_state(&base, &[], Some(sq(0, 0)));
    rule_difference(
        &p,
        mv(sq(5, 5), sq(5, 6), false),
        MoveRules::standard(),
        MoveRules {
            lion: LionRule::L1,
            ..MoveRules::standard()
        },
    );
}

/// 麒麟が獅子を取りながら成った升への取り返しをL2だけが許す。
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
    let denied = MoveRules {
        lion: LionRule::L1,
        ..MoveRules::standard()
    };
    let mut p = start;
    p.try_make_move(mv(sq(5, 3), sq(5, 1), true), &MoveGenerator::new(denied))
        .unwrap();
    assert_eq!(p.lion_capture_square(), Some(sq(5, 1)));
    assert!(p.piece_at(sq(5, 1)).unwrap().is_promoted());
    rule_difference(
        &p,
        mv(sq(6, 1), sq(5, 1), false),
        MoveRules { l2: true, ..denied },
        denied,
    );
}

/// 唯一の足の歩兵を経由升で取る着手にL3を反映する。
#[test]
fn l3_removes_pawn_support_between_stages() {
    // RULES.md第16条8〜10項・第29条L3: 歩兵は価値ある付け喰い駒にはならない。
    // 後手歩兵は南へ利くため、獅子を南側に置き、先手獅子は北から近づく。
    // 両獅子と盤端の後手歩兵1枚を初期配置から動かし、後手獅子の足を歩兵だけにする。
    let p = initial_with(
        Color::Black,
        &[sq(5, 2), sq(6, 9), sq(0, 8)],
        &[
            (sq(5, 7), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 6), piece(Color::White, PieceKind::Pawn)),
            (sq(5, 5), piece(Color::White, PieceKind::Lion)),
        ],
    );
    rule_difference(
        &p,
        Move {
            from: sq(5, 7),
            mid: Some(sq(5, 6)),
            to: sq(5, 5),
            promote: false,
        },
        MoveRules {
            l3: true,
            ..MoveRules::standard()
        },
        MoveRules::standard(),
    );
}

/// L4では獅子による取り返しを先獅子で禁止しない。
#[test]
fn l4_allows_lion_recapture_under_prior_record() {
    // RULES.md第15条9項・第29条L4: 足のある隣接獅子への取り返し。
    // 記録升のある局面では手番側の獅子在庫が欠けている必要があるため、
    // 先手の麒麟を除き、両獅子と盤端の後手歩兵1枚を初期配置から動かす。
    let base = initial_with(
        Color::Black,
        &[sq(5, 2), sq(5, 1), sq(6, 9), sq(0, 8)],
        &[
            (sq(5, 5), piece(Color::Black, PieceKind::Lion)),
            (sq(5, 6), piece(Color::White, PieceKind::Lion)),
            (sq(5, 7), piece(Color::White, PieceKind::Pawn)),
        ],
    );
    let p = with_state(&base, &[], Some(sq(5, 7)));
    rule_difference(
        &p,
        mv(sq(5, 5), sq(5, 6), false),
        MoveRules {
            lion: LionRule::L0 { l4: true },
            ..MoveRules::standard()
        },
        MoveRules::standard(),
    );
}

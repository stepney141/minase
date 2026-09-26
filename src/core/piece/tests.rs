//! 駒種、成りの対応、および駒コードの試験。

use super::*;

/// 初期配置で使用する21種の駒種(第4条・第5条)。王将・玉将は性能同一の
/// 1種として数える(第5条、SPEC_UNCLEAR SU-D4-3)。
const INITIAL_KINDS: [PieceKind; 21] = [
    PieceKind::Pawn,
    PieceKind::GoBetween,
    PieceKind::Lance,
    PieceKind::ReverseChariot,
    PieceKind::SideMover,
    PieceKind::VerticalMover,
    PieceKind::Bishop,
    PieceKind::Rook,
    PieceKind::DragonHorse,
    PieceKind::DragonKing,
    PieceKind::FreeKing,
    PieceKind::King,
    PieceKind::DrunkElephant,
    PieceKind::FerociousLeopard,
    PieceKind::BlindTiger,
    PieceKind::CopperGeneral,
    PieceKind::SilverGeneral,
    PieceKind::GoldGeneral,
    PieceKind::Kirin,
    PieceKind::Phoenix,
    PieceKind::Lion,
];

/// 成駒としてのみ現れる8種の駒種(第10条)。
const PROMOTED_ONLY_KINDS: [PieceKind; 8] = [
    PieceKind::WhiteHorse,
    PieceKind::Whale,
    PieceKind::FlyingStag,
    PieceKind::FreeBoar,
    PieceKind::FlyingOx,
    PieceKind::CrownPrince,
    PieceKind::HornedFalcon,
    PieceKind::SoaringEagle,
];

// 駒種は初期21種＋成駒のみ8種の29種である(第4条3項・第10条、D4-004-03)。
#[test]
fn article_4_3_piece_kinds_are_21_initial_plus_8_promoted_only() {
    assert_eq!(INITIAL_KINDS.len() + PROMOTED_ONLY_KINDS.len(), 29);

    // 21種と8種は互いに素で、全29種をちょうど覆う。
    for kind in PieceKind::ALL {
        let initial = INITIAL_KINDS.contains(&kind);
        let promoted_only = PROMOTED_ONLY_KINDS.contains(&kind);
        assert!(
            initial != promoted_only,
            "初期駒と成駒専用のどちらか一方に属する: {kind:?}"
        );
    }
    // ALLに重複はない。
    for (index, kind) in PieceKind::ALL.iter().enumerate() {
        assert!(
            !PieceKind::ALL[..index].contains(kind),
            "駒種の重複: {kind:?}"
        );
    }

    // 成りによって新たに現れる駒種は、ちょうど第10条の8種である(D4-004-03性質)。
    let mut newly_appearing: Vec<PieceKind> = PieceKind::ALL
        .iter()
        .filter_map(|kind| kind.promoted())
        .filter(|target| !INITIAL_KINDS.contains(target))
        .collect();
    newly_appearing.sort_by_key(|kind| kind.index());
    newly_appearing.dedup();
    let mut expected = PROMOTED_ONLY_KINDS.to_vec();
    expected.sort_by_key(|kind| kind.index());
    assert_eq!(newly_appearing, expected);
}

// 実装契約(D4-IMP-06): 成り対応は第9条の表とちょうど一致し、成る→戻すが恒等である。
// 成れない駒種(第17条1項)と成駒の再成り禁止(第17条4項)も表から確定する。
#[test]
fn promotion_pairs_match_article_9_table_and_demotion_is_inverse() {
    // 第9条の成駒対応表の写し(成れる18種)。
    let pairs = [
        (PieceKind::GoldGeneral, PieceKind::Rook),
        (PieceKind::SilverGeneral, PieceKind::VerticalMover),
        (PieceKind::CopperGeneral, PieceKind::SideMover),
        (PieceKind::FerociousLeopard, PieceKind::Bishop),
        (PieceKind::BlindTiger, PieceKind::FlyingStag),
        (PieceKind::DrunkElephant, PieceKind::CrownPrince),
        (PieceKind::Pawn, PieceKind::GoldGeneral),
        (PieceKind::GoBetween, PieceKind::DrunkElephant),
        (PieceKind::Lance, PieceKind::WhiteHorse),
        (PieceKind::ReverseChariot, PieceKind::Whale),
        (PieceKind::SideMover, PieceKind::FreeBoar),
        (PieceKind::VerticalMover, PieceKind::FlyingOx),
        (PieceKind::Bishop, PieceKind::DragonHorse),
        (PieceKind::Rook, PieceKind::DragonKing),
        (PieceKind::DragonHorse, PieceKind::HornedFalcon),
        (PieceKind::DragonKing, PieceKind::SoaringEagle),
        (PieceKind::Kirin, PieceKind::Lion),
        (PieceKind::Phoenix, PieceKind::FreeKing),
    ];

    for (base, target) in pairs {
        assert_eq!(base.promoted(), Some(target), "{base:?}の成り先");
        // demote∘promoteは恒等である。
        assert_eq!(target.unpromoted(), Some(base), "{target:?}の成り元");
        assert!(base.can_promote());
    }

    // 表にない駒種は成り先を持たない: 王将・玉将・獅子・奔王(第17条1項)と、
    // 成駒としてのみ現れる8種(第17条4項・第10条)。
    for kind in PieceKind::ALL {
        let in_table = pairs.iter().any(|&(base, _)| base == kind);
        assert_eq!(kind.promoted().is_some(), in_table, "{kind:?}");
        assert_eq!(kind.can_promote(), in_table);
        // 成駒として現れる駒種は、ちょうど表の右列に現れる駒種である。
        let is_target = pairs.iter().any(|&(_, target)| target == kind);
        assert_eq!(kind.unpromoted().is_some(), is_target, "{kind:?}");
    }

    for color in Color::ALL {
        for (base, target) in pairs {
            // 駒コードでの成りも表と一致し、所有者を保存する。
            let promoted = PieceCode::new(color, base).unwrap().promote().unwrap();
            assert_eq!(promoted.kind(), Some(target));
            assert_eq!(promoted.color(), Some(color));
            assert!(promoted.is_promoted());
            // 成った駒はさらに成れない(第17条4項)。仲人の成駒(醉象の動き)が
            // 太子へ成れないことも、この一般則に含まれる。
            assert!(promoted.promote().is_none());
        }
        for kind in [PieceKind::King, PieceKind::Lion, PieceKind::FreeKing] {
            assert!(PieceCode::new(color, kind).unwrap().promote().is_none());
        }
        // 成駒コードを構築できるのは、成駒として現れる駒種にちょうど限られる。
        for kind in PieceKind::ALL {
            assert_eq!(
                PieceCode::new_promoted(color, kind).is_some(),
                kind.unpromoted().is_some()
            );
        }
    }
}

// 実装契約(D4-IMP-05): 駒コードの符号化は(所有者, 駒種, 成否)の上で単射で、
// 復号との往復は恒等である。空升・番兵はどの駒コードとも区別される。
// 生の金将と歩兵の成駒のような動き同一の対も、成否によって区別される(第24条1項a)。
#[test]
fn piece_codes_are_injective_over_owner_kind_and_promotion() {
    let mut codes = Vec::new();
    for color in Color::ALL {
        for kind in PieceKind::ALL {
            let Some(raw) = PieceCode::new(color, kind) else {
                assert!(!kind.can_exist_unpromoted());
                continue;
            };
            assert_eq!(raw.color(), Some(color));
            assert_eq!(raw.kind(), Some(kind));
            assert!(!raw.is_promoted());
            codes.push(raw);
            if let Some(promoted) = PieceCode::new_promoted(color, kind) {
                assert_eq!(promoted.color(), Some(color));
                assert_eq!(promoted.kind(), Some(kind));
                assert!(promoted.is_promoted());
                codes.push(promoted);
            }
        }
    }

    // 相異なる(所有者, 駒種, 成否)は相異なる符号を持つ。
    for (index, code) in codes.iter().enumerate() {
        assert!(!codes[..index].contains(code), "駒コードの衝突: {code:?}");
        assert!(!code.is_empty());
        assert!(!code.is_wall());
        assert_ne!(*code, PieceCode::EMPTY);
        assert_ne!(*code, PieceCode::WALL);
    }

    // 空升・番兵は駒として復号されない。
    for sentinel in [PieceCode::EMPTY, PieceCode::WALL] {
        assert_eq!(sentinel.color(), None);
        assert_eq!(sentinel.kind(), None);
        assert!(!sentinel.is_promoted());
    }
    assert!(PieceCode::EMPTY.is_empty());
    assert!(PieceCode::WALL.is_wall());
    assert_ne!(PieceCode::EMPTY, PieceCode::WALL);
}

// 第10条。成駒専用8種は未成の盤上駒コードとして構築できない。
#[test]
fn promoted_only_kinds_have_no_unpromoted_piece_code() {
    for color in Color::ALL {
        for kind in PROMOTED_ONLY_KINDS {
            assert_eq!(PieceCode::new(color, kind), None, "{color:?} {kind:?}");
            assert!(!kind.can_exist_unpromoted());
        }
        for kind in INITIAL_KINDS {
            assert!(PieceCode::new(color, kind).is_some(), "{color:?} {kind:?}");
            assert!(kind.can_exist_unpromoted());
        }
    }
}

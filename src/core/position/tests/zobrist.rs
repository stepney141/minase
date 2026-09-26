//! 局面と成り権保留状態のzobristハッシュの試験。

use super::{
    position_after_non_lion_captures_lion, positions_with_distinct_temporary_states,
    rebuild_board_with_side,
};
use crate::MoveGenerator;
use crate::core::board::Square;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuildError, PositionBuilder, PositionError};
use crate::test_util::{position as position_with_pieces, position_from_codes, sq};
use core::hash::{Hash, Hasher};
use std::collections::HashSet;
use std::hash::{BuildHasherDefault, DefaultHasher};

#[test]
fn equal_positions_have_equal_hashes() {
    // 設計書predecessor-generator.md「Positionのハッシュ」
    let first = positions_with_distinct_temporary_states();
    let second = positions_with_distinct_temporary_states();
    let (first_capture, _) = position_after_non_lion_captures_lion();
    let (second_capture, _) = position_after_non_lion_captures_lion();
    for (first, second) in first
        .into_iter()
        .zip(second)
        .chain([(first_capture, second_capture)])
    {
        assert_eq!(first, second);
        let mut first_hasher = DefaultHasher::new();
        let mut second_hasher = DefaultHasher::new();
        first.hash(&mut first_hasher);
        second.hash(&mut second_hasher);
        assert_eq!(first_hasher.finish(), second_hasher.finish());
    }
}

#[test]
fn hash_set_distinguishes_side_and_temporary_states() {
    // 設計書predecessor-generator.md「Positionのハッシュ」
    let mut positions = HashSet::new();
    for position in positions_with_distinct_temporary_states() {
        assert!(positions.insert(position));
    }
    assert_eq!(positions.len(), 8);
    for position in positions_with_distinct_temporary_states() {
        assert!(positions.contains(&position));
        assert!(!positions.insert(position));
    }
}

#[test]
fn hash_set_preserves_distinct_positions_under_collisions() {
    // 設計書predecessor-generator.md「Positionのハッシュ」
    #[derive(Default)]
    struct ConstantHasher;

    impl Hasher for ConstantHasher {
        fn finish(&self) -> u64 {
            0
        }

        fn write(&mut self, _bytes: &[u8]) {}
    }

    let mut positions = HashSet::<Position, BuildHasherDefault<ConstantHasher>>::default();
    for position in positions_with_distinct_temporary_states() {
        assert!(positions.insert(position));
    }
    assert_eq!(positions.len(), 8);
    for position in positions_with_distinct_temporary_states() {
        assert!(positions.contains(&position));
        assert!(!positions.insert(position));
    }
}

// 局面キーは全駒の位置・種類・所有者・成否を区別する(第24条1項a・2項、D4-024-01)。
// 成否の区別は動きが同一の対(生の金将と歩兵の成駒など、第17条4項)にも及ぶ。
#[test]
fn article_24_1_a_key_distinguishes_placement_kind_owner_and_promotion() {
    let anchor = [
        (sq(0, 0), Color::Black, PieceKind::King),
        (sq(11, 11), Color::White, PieceKind::King),
    ];
    let with = |extra: (Square, Color, PieceKind)| {
        let mut pieces = anchor.to_vec();
        pieces.push(extra);
        position_with_pieces(Color::Black, &pieces)
    };

    // (1) 1枚の位置だけが異なる対。
    assert_ne!(
        with((sq(4, 4), Color::Black, PieceKind::GoldGeneral)).zobrist(),
        with((sq(4, 5), Color::Black, PieceKind::GoldGeneral)).zobrist()
    );
    // (2) 同一升・同一所有者で駒種だけが異なる対。
    assert_ne!(
        with((sq(4, 4), Color::Black, PieceKind::GoldGeneral)).zobrist(),
        with((sq(4, 4), Color::Black, PieceKind::SilverGeneral)).zobrist()
    );
    // (3) 同一升・同一駒種で所有者だけが異なる対。
    assert_ne!(
        with((sq(4, 4), Color::Black, PieceKind::Pawn)).zobrist(),
        with((sq(4, 4), Color::White, PieceKind::Pawn)).zobrist()
    );

    // (4) 成否だけが異なる対: 生の金将と歩兵の成駒、生の獅子と麒麟の成駒、
    // 生の醉象と仲人の成駒。
    let anchor_codes = [
        (
            sq(0, 0),
            PieceCode::new(Color::Black, PieceKind::King).unwrap(),
        ),
        (
            sq(11, 11),
            PieceCode::new(Color::White, PieceKind::King).unwrap(),
        ),
    ];
    for kind in [
        PieceKind::GoldGeneral,
        PieceKind::Lion,
        PieceKind::DrunkElephant,
    ] {
        let with_code = |code| {
            let mut pieces = anchor_codes.to_vec();
            pieces.push((sq(4, 4), code));
            position_from_codes(Color::Black, &pieces)
        };
        assert_ne!(
            with_code(PieceCode::new(Color::Black, kind).unwrap()).zobrist(),
            with_code(PieceCode::new_promoted(Color::Black, kind).unwrap()).zobrist(),
            "{kind:?}"
        );
    }
}

// 局面キーは手番側を区別し、その寄与はどの盤面でも現れる(第24条1項b、D4-024-02)。
#[test]
fn article_24_1_b_key_distinguishes_side_to_move_on_any_board() {
    assert_ne!(
        Position::empty(Color::Black).zobrist(),
        Position::empty(Color::White).zobrist()
    );

    let pieces = [
        (sq(4, 4), Color::Black, PieceKind::King),
        (sq(7, 7), Color::White, PieceKind::King),
    ];
    assert_ne!(
        position_with_pieces(Color::Black, &pieces).zobrist(),
        position_with_pieces(Color::White, &pieces).zobrist()
    );

    // 初期配置の盤面で手番だけを後手にした局面は、初期局面と同一局面ではない。
    let initial = Position::initial();
    let flipped = rebuild_board_with_side(&initial, Color::White);
    assert!(Square::all().all(|square| initial.piece_at(square) == flipped.piece_at(square)));
    assert_ne!(initial.zobrist(), flipped.zobrist());
}

// 局面キーは先獅子による直後の捕獲禁止の有無を反映する(第24条1項c、D4-024-03)。
#[test]
fn article_24_1_c_key_reflects_lion_recapture_state() {
    let generator = MoveGenerator::standard();

    // 非獅子が獅子を取った直後は、同一盤面・同一手番の禁止なし局面とキーが異なる。
    let (with_trigger, _) = position_after_non_lion_captures_lion();
    let same_board_without = rebuild_board_with_side(&with_trigger, with_trigger.side_to_move());
    assert!(
        Square::all()
            .all(|square| with_trigger.piece_at(square) == same_board_without.piece_at(square))
    );
    assert_ne!(with_trigger.zobrist(), same_board_without.zobrist());

    // 獅子が獅子を取った場合は先獅子が成立せず(第15条6項)、禁止なしの直接構築と一致する。
    let mut lion_takes_lion = position_with_pieces(
        Color::Black,
        &[
            (sq(4, 4), Color::Black, PieceKind::Lion),
            (sq(5, 4), Color::White, PieceKind::Lion),
        ],
    );
    lion_takes_lion
        .try_make_move(
            Move {
                from: sq(4, 4),
                mid: None,
                to: sq(5, 4),
                promote: false,
            },
            &generator,
        )
        .unwrap();
    assert_eq!(
        lion_takes_lion,
        rebuild_board_with_side(&lion_takes_lion, Color::White)
    );

    // 麒麟が獅子を取って同じ着手で成った場合、標準規則では先獅子が成立し得る(第15条7項)。
    let mut kirin = position_with_pieces(
        Color::Black,
        &[
            (sq(4, 7), Color::Black, PieceKind::Kirin),
            (sq(5, 8), Color::White, PieceKind::Lion),
        ],
    );
    kirin
        .try_make_move(
            Move {
                from: sq(4, 7),
                mid: None,
                to: sq(5, 8),
                promote: true,
            },
            &generator,
        )
        .unwrap();
    assert_ne!(
        kirin.zobrist(),
        rebuild_board_with_side(&kirin, Color::White).zobrist()
    );

    // 相手が別の着手を行うと禁止は消滅し(第15条5項)、禁止なしの同一配置とキーが一致する。
    let (mut cleared, _) = position_after_non_lion_captures_lion();
    cleared
        .try_make_move(
            Move {
                from: sq(10, 10),
                mid: None,
                to: sq(10, 9),
                promote: false,
            },
            &generator,
        )
        .unwrap();
    assert_eq!(cleared, rebuild_board_with_side(&cleared, Color::Black));
}

// 盤上に獅子が複数ある局面では、禁止の対象升が異なればキーも異なる
// (第24条1項c、D4-024-03境界)。
#[test]
fn article_24_1_c_key_distinguishes_the_prohibition_target_square() {
    let base = position_from_codes(
        Color::Black,
        &[
            (
                sq(2, 2),
                PieceCode::new(Color::White, PieceKind::Lion).unwrap(),
            ),
            (
                sq(9, 9),
                PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap(),
            ),
            (
                sq(5, 5),
                PieceCode::new(Color::Black, PieceKind::Lion).unwrap(),
            ),
        ],
    );

    let mut at_first = base.clone();
    at_first.set_lion_capture(Some(sq(3, 3))).unwrap();
    let mut at_second = base.clone();
    at_second.set_lion_capture(Some(sq(6, 6))).unwrap();
    assert_ne!(at_first.zobrist(), base.zobrist());
    assert_ne!(at_first.zobrist(), at_second.zobrist());

    // 禁止を解除すると元のキーへ完全に戻る。
    at_first.set_lion_capture(None).unwrap();
    assert_eq!(at_first, base);

    // 手番側の駒がある升は獅子捕獲升として受理されず、局面は変化しない。
    let mut rejected = base.clone();
    assert_eq!(
        rejected.set_lion_capture(Some(sq(5, 5))),
        Err(PositionError::InvalidLionCapture { square: sq(5, 5) })
    );
    assert_eq!(rejected, base);
}

// 構成要素d(成り権保留(P1・P2・P5))は盤面キー(a〜c)と分離して保持される(第24条1項d、
// D4-024-04)。R2・R3の同一局面判定はa〜cのみ、R1はd込みで行う(第31条)ため、
// dだけが異なる2局面は盤面キーが一致し権利キーだけが異なる。
#[test]
fn article_24_1_d_promotion_rights_key_is_separate_from_the_board_key() {
    let deferred_square = sq(4, 9);
    let piece = PieceCode::new(Color::Black, PieceKind::SilverGeneral).unwrap();
    let mut plain_builder = PositionBuilder::new(Color::Black);
    plain_builder.put(deferred_square, piece).unwrap();
    let plain = plain_builder.finish().unwrap();

    let mut deferred_builder = PositionBuilder::new(Color::Black);
    deferred_builder.put(deferred_square, piece).unwrap();
    deferred_builder
        .mark_promotion_deferred(deferred_square)
        .unwrap();
    let deferred = deferred_builder.finish().unwrap();

    assert_eq!(plain.zobrist(), deferred.zobrist());
    assert_ne!(plain.rights_zobrist(), deferred.rights_zobrist());

    // 保留状態を持てるのは敵陣内(第3条6項)の未成の成れる駒だけである(第30条P1)。
    // 空升・成れない駒(王将)・成駒・敵陣外の駒への保留指定は拒否される。
    let invalid = |square| {
        Err(PositionBuildError::InvalidPosition(
            PositionError::InvalidPromotionDeferred { square },
        ))
    };
    let mut empty_builder = PositionBuilder::new(Color::Black);
    assert_eq!(
        empty_builder.mark_promotion_deferred(sq(4, 9)),
        invalid(sq(4, 9))
    );

    let cases = [
        (
            PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            sq(5, 9),
        ),
        (
            PieceCode::new(Color::Black, PieceKind::SilverGeneral)
                .unwrap()
                .promote()
                .unwrap(),
            sq(6, 9),
        ),
        (
            PieceCode::new(Color::Black, PieceKind::SilverGeneral).unwrap(),
            sq(4, 7),
        ),
    ];
    for (code, square) in cases {
        let mut builder = PositionBuilder::new(Color::Black);
        builder.put(square, code).unwrap();
        assert_eq!(builder.mark_promotion_deferred(square), invalid(square));
    }
}

// 局面キーは到達手順・手数・盤外情報に依存しない(第24条3項、D4-024-05)。
#[test]
fn article_24_3_key_depends_on_the_position_not_on_the_path() {
    let generator = MoveGenerator::standard();
    let start = || {
        position_with_pieces(
            Color::Black,
            &[
                (sq(5, 0), Color::Black, PieceKind::King),
                (sq(4, 4), Color::Black, PieceKind::GoldGeneral),
                (sq(6, 11), Color::White, PieceKind::King),
                (sq(7, 7), Color::White, PieceKind::GoldGeneral),
            ],
        )
    };
    let mv = |from, to| Move {
        from,
        mid: None,
        to,
        promote: false,
    };
    let play = |steps: &[Move]| {
        let mut position = start();
        for &step in steps {
            position.try_make_move(step, &generator).unwrap();
        }
        position
    };

    // 手順の入れ替え(transposition)で同一局面へ到達してもキーは一致する。
    let path_a = play(&[
        mv(sq(4, 4), sq(4, 5)),
        mv(sq(7, 7), sq(7, 6)),
        mv(sq(5, 0), sq(5, 1)),
        mv(sq(6, 11), sq(6, 10)),
    ]);
    let path_b = play(&[
        mv(sq(5, 0), sq(5, 1)),
        mv(sq(6, 11), sq(6, 10)),
        mv(sq(4, 4), sq(4, 5)),
        mv(sq(7, 7), sq(7, 6)),
    ]);
    assert_eq!(path_a, path_b);

    // 往復を含む長い経路(手数が異なる)でも、同じ局面なら同じキーになる。
    let path_long = play(&[
        mv(sq(4, 4), sq(4, 5)),
        mv(sq(7, 7), sq(7, 6)),
        mv(sq(5, 0), sq(5, 1)),
        mv(sq(6, 11), sq(6, 10)),
        mv(sq(4, 5), sq(4, 6)),
        mv(sq(6, 10), sq(6, 11)),
        mv(sq(4, 6), sq(4, 5)),
        mv(sq(6, 11), sq(6, 10)),
    ]);
    assert_eq!(path_long, path_a);
}

// 実装契約(D4-IMP-02): キーは局面状態の純関数であり、同一の目標局面は
// 構築経路(配置順序・指し手適用)によらず同一キーになる。
#[test]
fn equal_configurations_share_one_key_regardless_of_construction_path() {
    let generator = MoveGenerator::standard();

    // 歩兵の成り(第18条)を経由した局面と、成駒を直接配置した局面。
    let mut played = position_with_pieces(
        Color::Black,
        &[
            (sq(4, 7), Color::Black, PieceKind::Pawn),
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(6, 11), Color::White, PieceKind::King),
        ],
    );
    played
        .try_make_move(
            Move {
                from: sq(4, 7),
                mid: None,
                to: sq(4, 8),
                promote: true,
            },
            &generator,
        )
        .unwrap();
    let built = position_from_codes(
        Color::White,
        &[
            (
                sq(4, 8),
                PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap(),
            ),
            (
                sq(0, 0),
                PieceCode::new(Color::Black, PieceKind::King).unwrap(),
            ),
            (
                sq(6, 11),
                PieceCode::new(Color::White, PieceKind::King).unwrap(),
            ),
        ],
    );
    assert_eq!(played, built);
}

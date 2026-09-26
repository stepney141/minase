//! 成りを保留した駒の権利の管理の試験。

use crate::core::board::Bitboard;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::{MoveRules, PromotionRule};
use crate::test_util::{position as position_with_pieces, sq};

#[test]
fn promotion_deferred_returns_the_marked_square_set() {
    // 設計書predecessor-generator.md「Positionの公開面」
    let mut builder = PositionBuilder::new(Color::Black);
    for (square, color) in [
        (sq(4, 9), Color::Black),
        (sq(5, 2), Color::White),
        (sq(6, 9), Color::Black),
    ] {
        builder
            .put(
                square,
                PieceCode::new(color, PieceKind::SilverGeneral).unwrap(),
            )
            .unwrap();
    }
    let mut expected = Bitboard::EMPTY;
    for square in [sq(4, 9), sq(5, 2)] {
        builder.mark_promotion_deferred(square).unwrap();
        expected.set(square);
    }
    assert_eq!(builder.finish().unwrap().promotion_deferred(), expected);
    assert_eq!(Position::initial().promotion_deferred(), Bitboard::EMPTY);
}

#[test]
fn article_30_p2_waiting_right_survives_the_reply_and_expires_on_the_next_own_move() {
    // 第30条P2: 敵陣入りの不成で生じた待機状態は相手の応手では残り、
    // その側の直後の1手番で満了する。権利キーとundoも同じ遷移を保存する。
    let rules = MoveRules {
        promotion: PromotionRule::P2,
        ..MoveRules::standard()
    };
    let mut position = position_with_pieces(
        Color::Black,
        &[
            (sq(4, 7), Color::Black, PieceKind::SilverGeneral),
            (sq(10, 10), Color::White, PieceKind::GoldGeneral),
        ],
    );
    let entry = Move {
        from: sq(4, 7),
        mid: None,
        to: sq(4, 8),
        promote: false,
    };
    position.make_move_unchecked(entry, rules);
    assert!(position.promotion_deferred().contains(sq(4, 8)));
    assert_ne!(position.rights_zobrist(), 0);
    assert_eq!(
        position.rights_zobrist(),
        position.recompute_rights_zobrist()
    );

    position.make_move_unchecked(
        Move {
            from: sq(10, 10),
            mid: None,
            to: sq(10, 9),
            promote: false,
        },
        rules,
    );
    assert!(position.promotion_deferred().contains(sq(4, 8)));
    let before_expiry = position.clone();

    let undo = position.make_move_unchecked(
        Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 9),
            promote: false,
        },
        rules,
    );
    assert!(position.promotion_deferred().is_empty());
    assert_eq!(position.rights_zobrist(), 0);
    assert_eq!(
        position.rights_zobrist(),
        position.recompute_rights_zobrist()
    );

    position.unmake_move(undo);
    assert_eq!(position, before_expiry);
}

#[test]
fn article_30_p5_deferred_pawn_state_moves_with_an_unpromoted_pawn() {
    // 第30条P5: 敵陣入りで不成を選んだ歩兵の保留状態は、以後その歩兵が
    // 不成のまま前進しても消滅せず、移動先へ引き継がれる。
    let rules = MoveRules {
        p5: true,
        ..MoveRules::standard()
    };
    let mut position = position_with_pieces(
        Color::Black,
        &[
            (sq(4, 7), Color::Black, PieceKind::Pawn),
            (sq(10, 10), Color::White, PieceKind::GoldGeneral),
        ],
    );
    position.make_move_unchecked(
        Move {
            from: sq(4, 7),
            mid: None,
            to: sq(4, 8),
            promote: false,
        },
        rules,
    );
    position.make_move_unchecked(
        Move {
            from: sq(10, 10),
            mid: None,
            to: sq(10, 9),
            promote: false,
        },
        rules,
    );
    let old_rights = position.rights_zobrist();

    position.make_move_unchecked(
        Move {
            from: sq(4, 8),
            mid: None,
            to: sq(4, 9),
            promote: false,
        },
        rules,
    );
    assert!(!position.promotion_deferred().contains(sq(4, 8)));
    assert!(position.promotion_deferred().contains(sq(4, 9)));
    assert_ne!(position.rights_zobrist(), old_rights);
    assert_eq!(
        position.rights_zobrist(),
        position.recompute_rights_zobrist()
    );
}

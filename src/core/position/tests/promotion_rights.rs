//! 成りを保留した駒の権利の管理の試験。

use crate::core::mv::Move;
use crate::core::piece::{Color, PieceKind};
use crate::core::rules::{MoveRules, PromotionRule};
use crate::test_util::{position as position_with_pieces, sq};

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

    position.unmake_move(undo);
    assert_eq!(position, before_expiry);
}

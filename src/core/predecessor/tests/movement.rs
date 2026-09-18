use super::*;

#[test]
fn gold_steps_and_captures_for_both_sides_and_corners() {
    // 第7・9条、設計書「検証 > 固定テスト」: 金将の1升移動・捕獲・盤端を逆生成する。
    for color in Color::ALL {
        for corner in [false, true] {
            let rank = if color == Color::Black { 4 } else { 7 };
            let (from, to, forbidden) = if corner {
                if color == Color::Black {
                    (sq(0, 10), sq(0, 11), sq(1, 9))
                } else {
                    (sq(11, 1), sq(11, 0), sq(10, 2))
                }
            } else {
                (sq(5, rank), sq(6, rank), sq(3, rank))
            };
            for capture in [false, true] {
                let mut pieces = vec![(from, color, PieceKind::GoldGeneral)];
                if capture {
                    pieces.push((to, color.opposite(), PieceKind::Pawn));
                }
                let predecessor = position(color, &pieces);
                let result = round_trip(MoveRules::standard(), &predecessor, mv(from, to, false));
                pieces[0].0 = forbidden;
                assert!(!result.contains(&position(color, &pieces)));
            }
        }
    }
}

#[test]
fn rook_slides_short_long_and_captures() {
    // 第7・9条: 飛車は縦横に走り、到達升の相手駒を取れるが斜めには動けない。
    for distance in [1, 4] {
        for capture in [false, true] {
            let from = sq(2, 5);
            let to = sq(2 + distance, 5);
            let mut pieces = vec![(from, Color::Black, PieceKind::Rook)];
            if capture {
                pieces.push((to, Color::White, PieceKind::Pawn));
            }
            let result = round_trip(
                MoveRules::standard(),
                &position(Color::Black, &pieces),
                mv(from, to, false),
            );
            pieces[0].0 = sq(1, 4);
            assert!(!result.contains(&position(Color::Black, &pieces)));
        }
    }
}

#[test]
fn rook_cannot_reverse_through_friendly_or_enemy_blocker() {
    // 第7条4・5項: 捕獲駒を復元しても、途中の味方駒・相手駒は飛び越せない。
    for blocker_color in Color::ALL {
        let to = sq(7, 5);
        let mut pieces = vec![
            (sq(6, 5), Color::Black, PieceKind::Rook),
            (sq(4, 5), blocker_color, PieceKind::Pawn),
            (to, Color::White, PieceKind::SilverGeneral),
        ];
        let result = round_trip(
            MoveRules::standard(),
            &position(Color::Black, &pieces),
            mv(sq(6, 5), to, false),
        );
        pieces[0].0 = sq(3, 5);
        assert!(!result.contains(&position(Color::Black, &pieces)));
        pieces.pop();
        assert!(!result.contains(&position(Color::Black, &pieces)));
    }
}

#[test]
fn fixed_jumps_ignore_intermediate_occupancy_and_capture_at_destination() {
    // 第7条6・7項、第9条: 麒麟の縦横2升跳びは中間の駒を残し、到達升だけで捕獲する。
    for blocker in [None, Some(Color::Black), Some(Color::White)] {
        for capture in [false, true] {
            let from = sq(3, 5);
            let to = sq(5, 5);
            let mut pieces = vec![(from, Color::Black, PieceKind::Kirin)];
            if let Some(color) = blocker {
                pieces.push((sq(4, 5), color, PieceKind::Pawn));
            }
            if capture {
                pieces.push((to, Color::White, PieceKind::SilverGeneral));
            }
            let result = round_trip(
                MoveRules::standard(),
                &position(Color::Black, &pieces),
                mv(from, to, false),
            );
            pieces[0].0 = sq(2, 5);
            assert!(!result.contains(&position(Color::Black, &pieces)));
        }
    }
}

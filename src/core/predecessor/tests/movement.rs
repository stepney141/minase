use super::*;

#[test]
fn gold_steps_and_captures_for_both_sides_and_corners() {
    // 第7・9条、設計書「検証 > 固定テスト」: 金将の1升移動・捕獲・盤端を逆生成する。
    for color in Color::ALL {
        for corner in [false, true] {
            let rank = if color == Color::Black { 4 } else { 7 };
            let (from, to) = if corner {
                if color == Color::Black {
                    (sq(0, 10), sq(0, 11))
                } else {
                    (sq(11, 1), sq(11, 0))
                }
            } else {
                (sq(5, rank), sq(6, rank))
            };
            for capture in [false, true] {
                let mut pieces = vec![(from, color, PieceKind::GoldGeneral)];
                if capture {
                    pieces.push((to, color.opposite(), PieceKind::Pawn));
                }
                let predecessor = position(color, &pieces);
                round_trip(MoveRules::standard(), &predecessor, mv(from, to, false));
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
            round_trip(
                MoveRules::standard(),
                &position(Color::Black, &pieces),
                mv(from, to, false),
            );
        }
    }
}

#[test]
fn fixed_jumps_ignore_intermediate_occupancy_and_capture_at_destination() {
    // 第7条6・7項、第9条: 麒麟の縦横2升跳びは中間の駒を残し、到達升だけで捕獲する。
    for blocker in [None, Some(Color::Black)] {
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
            round_trip(
                MoveRules::standard(),
                &position(Color::Black, &pieces),
                mv(from, to, false),
            );
        }
    }
}

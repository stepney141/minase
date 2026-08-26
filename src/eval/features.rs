//! 学習評価関数で共有する駒状態と手番側視点の特徴番号。

use crate::core::piece::PIECE_KIND_COUNT;
use crate::{Color, PieceCode, PieceKind, Position, Square};

/// 学習評価関数が使う特徴の総数。
pub const FEATURE_COUNT: usize = 13_680;
/// 駒種と現在の成り可否を区別した駒状態の総数。
pub const PIECE_STATE_COUNT: usize = 47;
/// 駒と升の組からなる特徴の総数。
const BOARD_FEATURE_COUNT: usize = 2 * PIECE_STATE_COUNT * 144;
/// 成っていない駒の駒種番号から状態番号への表。成れる駒種は29以降の
/// 「成れる」状態、それ以外は駒種番号そのものへ写す。
const UNPROMOTED_STATES: [u8; PIECE_KIND_COUNT] = build_unpromoted_states();

/// 成っていない駒の状態表を作る。成れる駒種には駒種番号順に29から番号を振る。
const fn build_unpromoted_states() -> [u8; PIECE_KIND_COUNT] {
    let mut table = [0_u8; PIECE_KIND_COUNT];
    let mut next_promotable_state = 29_u8;
    let mut index = 0;
    while index < PIECE_KIND_COUNT {
        if PieceKind::ALL[index].can_promote() {
            table[index] = next_promotable_state;
            next_promotable_state += 1;
        } else {
            table[index] = index as u8;
        }
        index += 1;
    }
    table
}

/// 駒コードを駒種と現在の成り可否からなる状態番号へ変換する。
pub fn piece_state(piece: PieceCode) -> usize {
    let kind = piece.kind().expect("feature piece must have a valid kind");
    if piece.is_promoted() {
        kind.index()
    } else {
        UNPROMOTED_STATES[kind.index()] as usize
    }
}

/// 指定視点における盤上の駒の特徴番号を返す。
pub fn feature_index(perspective: Color, piece: PieceCode, square: Square) -> usize {
    let relative_color = usize::from(
        piece
            .color()
            .expect("feature piece must have a valid color")
            != perspective,
    );
    let relative_rank = match perspective {
        Color::Black => square.rank(),
        Color::White => 11 - square.rank(),
    };
    let relative_square = relative_rank as usize * 12 + square.file() as usize;
    (relative_color * PIECE_STATE_COUNT + piece_state(piece)) * 144 + relative_square
}

/// 指定視点における先獅子対象升の特徴番号を返す。
pub fn lion_feature_index(perspective: Color, square: Square) -> usize {
    let relative_rank = match perspective {
        Color::Black => square.rank(),
        Color::White => 11 - square.rank(),
    };
    BOARD_FEATURE_COUNT + relative_rank as usize * 12 + square.file() as usize
}

/// 局面で有効な指定視点の特徴番号を列挙する。
pub fn active_features_for(perspective: Color, position: &Position, mut f: impl FnMut(usize)) {
    for square in position.occupied() {
        let piece = position
            .piece_at(square)
            .expect("occupied square must contain a piece");
        f(feature_index(perspective, piece, square));
    }
    if let Some(trigger) = position.lion_taken_by_non_lion() {
        f(lion_feature_index(perspective, trigger.square));
    }
}

/// 局面で有効な手番側視点の特徴番号を列挙する。
pub fn active_features(position: &Position, f: impl FnMut(usize)) {
    active_features_for(position.side_to_move(), position, f);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Position;
    use crate::test_util::{position_from_codes, sq};

    /// 駒コードの全定義域が47状態へ仕様どおり写ることを検査する。
    #[test]
    fn every_piece_code_maps_to_the_specified_state() {
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                let piece = PieceCode::new(color, kind);
                let expected = if kind.can_promote() {
                    29 + PieceKind::ALL[..kind.index()]
                        .iter()
                        .filter(|kind| kind.can_promote())
                        .count()
                } else {
                    kind.index()
                };
                assert_eq!(piece_state(piece), expected);
                assert!(piece_state(piece) < PIECE_STATE_COUNT);

                if let Some(piece) = PieceCode::new_promoted(color, kind) {
                    assert_eq!(piece_state(piece), kind.index());
                    assert!(piece_state(piece) < PIECE_STATE_COUNT);
                }
            }
        }

        let lion = PieceCode::new(Color::Black, PieceKind::Lion);
        let promoted_kirin = PieceCode::new_promoted(Color::Black, PieceKind::Lion).unwrap();
        assert_eq!(piece_state(lion), piece_state(promoted_kirin));
        let gold = PieceCode::new(Color::Black, PieceKind::GoldGeneral);
        let promoted_pawn = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
        assert_ne!(piece_state(gold), piece_state(promoted_pawn));
    }

    /// 段反転と陣営交換が後手視点を先手視点へ写すことを検査する。
    #[test]
    fn white_features_match_rank_reflection_with_colors_swapped() {
        let mut positions = vec![Position::initial()];
        let promoted_gold = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted_lion = PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap();
        let fixture = position_from_codes(
            Color::White,
            &[
                (sq(1, 2), promoted_gold),
                (sq(7, 8), promoted_lion),
                (sq(4, 6), PieceCode::new(Color::White, PieceKind::King)),
            ],
        );
        positions.push(fixture);

        for mut position in positions {
            if position.side_to_move() == Color::Black {
                let pieces: Vec<_> = Square::all()
                    .filter_map(|square| position.piece_at(square).map(|piece| (square, piece)))
                    .collect();
                position = position_from_codes(Color::White, &pieces);
            } else {
                position.set_lion_capture(Some(sq(3, 5))).unwrap();
            }
            let mut white_features = Vec::new();
            active_features(&position, |feature| white_features.push(feature));
            white_features.sort_unstable();

            let reflected_pieces: Vec<_> = Square::all()
                .filter_map(|square| {
                    position.piece_at(square).map(|piece| {
                        let color = piece.color().unwrap().opposite();
                        let kind = piece.kind().unwrap();
                        let reflected_piece = if piece.is_promoted() {
                            PieceCode::new_promoted(color, kind).unwrap()
                        } else {
                            PieceCode::new(color, kind)
                        };
                        (sq(square.file(), 11 - square.rank()), reflected_piece)
                    })
                })
                .collect();
            let mut reflected = position_from_codes(Color::Black, &reflected_pieces);
            if position.lion_taken_by_non_lion().is_some() {
                reflected.set_lion_capture(Some(sq(3, 6))).unwrap();
            }
            let mut black_features = Vec::new();
            active_features(&reflected, |feature| black_features.push(feature));
            black_features.sort_unstable();
            assert_eq!(white_features, black_features);
        }
    }

    /// 盤上の駒数と先獅子状態が有効特徴数へそのまま反映されることを検査する。
    #[test]
    fn active_feature_count_matches_pieces_and_optional_lion_square() {
        let initial = Position::initial();
        let mut features = Vec::new();
        active_features(&initial, |feature| features.push(feature));
        assert_eq!(features.len(), 92);

        let mut with_lion = position_from_codes(
            Color::Black,
            &[(sq(5, 0), PieceCode::new(Color::Black, PieceKind::King))],
        );
        with_lion.set_lion_capture(Some(sq(4, 4))).unwrap();
        let mut features = Vec::new();
        active_features(&with_lion, |feature| features.push(feature));
        assert_eq!(features.len(), 2);
    }
}

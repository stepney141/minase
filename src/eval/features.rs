//! 学習評価関数で共有する駒状態と手番側視点の特徴番号。

use crate::core::piece::PIECE_KIND_COUNT;
use crate::{Color, PieceCode, PieceKind, Position, Square};

/// 学習評価関数が使う特徴の総数。
pub(super) const FEATURE_COUNT: usize = 13_680;
/// 駒種と現在の成り可否を区別した駒状態の総数。
pub(super) const PIECE_STATE_COUNT: usize = 47;
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
pub(super) fn piece_state(piece: PieceCode) -> usize {
    let kind = piece.kind().expect("feature piece must have a valid kind");
    if piece.is_promoted() {
        kind.index()
    } else {
        UNPROMOTED_STATES[kind.index()] as usize
    }
}

/// 指定視点における盤上の駒の特徴番号を返す。
pub(super) fn feature_index(perspective: Color, piece: PieceCode, square: Square) -> usize {
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
pub(super) fn lion_feature_index(perspective: Color, square: Square) -> usize {
    let relative_rank = match perspective {
        Color::Black => square.rank(),
        Color::White => 11 - square.rank(),
    };
    BOARD_FEATURE_COUNT + relative_rank as usize * 12 + square.file() as usize
}

/// 局面で有効な手番側視点の特徴番号を列挙する。
pub(super) fn active_features(position: &Position, mut f: impl FnMut(usize)) {
    active_features_for(position.side_to_move(), position, &mut f);
}

/// 局面で有効な指定視点の特徴番号を列挙する。
pub(super) fn active_features_for(
    perspective: Color,
    position: &Position,
    mut f: impl FnMut(usize),
) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Position;
    use crate::test_util::{position_from_codes, sq};

    /// 駒コードの全定義域が47状態へ仕様どおり写ることを検査する。
    #[test]
    fn every_piece_code_maps_to_the_specified_state() {
        // evaluation.md「入力特徴」の交換形式を固定する。右2列は未成コードと成駒コードの状態番号。
        let states = [
            (PieceKind::Pawn, Some(29), None),
            (PieceKind::GoBetween, Some(30), None),
            (PieceKind::Lance, Some(31), None),
            (PieceKind::ReverseChariot, Some(32), None),
            (PieceKind::SideMover, Some(33), Some(4)),
            (PieceKind::VerticalMover, Some(34), Some(5)),
            (PieceKind::Bishop, Some(35), Some(6)),
            (PieceKind::Rook, Some(36), Some(7)),
            (PieceKind::DragonHorse, Some(37), Some(8)),
            (PieceKind::DragonKing, Some(38), Some(9)),
            (PieceKind::FreeKing, Some(10), Some(10)),
            (PieceKind::King, Some(11), None),
            (PieceKind::DrunkElephant, Some(39), Some(12)),
            (PieceKind::FerociousLeopard, Some(40), None),
            (PieceKind::BlindTiger, Some(41), None),
            (PieceKind::CopperGeneral, Some(42), None),
            (PieceKind::SilverGeneral, Some(43), None),
            (PieceKind::GoldGeneral, Some(44), Some(17)),
            (PieceKind::Kirin, Some(45), None),
            (PieceKind::Phoenix, Some(46), None),
            (PieceKind::Lion, Some(20), Some(20)),
            (PieceKind::CrownPrince, None, Some(21)),
            (PieceKind::WhiteHorse, None, Some(22)),
            (PieceKind::Whale, None, Some(23)),
            (PieceKind::FlyingOx, None, Some(24)),
            (PieceKind::FreeBoar, None, Some(25)),
            (PieceKind::FlyingStag, None, Some(26)),
            (PieceKind::HornedFalcon, None, Some(27)),
            (PieceKind::SoaringEagle, None, Some(28)),
        ];
        for color in Color::ALL {
            for (kind, unpromoted, promoted) in states {
                if let Some(expected) = unpromoted {
                    assert_eq!(piece_state(PieceCode::new(color, kind).unwrap()), expected);
                }
                if let Some(expected) = promoted {
                    assert_eq!(
                        piece_state(PieceCode::new_promoted(color, kind).unwrap()),
                        expected
                    );
                }
            }
        }
    }

    /// 段反転と陣営交換が後手視点を先手視点へ写すことを検査する。
    #[test]
    fn white_features_match_rank_reflection_with_colors_swapped() {
        let promoted_gold = PieceCode::new_promoted(Color::Black, PieceKind::GoldGeneral).unwrap();
        let promoted_lion = PieceCode::new_promoted(Color::White, PieceKind::Lion).unwrap();
        let fixture = position_from_codes(
            Color::White,
            &[
                (sq(1, 2), promoted_gold),
                (sq(7, 8), promoted_lion),
                (
                    sq(4, 6),
                    PieceCode::new(Color::White, PieceKind::King).unwrap(),
                ),
            ],
        );

        for (mut position, expected_count) in [(Position::initial(), 92), (fixture, 4)] {
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
            assert_eq!(white_features.len(), expected_count);
            white_features.sort_unstable();

            let reflected_pieces: Vec<_> = Square::all()
                .filter_map(|square| {
                    position.piece_at(square).map(|piece| {
                        let color = piece.color().unwrap().opposite();
                        let kind = piece.kind().unwrap();
                        let reflected_piece = if piece.is_promoted() {
                            PieceCode::new_promoted(color, kind).unwrap()
                        } else {
                            PieceCode::new(color, kind).unwrap()
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
}

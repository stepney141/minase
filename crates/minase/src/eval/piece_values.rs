//! 探索と学習器が共有する、v0の駒割に基づく47状態の固定表。
//!
//! 探索は静的交換評価、捕獲手の並べ替え、歩兵価値に基づく枝刈りの幅、
//! および成りの得の最大値にこの表を使う。tataraの形式には駒価値の欄がないため、
//! ネットのファイルには含めない。A型とHalfKP_vmの世代反復をv0から始めるので、
//! 第1世代から後の世代まで探索用の値を固定するためにv0の駒割を出どころとする。

use super::{
    features::{INVALID_PIECE_STATE, PIECE_STATES_BY_CODE},
    handcrafted,
};
use minase_core::{PieceCode, PieceKind};

const VALUES: [i32; 47] = build_values();
const INVALID_PIECE_VALUE: i32 = i32::MIN;
const PIECE_VALUES_BY_CODE: [i32; 256] = build_piece_values_by_code();

const fn build_piece_values_by_code() -> [i32; 256] {
    let mut values = [INVALID_PIECE_VALUE; 256];
    let mut code = 0;
    while code < values.len() {
        let state = PIECE_STATES_BY_CODE[code];
        if state != INVALID_PIECE_STATE {
            values[code] = piece_value_of_state(state as usize);
        }
        code += 1;
    }
    values
}

const fn build_values() -> [i32; 47] {
    let mut values = [0; 47];
    let mut kind = 0;
    let mut unpromoted = 29;
    while kind < PieceKind::ALL.len() {
        let piece = PieceKind::ALL[kind];
        values[kind] = handcrafted::piece_value(piece);
        if piece.can_promote() {
            values[unpromoted] = handcrafted::piece_value(piece);
            unpromoted += 1;
        }
        kind += 1;
    }
    values
}

/// 駒状態の固定価値をセンチポーンで返す。
/// 0〜28は駒種番号、29〜46は成れる18種の駒種番号順に対応する。
///
/// # Panics
/// `state`が47以上ならパニックする。
pub const fn piece_value_of_state(state: usize) -> i32 {
    VALUES[state]
}

/// 現在の駒種と成り可否に対応する固定価値を返す。
/// 駒コードを添字として定数の表を引く。
///
/// # Panics
/// 空升または盤外の番兵ならパニックする。
pub fn piece_value(piece: PieceCode) -> i32 {
    let value = PIECE_VALUES_BY_CODE[piece.table_index()];
    assert!(
        value != INVALID_PIECE_VALUE,
        "feature piece must have a valid kind"
    );
    value
}

/// 成っていない歩兵の価値100を返す。
pub const fn pawn_value() -> i32 {
    piece_value_of_state(29)
}

/// 成れる18種の成りによる価値の差と0の最大値を返す。
pub const fn max_promotion_gain() -> i32 {
    MAX_PROMOTION_GAIN
}

const MAX_PROMOTION_GAIN: i32 = 2115;

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::Color;

    #[test]
    fn promotion_bound_matches_all_promotable_kinds() {
        let actual = PieceKind::ALL
            .into_iter()
            .filter(|kind| kind.can_promote())
            .map(|kind| {
                let before = PieceCode::new(Color::Black, kind).unwrap();
                piece_value(before.promote().unwrap()) - piece_value(before)
            })
            .max()
            .unwrap()
            .max(0);
        assert_eq!(max_promotion_gain(), actual);
    }

    #[test]
    fn all_states_follow_v0_piece_kinds() {
        let kinds: Vec<_> = PieceKind::ALL
            .into_iter()
            .chain(PieceKind::ALL.into_iter().filter(|kind| kind.can_promote()))
            .collect();
        assert_eq!(kinds.len(), 47);
        for (state, kind) in kinds.into_iter().enumerate() {
            assert_eq!(piece_value_of_state(state), handcrafted::piece_value(kind));
        }
        for (state, value) in [
            (11, 2600),
            (21, 2600),
            (20, 2500),
            (29, 100),
            (17, 378),
            (44, 378),
        ] {
            assert_eq!(piece_value_of_state(state), value);
        }
        let nonroyal: Vec<_> = (0..47)
            .filter(|s| ![11, 21].contains(s))
            .map(piece_value_of_state)
            .collect();
        assert!(nonroyal.iter().all(|&v| v > 0));
        assert_eq!(nonroyal.iter().max().unwrap() + pawn_value(), 2600);
        assert_eq!(max_promotion_gain(), 2115);
    }

    #[test]
    fn every_piece_code_has_the_value_of_its_reference_state() {
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                for piece in [
                    PieceCode::new(color, kind),
                    PieceCode::new_promoted(color, kind),
                ]
                .into_iter()
                .flatten()
                {
                    let kind = piece.kind().unwrap();
                    let state = if piece.is_promoted() || !kind.can_promote() {
                        kind.index()
                    } else {
                        PieceKind::ALL.len()
                            + PieceKind::ALL[..kind.index()]
                                .iter()
                                .filter(|kind| kind.can_promote())
                                .count()
                    };
                    assert_eq!(piece_value(piece), piece_value_of_state(state), "{piece:?}");
                }
            }
        }
    }

    #[test]
    #[should_panic(expected = "feature piece must have a valid kind")]
    fn piece_value_rejects_empty() {
        piece_value(PieceCode::EMPTY);
    }

    #[test]
    #[should_panic(expected = "feature piece must have a valid kind")]
    fn piece_value_rejects_wall() {
        piece_value(PieceCode::WALL);
    }
}

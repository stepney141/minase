//! 探索と学習器が共有する、v0の駒割に基づく47状態の固定表。
//!
//! 探索は静的交換評価、捕獲手の並べ替え、歩兵価値に基づく枝刈りの幅、
//! および成りの得の最大値にこの表を使う。tataraの形式には駒価値の欄がないため、
//! ネットのファイルには含めない。P型とHalfKP_vmの世代反復をv0から始めるので、
//! 第1世代から後の世代まで探索用の値を固定するためにv0の駒割を出どころとする。

use super::{features::piece_state, handcrafted};
use minase_core::{Color, PieceCode, PieceKind};

const VALUES: [i32; 47] = build_values();

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
///
/// # Panics
/// 空升または盤外の番兵ならパニックする。
pub fn piece_value(piece: PieceCode) -> i32 {
    piece_value_of_state(piece_state(piece))
}

/// 成っていない歩兵の価値100を返す。
pub const fn pawn_value() -> i32 {
    piece_value_of_state(29)
}

/// 成れる18種の成りによる価値の差と0の最大値を返す。
pub fn max_promotion_gain() -> i32 {
    let mut maximum = 0;
    for kind in PieceKind::ALL.into_iter().filter(|kind| kind.can_promote()) {
        let before = PieceCode::new(Color::Black, kind).expect("promotable kind is unpromoted");
        let after = before.promote().expect("promotable kind has a promotion");
        maximum = maximum.max(piece_value(after) - piece_value(before));
    }
    maximum
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for color in Color::ALL {
            for kind in PieceKind::ALL {
                if let Some(piece) = PieceCode::new(color, kind) {
                    assert_eq!(piece_value(piece), handcrafted::piece_value(kind));
                    if let Some(promoted) = piece.promote() {
                        assert_eq!(
                            piece_value(promoted),
                            handcrafted::piece_value(promoted.kind().unwrap())
                        );
                    }
                }
            }
        }
    }
}

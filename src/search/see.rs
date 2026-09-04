//! 到達升での駒の取り合いを見積もる静的交換評価。
//!
//! 経由升を持つ着手と、獅子を獅子または成れる麒麟が取る段階を
//! 含む交換列は、捕獲規則に依存するため判定不能とする。非獅子が獅子を
//! 取る手、獅子が非獅子を取る手、および角鷹と飛鷲が到達升だけで取る手は
//! 通常の交換として評価する。

use crate::core::bitboard::Bitboard;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::{MoveRules, PromotionChoice};
use crate::eval::Pst;

/// 交換列で保持できる利得の数。中将棋の盤上の駒は最大92枚である。
const MAX_GAINS: usize = 93;

/// 捕獲手について、到達升での駒の取り合いを手番側視点で見積もる。
///
/// 経由升を持つ着手、麒麟が獅子を取って成る着手、および到達升の
/// 獅子を獅子または成れる麒麟が取る段階を含む場合は`None`を返す。
/// これらは第14条、第15条第7項、および第16条の獅子捕獲規則に依存する。
/// その他の捕獲は、`docs/plans/strength-stage3.md`「静的交換評価」に従い、
/// 規則に依存しない通常の交換列として評価する。
pub(super) fn see(position: &Position, rules: MoveRules, pst: &Pst, mv: Move) -> Option<i32> {
    if mv.mid.is_some() {
        return None;
    }

    let moving_piece = piece_at(position, mv.from);
    let moving_kind = moving_piece.kind().expect("moving piece has a kind");
    let captured_piece = piece_at(position, mv.to);
    let captured_kind = captured_piece.kind().expect("captured piece has a kind");
    if moving_kind == PieceKind::Kirin && mv.promote && captured_kind == PieceKind::Lion {
        return None;
    }

    let piece_after_move = if mv.promote {
        moving_piece
            .promote()
            .expect("a promoting capture must move a promotable piece")
    } else {
        moving_piece
    };
    let mut gains = [0_i32; MAX_GAINS];
    gains[0] = pst.piece_value(captured_piece) + pst.piece_value(piece_after_move)
        - pst.piece_value(moving_piece);
    let mut piece_value = pst.piece_value(piece_after_move);
    let mut lion_on_square =
        moving_kind == PieceKind::Lion || (moving_kind == PieceKind::Kirin && mv.promote);
    let mut side = moving_piece
        .color()
        .expect("moving piece has a color")
        .opposite();
    let mut occupied = position.occupied();
    occupied.clear(mv.from);
    let mut depth = 0_usize;

    loop {
        let attackers = position.attackers_to(mv.to, occupied) & position.pieces_of(side);
        if lion_on_square && lion_capture_is_rule_dependent(position, rules, mv.to, side, attackers)
        {
            return None;
        }
        if attackers.is_empty() {
            break;
        }

        depth += 1;
        debug_assert!(depth < MAX_GAINS);
        gains[depth] = piece_value - gains[depth - 1];

        let next = least_valuable_attacker(position, pst, attackers);
        let next_piece = piece_at(position, next);
        let next_kind = next_piece.kind().expect("attacking piece has a kind");
        let promotion_choice = rules.promotion_choice_for(
            side,
            next_kind,
            next_piece.is_promoted(),
            next,
            mv.to,
            true,
            position.promotion_deferred().contains(next),
        );
        match promotion_choice {
            PromotionChoice::NoPromotion => {
                piece_value = pst.piece_value(next_piece);
            }
            PromotionChoice::PromotionOptional => {
                let promoted = promoted_piece(side, next_kind);
                let unpromoted_value = pst.piece_value(next_piece);
                let promoted_value = pst.piece_value(promoted);
                gains[depth] += (promoted_value - unpromoted_value).max(0);
                piece_value = unpromoted_value.max(promoted_value);
            }
            PromotionChoice::PromotionForced => {
                let promoted = promoted_piece(side, next_kind);
                let unpromoted_value = pst.piece_value(next_piece);
                let promoted_value = pst.piece_value(promoted);
                gains[depth] += promoted_value - unpromoted_value;
                piece_value = promoted_value;
            }
        }
        lion_on_square = next_kind == PieceKind::Lion
            || (next_kind == PieceKind::Kirin && promotion_choice != PromotionChoice::NoPromotion);

        occupied.clear(next);
        side = side.opposite();
    }

    while depth >= 1 {
        gains[depth - 1] = -(-gains[depth - 1]).max(gains[depth]);
        depth -= 1;
    }
    Some(gains[0])
}

/// 到達升の獅子を取ると規則依存になる攻撃駒が含まれるかを返す。
fn lion_capture_is_rule_dependent(
    position: &Position,
    rules: MoveRules,
    target: crate::Square,
    side: Color,
    attackers: Bitboard,
) -> bool {
    if attackers.intersects(position.pieces_of_kind(side, PieceKind::Lion)) {
        return true;
    }

    (attackers & position.pieces_of_kind(side, PieceKind::Kirin))
        .into_iter()
        .any(|from| {
            rules.promotion_choice_for(
                side,
                PieceKind::Kirin,
                false,
                from,
                target,
                true,
                position.promotion_deferred().contains(from),
            ) != PromotionChoice::NoPromotion
        })
}

/// 最小駒価値の攻撃駒を返す。同値なら生の升番号が小さい駒を選ぶ。
fn least_valuable_attacker(position: &Position, pst: &Pst, attackers: Bitboard) -> crate::Square {
    attackers
        .into_iter()
        .min_by_key(|&square| (pst.piece_value(piece_at(position, square)), square.raw()))
        .expect("non-empty attacker set has a least valuable piece")
}

/// 成り後の駒コードを返す。
fn promoted_piece(color: Color, kind: PieceKind) -> PieceCode {
    PieceCode::new_promoted(
        color,
        kind.promoted()
            .expect("promotion choice requires a promotable piece"),
    )
    .expect("promoted kind has a promoted piece code")
}

/// 指定升の駒を返す。
fn piece_at(position: &Position, square: crate::Square) -> PieceCode {
    position
        .piece_at(square)
        .expect("SEE square must contain a piece")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MoveGenerator;
    use crate::core::position::PositionBuilder;
    use crate::eval::weights;
    use crate::search::capture_is_pruned_by_see;
    use crate::test_util::{position_from_codes, sq};

    fn unpromoted(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new(color, kind).expect("test piece has an unpromoted state")
    }

    fn promoted(color: Color, kind: PieceKind) -> PieceCode {
        PieceCode::new_promoted(color, kind).expect("test piece is a promoted state")
    }

    fn position(side: Color, pieces: &[(crate::Square, PieceCode)]) -> Position {
        position_from_codes(side, pieces)
    }

    fn capture(from: crate::Square, to: crate::Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    fn value(pst: &Pst, piece: PieceCode) -> i32 {
        pst.piece_value(piece)
    }

    // strength-stage3.md「静的交換評価」: 2段階移動と、麒麟が
    // 獅子を取って成る初手は規則依存なので判定不能とする。
    #[test]
    fn see_returns_none_for_rule_dependent_initial_moves() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 5);
        let valuable_piece = unpromoted(Color::White, PieceKind::SilverGeneral);

        let two_stage = position(
            Color::Black,
            &[
                (sq(5, 7), unpromoted(Color::Black, PieceKind::Lion)),
                (sq(5, 6), valuable_piece),
                (target, unpromoted(Color::White, PieceKind::Lion)),
            ],
        );
        let attached_capture = Move {
            from: sq(5, 7),
            mid: Some(sq(5, 6)),
            to: target,
            promote: false,
        };
        assert_eq!(see(&two_stage, rules, &pst, attached_capture), None);

        let igui = Move {
            from: sq(5, 7),
            mid: Some(sq(5, 6)),
            to: sq(5, 7),
            promote: false,
        };
        assert_eq!(see(&two_stage, rules, &pst, igui), None);

        let kirin = position(
            Color::Black,
            &[
                (sq(4, 7), unpromoted(Color::Black, PieceKind::Kirin)),
                (sq(5, 8), unpromoted(Color::White, PieceKind::Lion)),
            ],
        );
        assert_eq!(
            see(
                &kirin,
                rules,
                &pst,
                Move {
                    from: sq(4, 7),
                    mid: None,
                    to: sq(5, 8),
                    promote: true,
                },
            ),
            None
        );
    }

    // strength-stage3.md「静的交換評価」: 到達升の獅子を、獅子または
    // 成れる麒麟が取る段階を含む交換列は判定不能とする。
    #[test]
    fn see_returns_none_for_rule_dependent_recaptures() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let victim = unpromoted(Color::White, PieceKind::Pawn);

        let cases: Vec<(Position, Move)> = vec![
            // 獅子で非獅子を取った後、距離2の相手獅子が取り返す。
            (
                position(
                    Color::Black,
                    &[
                        (sq(4, 4), unpromoted(Color::Black, PieceKind::Lion)),
                        (sq(5, 5), victim),
                        (sq(7, 7), unpromoted(Color::White, PieceKind::Lion)),
                    ],
                ),
                capture(sq(4, 4), sq(5, 5)),
            ),
            // 非獅子で獅子を取り、歩兵、自獅子と取り返した後、
            // 到達升の自獅子に相手獅子が届く。
            (
                position(
                    Color::Black,
                    &[
                        (sq(5, 4), unpromoted(Color::Black, PieceKind::Pawn)),
                        (sq(5, 5), unpromoted(Color::White, PieceKind::Lion)),
                        (sq(5, 6), unpromoted(Color::White, PieceKind::Pawn)),
                        (sq(7, 7), unpromoted(Color::Black, PieceKind::Lion)),
                        (sq(3, 3), unpromoted(Color::White, PieceKind::Lion)),
                    ],
                ),
                capture(sq(5, 4), sq(5, 5)),
            ),
            // 到達升の獅子を、敵陣へ入る麒麟が取って成れる。
            (
                position(
                    Color::Black,
                    &[
                        (sq(5, 2), unpromoted(Color::Black, PieceKind::Lion)),
                        (sq(5, 3), victim),
                        (sq(4, 4), unpromoted(Color::White, PieceKind::Kirin)),
                    ],
                ),
                capture(sq(5, 2), sq(5, 3)),
            ),
        ];

        for (board, mv) in cases {
            assert_eq!(see(&board, rules, &pst, mv), None, "move={mv:?}");
        }
    }

    // strength-stage3.md「静的交換評価」: 守りなし、損な高価駒の捕獲、x-ray、
    // および王駒による取り返しを手計算した交換列と照合する。
    #[test]
    fn see_values_match_hand_calculated_exchange_sequences() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 5);

        let black_pawn = unpromoted(Color::Black, PieceKind::Pawn);
        let white_go_between = unpromoted(Color::White, PieceKind::GoBetween);
        let unguarded = position(
            Color::Black,
            &[(sq(5, 4), black_pawn), (target, white_go_between)],
        );
        assert_eq!(
            see(&unguarded, rules, &pst, capture(sq(5, 4), target)),
            Some(value(&pst, white_go_between))
        );

        for white_lion in [
            unpromoted(Color::White, PieceKind::Lion),
            promoted(Color::White, PieceKind::Lion),
        ] {
            let lion_capture = position(
                Color::Black,
                &[
                    (sq(5, 0), unpromoted(Color::Black, PieceKind::Rook)),
                    (target, white_lion),
                ],
            );
            assert_eq!(
                see(&lion_capture, rules, &pst, capture(sq(5, 0), target)),
                Some(value(&pst, white_lion))
            );
        }

        let black_rook = unpromoted(Color::Black, PieceKind::Rook);
        let white_pawn = unpromoted(Color::White, PieceKind::Pawn);
        let white_rook = unpromoted(Color::White, PieceKind::Rook);
        let defended = position(
            Color::Black,
            &[
                (sq(5, 0), black_rook),
                (target, white_pawn),
                (sq(5, 10), white_rook),
            ],
        );
        assert_eq!(
            see(&defended, rules, &pst, capture(sq(5, 0), target)),
            Some(value(&pst, white_pawn) - value(&pst, black_rook))
        );

        let black_gold = unpromoted(Color::Black, PieceKind::GoldGeneral);
        let black_silver = unpromoted(Color::Black, PieceKind::SilverGeneral);
        let xray = position(
            Color::Black,
            &[
                (sq(4, 4), black_gold),
                (target, white_go_between),
                (sq(5, 6), white_pawn),
                (sq(6, 4), black_silver),
                (sq(5, 10), white_rook),
            ],
        );
        let mut expected = [0_i32; 4];
        expected[0] = value(&pst, white_go_between);
        expected[1] = value(&pst, black_gold) - expected[0];
        expected[2] = value(&pst, white_pawn) - expected[1];
        expected[3] = value(&pst, black_silver) - expected[2];
        for depth in (1..=3).rev() {
            expected[depth - 1] = -(-expected[depth - 1]).max(expected[depth]);
        }
        assert_eq!(
            see(&xray, rules, &pst, capture(sq(4, 4), target)),
            Some(expected[0])
        );

        let free_king = unpromoted(Color::White, PieceKind::FreeKing);
        let white_king = unpromoted(Color::White, PieceKind::King);
        let king_recapture = position(
            Color::Black,
            &[
                (sq(4, 4), black_gold),
                (target, free_king),
                (sq(5, 6), white_king),
                (sq(5, 4), black_pawn),
            ],
        );
        assert_eq!(
            see(&king_recapture, rules, &pst, capture(sq(4, 4), target)),
            Some(value(&pst, free_king))
        );
    }

    // RULES.md第14条第5項: 獅子が非獅子を取る段階は通常の交換とし、
    // 非獅子の取り返しと獅子による取り返しの価値を反映する。
    #[test]
    fn see_values_lion_captures_of_non_lions() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 5);
        let black_lion = unpromoted(Color::Black, PieceKind::Lion);
        let white_pawn = unpromoted(Color::White, PieceKind::Pawn);

        let lion_is_recaptured = position(
            Color::Black,
            &[
                (sq(4, 4), black_lion),
                (target, white_pawn),
                (sq(5, 6), white_pawn),
            ],
        );
        assert_eq!(
            see(&lion_is_recaptured, rules, &pst, capture(sq(4, 4), target)),
            Some(value(&pst, white_pawn) - value(&pst, black_lion))
        );

        let black_gold = unpromoted(Color::Black, PieceKind::GoldGeneral);
        let white_rook = unpromoted(Color::White, PieceKind::Rook);
        let lion_recaptures = position(
            Color::Black,
            &[
                (sq(4, 4), black_gold),
                (target, white_pawn),
                (sq(5, 10), white_rook),
                (sq(7, 7), black_lion),
            ],
        );
        let mut expected = [0_i32; 3];
        expected[0] = value(&pst, white_pawn);
        expected[1] = value(&pst, black_gold) - expected[0];
        expected[2] = value(&pst, white_rook) - expected[1];
        for depth in (1..=2).rev() {
            expected[depth - 1] = -(-expected[depth - 1]).max(expected[depth]);
        }
        assert_eq!(
            see(&lion_recaptures, rules, &pst, capture(sq(4, 4), target)),
            Some(expected[0])
        );
    }

    // RULES.md第11条: 角鷹と飛鷲は、2升目への特殊到達範囲だけで
    // 届く場合も、非獅子を1枚取る取り返しとして評価する。
    #[test]
    fn see_values_lion_like_special_recaptures() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 5);
        let mover = unpromoted(Color::Black, PieceKind::GoldGeneral);
        let victim = unpromoted(Color::White, PieceKind::Pawn);
        let cases = [
            position(
                Color::Black,
                &[
                    (sq(4, 4), mover),
                    (target, victim),
                    (sq(5, 6), unpromoted(Color::Black, PieceKind::Pawn)),
                    (sq(5, 7), promoted(Color::White, PieceKind::HornedFalcon)),
                ],
            ),
            position(
                Color::Black,
                &[
                    (sq(4, 4), mover),
                    (target, victim),
                    (sq(6, 6), unpromoted(Color::Black, PieceKind::Pawn)),
                    (sq(7, 7), promoted(Color::White, PieceKind::SoaringEagle)),
                ],
            ),
        ];
        let expected = value(&pst, victim) - value(&pst, mover);

        for (index, board) in cases.into_iter().enumerate() {
            assert_eq!(
                see(&board, rules, &pst, capture(sq(4, 4), target)),
                Some(expected),
                "case={index}"
            );
        }
    }

    // strength-stage3.md「静的交換評価」: 初手の成駒が失われる価値と、取り返す
    // 駒の任意成りによる正の差額を交換列へ含める。
    #[test]
    fn see_accounts_for_promotions_in_initial_and_recapture_moves() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 8);
        let dragon_horse = unpromoted(Color::Black, PieceKind::DragonHorse);
        let horned_falcon = promoted(Color::Black, PieceKind::HornedFalcon);
        let white_pawn = unpromoted(Color::White, PieceKind::Pawn);
        let first_promotion = position(
            Color::Black,
            &[
                (sq(4, 7), dragon_horse),
                (target, white_pawn),
                (sq(5, 9), white_pawn),
            ],
        );
        let promoting_capture = Move {
            from: sq(4, 7),
            mid: None,
            to: target,
            promote: true,
        };
        assert_eq!(
            see(&first_promotion, rules, &pst, promoting_capture),
            Some(value(&pst, white_pawn) - value(&pst, dragon_horse))
        );
        assert!(value(&pst, horned_falcon) > value(&pst, dragon_horse));

        let white_rook = unpromoted(Color::White, PieceKind::Rook);
        let black_pawn = unpromoted(Color::Black, PieceKind::Pawn);
        let promoted_recapture = position(
            Color::White,
            &[
                (sq(5, 10), white_rook),
                (target, black_pawn),
                (sq(4, 7), dragon_horse),
            ],
        );
        let promotion_bonus = (value(&pst, horned_falcon) - value(&pst, dragon_horse)).max(0);
        assert_eq!(
            see(&promoted_recapture, rules, &pst, capture(sq(5, 10), target)),
            Some(value(&pst, black_pawn) - value(&pst, white_rook) - promotion_bonus)
        );
    }

    // RULES.md第30条P5: 成りを保留した歩兵は、最奥段以外で取り返す場合に
    // 成れないため、交換列へ成りの差額を加えない。
    #[test]
    fn see_respects_p5_deferred_pawn_on_recapture() {
        let pst = weights().unwrap();
        let target = sq(5, 9);
        let white_rook = unpromoted(Color::White, PieceKind::Rook);
        let black_go_between = unpromoted(Color::Black, PieceKind::GoBetween);
        let deferred_pawn = unpromoted(Color::Black, PieceKind::Pawn);
        let mut builder = PositionBuilder::new(Color::White);
        builder.put(sq(5, 10), white_rook).unwrap();
        builder.put(target, black_go_between).unwrap();
        builder.put(sq(5, 8), deferred_pawn).unwrap();
        builder.mark_promotion_deferred(sq(5, 8)).unwrap();
        let board = builder.finish().unwrap();
        let rules = MoveRules {
            p5: true,
            ..MoveRules::standard()
        };

        assert_eq!(
            see(&board, rules, &pst, capture(sq(5, 10), target)),
            Some(value(&pst, black_go_between) - value(&pst, white_rook))
        );
    }

    // RULES.md第11条: 角鷹と飛鷲がmidなしで1枚だけ取る初手は、隣接移動でも
    // 2升目への直接跳びでも通常の交換として扱う。
    #[test]
    fn see_accepts_single_capture_lion_like_moves_without_mid() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let victim = unpromoted(Color::White, PieceKind::GoBetween);
        let cases = [
            (
                position(
                    Color::Black,
                    &[
                        (sq(5, 4), promoted(Color::Black, PieceKind::HornedFalcon)),
                        (sq(5, 5), victim),
                    ],
                ),
                capture(sq(5, 4), sq(5, 5)),
            ),
            (
                position(
                    Color::Black,
                    &[
                        (sq(5, 3), promoted(Color::Black, PieceKind::HornedFalcon)),
                        (sq(5, 4), unpromoted(Color::White, PieceKind::Pawn)),
                        (sq(5, 5), victim),
                    ],
                ),
                capture(sq(5, 3), sq(5, 5)),
            ),
            (
                position(
                    Color::Black,
                    &[
                        (sq(3, 3), promoted(Color::Black, PieceKind::SoaringEagle)),
                        (sq(4, 4), unpromoted(Color::White, PieceKind::Pawn)),
                        (sq(5, 5), victim),
                    ],
                ),
                capture(sq(3, 3), sq(5, 5)),
            ),
        ];

        for (board, mv) in cases {
            assert_eq!(see(&board, rules, &pst, mv), Some(value(&pst, victim)));
        }
    }

    // strength-stage3.md「検証」: 新しい判定不能条件に該当する捕獲は、
    // SEEの枝刈りで除外しない。
    #[test]
    fn see_none_never_prunes_quiescence_captures() {
        let pst = weights().unwrap();
        let rules = MoveRules::standard();
        let board = position(
            Color::Black,
            &[
                (sq(5, 5), unpromoted(Color::Black, PieceKind::Lion)),
                (sq(6, 6), unpromoted(Color::White, PieceKind::Pawn)),
                (sq(7, 7), unpromoted(Color::White, PieceKind::Lion)),
            ],
        );
        let generator = MoveGenerator::standard();
        let mut captures = Vec::new();
        generator.generate_captures(&board, &mut captures);
        let none_captures: Vec<_> = captures
            .iter()
            .copied()
            .filter(|&mv| see(&board, rules, &pst, mv).is_none())
            .collect();
        assert!(!none_captures.is_empty());
        assert!(
            none_captures
                .iter()
                .all(|&mv| { !capture_is_pruned_by_see(&board, rules, &pst, mv) })
        );
    }
}

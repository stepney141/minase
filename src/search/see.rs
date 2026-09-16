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

/// 捕獲の交換評価が負で、規則依存による判定不能でもない場合に枝刈りする。
///
/// 最初の取り返しで損をしないと確定したら、残る逆引きと逆算を省く。
/// 正確な評価値との契約は`docs/plans/movegen-speedup.md`「SEEの不要な反復を省く」。
pub(super) fn see_prunes(position: &Position, rules: MoveRules, pst: &Pst, mv: Move) -> bool {
    if mv.mid.is_some() {
        return false;
    }

    let moving_piece = piece_at(position, mv.from);
    let moving_kind = moving_piece.kind().expect("moving piece has a kind");
    let captured_piece = piece_at(position, mv.to);
    let captured_kind = captured_piece.kind().expect("captured piece has a kind");
    if moving_kind == PieceKind::Kirin && mv.promote && captured_kind == PieceKind::Lion {
        return false;
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
    // 同「段階6」: 成り益の上限で最初の取り返しも損をしないと分かる。
    if gains[0] >= 0
        && pst.piece_value(moving_piece) - pst.piece_value(captured_piece)
            + pst.max_promotion_gain()
            <= 0
    {
        return false;
    }
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
        #[cfg(test)]
        tests::LOOKUPS.set(tests::LOOKUPS.get() + 1);
        let attackers = position.attackers_to_by(side, mv.to, occupied);
        if lion_on_square && lion_capture_is_rule_dependent(position, rules, mv.to, side, attackers)
        {
            return false;
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
        // movegen-speedup.md「SEEの不要な反復を省く」:
        // 逆算結果は min(g0, max(-g1, X))。成り益を含むg1で判定する。
        // 後続が獅子規則に依存しても枝刈りしないので、この終了は安全である。
        if depth == 1 && gains[0] >= 0 && gains[1] <= 0 {
            return false;
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
    gains[0] < 0
}

/// 捕獲手について、到達升での駒の取り合いを手番側視点で見積もる。
///
/// 経由升を持つ着手、麒麟が獅子を取って成る着手、および到達升の
/// 獅子を獅子または成れる麒麟が取る段階を含む場合は`None`を返す。
/// これらは第14条、第15条第7項、および第16条の獅子捕獲規則に依存する。
/// その他の捕獲は、`docs/plans/strength-stage3.md`「静的交換評価」に従い、
/// 規則に依存しない通常の交換列として評価する。
#[cfg(test)]
fn see_reference(position: &Position, rules: MoveRules, pst: &Pst, mv: Move) -> Option<i32> {
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
        tests::LOOKUPS.set(tests::LOOKUPS.get() + 1);
        let attackers = position.attackers_to_by(side, mv.to, occupied);
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
    use crate::core::rules::Rules;
    use crate::eval::weights;
    use crate::search::capture_is_pruned_by_see;
    use crate::test_util::{bench_positions, position_from_codes, sampled_random_positions, sq};

    use core::cell::Cell;

    thread_local! {
        pub(super) static LOOKUPS: Cell<usize> = const { Cell::new(0) };
    }

    /// 参照値と枝刈り契約を検査し、両経路の逆引き回数を返す。
    fn assert_prune_contract(
        board: &Position,
        rules: MoveRules,
        pst: &Pst,
        mv: Move,
        expected: Option<i32>,
    ) -> (usize, usize) {
        LOOKUPS.set(0);
        let reference = see_reference(board, rules, pst, mv);
        let reference_lookups = LOOKUPS.get();
        assert_eq!(reference, expected, "move={mv:?}");
        LOOKUPS.set(0);
        assert_eq!(
            see_prunes(board, rules, pst, mv),
            reference.is_some_and(|v| v < 0),
            "move={mv:?}"
        );
        (LOOKUPS.get(), reference_lookups)
    }

    // movegen-speedup.md「SEEの不要な反復を省く」: 全捕獲の枝刈り判断を
    // benchの15局面と各規則32局面の固定シード対局で参照値と照合する。
    #[test]
    fn see_prunes_matches_reference_on_bench_and_random_captures() {
        let pst = weights().unwrap();
        for rules in [Rules::ENGINE_DEFAULT.moves, Rules::LISHOGI.moves] {
            let generator = MoveGenerator::new(rules);
            let mut checked = 0;
            for (index, board) in bench_positions()
                .into_iter()
                .chain(sampled_random_positions(rules))
                .enumerate()
            {
                let mut captures = Vec::new();
                generator.generate_captures(&board, &mut captures);
                for mv in captures {
                    assert_eq!(
                        see_prunes(&board, rules, &pst, mv),
                        see_reference(&board, rules, &pst, mv).is_some_and(|v| v < 0),
                        "rules={rules:?}, position={index}, move={mv:?}"
                    );
                    checked += 1;
                }
            }
            assert!(checked > 0);
        }
    }

    /// 「段階6」（movegen-speedup-2.md）の上限の両側と等号で逆引きの省略を検査する。
    #[test]
    fn see_prunes_skips_lookup_at_the_promotion_gain_bound() {
        use crate::eval::pst::{PIECE_STATE_COUNT, piece_state_of};
        use sha2::{Digest, Sha256};

        for captured_value in [999_i32, 1000, 1001] {
            // MNPTの駒価値表を直接指定する。最大の成り益は歩100→金1000の900。
            let mut bytes = include_bytes!("../../nets/pst-init.bin").to_vec();
            let base = bytes.len() - PIECE_STATE_COUNT * 4;
            for state in 0..PIECE_STATE_COUNT {
                bytes[base + state * 4..base + state * 4 + 4]
                    .copy_from_slice(&1000_i32.to_le_bytes());
            }
            let mover = unpromoted(Color::Black, PieceKind::Pawn);
            let victim = unpromoted(Color::White, PieceKind::FreeKing);
            let royal_value = captured_value.max(1000) + 100;
            for (state, value) in [
                (piece_state_of(mover), 100),
                (piece_state_of(victim), captured_value),
                (PieceKind::King.index(), royal_value),
                (PieceKind::CrownPrince.index(), royal_value),
            ] {
                bytes[base + state * 4..base + state * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            let checksum = Sha256::digest(&bytes[80..]);
            bytes[48..80].copy_from_slice(&checksum);
            let pst = Pst::decode(&bytes).unwrap();
            assert_eq!(pst.max_promotion_gain(), 900);
            let board = position(
                Color::Black,
                &[
                    (sq(5, 4), mover),
                    (sq(5, 5), victim),
                    (sq(5, 6), unpromoted(Color::White, PieceKind::Pawn)),
                ],
            );
            let counts = assert_prune_contract(
                &board,
                MoveRules::standard(),
                &pst,
                capture(sq(5, 4), sq(5, 5)),
                Some(captured_value - 100),
            );
            assert_eq!(counts, (usize::from(captured_value < 1000), 2));
        }
    }

    // movegen-speedup.md「SEEの不要な反復を省く」: 値0を枝刈りせず、
    // 最初の取り返し後の逆引きを実際に省く。
    #[test]
    fn see_prunes_stops_at_zero_and_skips_remaining_lookups() {
        let pst = weights().unwrap();
        let board = position(
            Color::Black,
            &[
                (sq(5, 4), unpromoted(Color::Black, PieceKind::Pawn)),
                (sq(5, 5), unpromoted(Color::White, PieceKind::Pawn)),
                (sq(5, 6), unpromoted(Color::White, PieceKind::Pawn)),
            ],
        );
        let counts = assert_prune_contract(
            &board,
            MoveRules::standard(),
            &pst,
            capture(sq(5, 4), sq(5, 5)),
            Some(0),
        );
        assert_eq!(counts, (1, 2));
    }

    // movegen-speedup.md「SEEの不要な反復を省く」: 相手の正の成り益を
    // 加えてから判定する。同価値の捕獲でも成り益分の損を枝刈りする。
    #[test]
    fn see_prunes_includes_recapture_promotion_before_early_exit() {
        let pst = Pst::decode(include_bytes!("../../nets/pst-init.bin")).unwrap();
        let mover = unpromoted(Color::White, PieceKind::Rook);
        let attacker = unpromoted(Color::Black, PieceKind::DragonHorse);
        let bonus =
            value(&pst, promoted(Color::Black, PieceKind::HornedFalcon)) - value(&pst, attacker);
        assert!(bonus > 0);
        for kind in [PieceKind::Rook, PieceKind::FreeKing] {
            let victim = unpromoted(Color::Black, kind);
            let board = position(
                Color::White,
                &[(sq(5, 10), mover), (sq(5, 8), victim), (sq(4, 7), attacker)],
            );
            let expected = value(&pst, victim) - value(&pst, mover) - bonus;
            let counts = assert_prune_contract(
                &board,
                MoveRules::standard(),
                &pst,
                capture(sq(5, 10), sq(5, 8)),
                Some(expected),
            );
            if kind == PieceKind::Rook {
                assert!(expected < 0);
                assert_eq!(counts, (2, 2));
            } else {
                assert!(expected >= 0);
                assert_eq!(counts, (1, 2));
            }
        }
    }

    // movegen-speedup.md「SEEの不要な反復を省く」: P6の強制成りで
    // 価値が下がる初手もg0へ反映する。取り返しの有無を両方検査する。
    #[test]
    fn see_prunes_accounts_for_value_loss_on_forced_initial_promotion() {
        use crate::eval::pst::{PIECE_STATE_COUNT, piece_state_of};
        use sha2::{Digest, Sha256};

        // evaluation.mdのMNPT形式に従い、成香の価値だけを1へ下げる。
        let mut bytes = include_bytes!("../../nets/pst-init.bin").to_vec();
        let promoted_lance = promoted(Color::Black, PieceKind::WhiteHorse);
        let offset = bytes.len() - PIECE_STATE_COUNT * 4 + piece_state_of(promoted_lance) * 4;
        bytes[offset..offset + 4].copy_from_slice(&1_i32.to_le_bytes());
        let checksum = Sha256::digest(&bytes[80..]);
        bytes[48..80].copy_from_slice(&checksum);
        let pst = Pst::decode(&bytes).unwrap();
        let mover = unpromoted(Color::Black, PieceKind::Lance);
        let victim = unpromoted(Color::White, PieceKind::Pawn);
        assert!(value(&pst, promoted_lance) < value(&pst, mover));
        let rules = MoveRules {
            p6: true,
            ..MoveRules::standard()
        };
        let mv = Move {
            promote: true,
            ..capture(sq(5, 10), sq(5, 11))
        };
        assert_eq!(
            rules.promotion_choice_for(
                Color::Black,
                PieceKind::Lance,
                false,
                mv.from,
                mv.to,
                true,
                false,
            ),
            PromotionChoice::PromotionForced
        );
        for defended in [false, true] {
            let mut pieces = vec![(mv.from, mover), (mv.to, victim)];
            if defended {
                pieces.push((sq(5, 9), unpromoted(Color::White, PieceKind::Rook)));
            }
            let board = position(Color::Black, &pieces);
            let expected = value(&pst, victim) - value(&pst, mover)
                + if defended {
                    0
                } else {
                    value(&pst, promoted_lance)
                };
            assert!(expected < 0);
            let counts = assert_prune_contract(&board, rules, &pst, mv, Some(expected));
            assert_eq!(counts, if defended { (2, 2) } else { (1, 1) });
        }
    }

    // movegen-speedup.md「SEEの不要な反復を省く」: 後続で判定不能に
    // なる列も枝刈りしない。負の中間利得による早期枝刈りは禁止する。
    #[test]
    fn see_prunes_preserves_late_rule_dependence_with_and_without_early_exit() {
        let pst = weights().unwrap();
        for victim_kind in [PieceKind::Lion, PieceKind::Pawn] {
            let board = position(
                Color::Black,
                &[
                    (sq(5, 0), unpromoted(Color::Black, PieceKind::Rook)),
                    (sq(5, 5), unpromoted(Color::White, victim_kind)),
                    (sq(5, 6), unpromoted(Color::White, PieceKind::Pawn)),
                    (sq(7, 7), unpromoted(Color::Black, PieceKind::Lion)),
                    (sq(3, 3), unpromoted(Color::White, PieceKind::Lion)),
                ],
            );
            let counts = assert_prune_contract(
                &board,
                MoveRules::standard(),
                &pst,
                capture(sq(5, 0), sq(5, 5)),
                None,
            );
            assert_eq!(
                counts,
                if victim_kind == PieceKind::Lion {
                    (1, 3)
                } else {
                    (3, 3)
                }
            );
        }
    }

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

    // movegen-speedup.md「SEEの不要な反復を省く」: 初手から判定不能なら枝刈りしない。
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
        assert_eq!(
            assert_prune_contract(&two_stage, rules, &pst, attached_capture, None),
            (0, 0)
        );

        let igui = Move {
            from: sq(5, 7),
            mid: Some(sq(5, 6)),
            to: sq(5, 7),
            promote: false,
        };
        assert_eq!(
            assert_prune_contract(&two_stage, rules, &pst, igui, None),
            (0, 0)
        );

        let kirin = position(
            Color::Black,
            &[
                (sq(4, 7), unpromoted(Color::Black, PieceKind::Kirin)),
                (sq(5, 8), unpromoted(Color::White, PieceKind::Lion)),
            ],
        );
        let mv = Move {
            from: sq(4, 7),
            mid: None,
            to: sq(5, 8),
            promote: true,
        };
        assert_eq!(assert_prune_contract(&kirin, rules, &pst, mv, None), (0, 0));
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
            assert_prune_contract(&board, rules, &pst, mv, None);
        }
    }

    // strength-stage3.md「静的交換評価」: 守りなし、損な高価駒の捕獲、x-ray、
    // および王駒による取り返しを手計算した交換列と照合する。
    #[test]
    fn see_values_match_hand_calculated_exchange_sequences() {
        let pst = Pst::decode(include_bytes!("../../nets/pst-init.bin")).unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 5);

        let black_pawn = unpromoted(Color::Black, PieceKind::Pawn);
        let white_go_between = unpromoted(Color::White, PieceKind::GoBetween);
        let unguarded = position(
            Color::Black,
            &[(sq(5, 4), black_pawn), (target, white_go_between)],
        );
        assert_eq!(
            see_reference(&unguarded, rules, &pst, capture(sq(5, 4), target)),
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
                see_reference(&lion_capture, rules, &pst, capture(sq(5, 0), target)),
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
            see_reference(&defended, rules, &pst, capture(sq(5, 0), target)),
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
        // 凍結駒価値は仲人125、金378、歩100、銀250、飛750。
        // 金で仲人を取って歩に取り返された時点で125−378=−253。
        // 銀で歩を取り返すと遮蔽が外れた飛車に銀を取られ、さらに150損するため中止する。
        assert_eq!(
            see_reference(&xray, rules, &pst, capture(sq(4, 4), target)),
            Some(-253)
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
            see_reference(&king_recapture, rules, &pst, capture(sq(4, 4), target)),
            Some(value(&pst, free_king))
        );
    }

    // RULES.md第14条第5項: 獅子が非獅子を取る段階は通常の交換とし、
    // 非獅子の取り返しと獅子による取り返しの価値を反映する。
    #[test]
    fn see_values_lion_captures_of_non_lions() {
        let pst = Pst::decode(include_bytes!("../../nets/pst-init.bin")).unwrap();
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
            see_reference(&lion_is_recaptured, rules, &pst, capture(sq(4, 4), target)),
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
        // 凍結駒価値は歩100、金378、飛750。金が歩を取った後に飛車が
        // 金を取ると獅子が飛車を取り返し、先手は100−378+750=472を得る。
        // 後手は取り返さず歩100の損で止められるため、交換評価は100となる。
        assert_eq!(
            see_reference(&lion_recaptures, rules, &pst, capture(sq(4, 4), target)),
            Some(100)
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
                see_reference(&board, rules, &pst, capture(sq(4, 4), target)),
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
            see_reference(&first_promotion, rules, &pst, promoting_capture),
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
            see_reference(&promoted_recapture, rules, &pst, capture(sq(5, 10), target)),
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
            see_reference(&board, rules, &pst, capture(sq(5, 10), target)),
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
            assert_eq!(
                see_reference(&board, rules, &pst, mv),
                Some(value(&pst, victim))
            );
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
            .filter(|&mv| see_reference(&board, rules, &pst, mv).is_none())
            .collect();
        assert!(!none_captures.is_empty());
        assert!(
            none_captures
                .iter()
                .all(|&mv| { !capture_is_pruned_by_see(&board, rules, &pst, mv) })
        );
    }
}

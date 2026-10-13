//! 到達升での駒の取り合いを見積もる静的交換評価（SEE）。
//!
//! 経由升を持つ着手と、獅子を獅子または成れる麒麟が取る段階を
//! 含む交換列は、捕獲規則に依存するため判定不能とする。非獅子が獅子を
//! 取る手、獅子が非獅子を取る手、および角鷹と飛鷲が到達升だけで取る手は
//! 通常の交換として評価する。
//! 判定不能とする範囲の理由は`docs/adr/mark-only-lion-rule-dependent-exchanges-unknown-in-see.md`にある。
//!
//! 取り返す駒の成りは[`MoveRules::promotion_choice_for`]で判定し、成り規則を複製しない。
//! 任意成りは成り前後の高い方の価値を使い、正の差額だけを利得に加える。
//! 強制成りは成り後の価値を使い、負も含む差額を加える。
//! 到達升以外での2段階移動による2枚目の捕獲益は見積もらず、王駒による取り返しにも特別な停止規則を置かない。

use crate::eval::Nnue;
use minase_core::board::Bitboard;
use minase_core::mv::Move;
use minase_core::piece::{Color, PieceCode, PieceKind};
use minase_core::position::Position;
use minase_core::promotion::PromotionChoice;
use minase_core::rules::MoveRules;

/// 交換列で保持できる利得の数。中将棋の盤上の駒は最大92枚である。
const MAX_GAINS: usize = 93;

/// 捕獲の交換評価が`-margin`未満で、規則依存による判定不能でもない場合に枝刈りする。
///
/// `margin`は駒価値と同じ単位の非負の余裕値であり、等号では枝刈りしない。
/// 呼び出し側は合法な捕獲手だけを渡し、相手の最後の王駒を取る手は除く。
/// 交換列を最後まで逆算した値が得られる場合と同じ閾値判定を返し、規則依存で判定不能なら偽を返す。
/// 最初の取り返しで損が余裕値以下と確定したら、残る逆引きと逆算を省く。
pub(super) fn see_prunes(
    position: &Position,
    rules: MoveRules,
    nnue: &Nnue,
    mv: Move,
    margin: i32,
) -> bool {
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
    gains[0] = nnue.piece_value(captured_piece) + nnue.piece_value(piece_after_move)
        - nnue.piece_value(moving_piece);
    // 相手の成り益を最大に見積もっても損が余裕値以下なら、取り返しの列挙を省く。
    if gains[0] >= -margin
        && nnue.piece_value(moving_piece) - nnue.piece_value(captured_piece)
            + nnue.max_promotion_gain()
            <= margin
    {
        return false;
    }
    let mut piece_value = nnue.piece_value(piece_after_move);
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

        let next = least_valuable_attacker(position, nnue, attackers);
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
                piece_value = nnue.piece_value(next_piece);
            }
            PromotionChoice::PromotionOptional => {
                let promoted = promoted_piece(side, next_kind);
                let unpromoted_value = nnue.piece_value(next_piece);
                let promoted_value = nnue.piece_value(promoted);
                gains[depth] += (promoted_value - unpromoted_value).max(0);
                piece_value = unpromoted_value.max(promoted_value);
            }
            PromotionChoice::PromotionForced => {
                let promoted = promoted_piece(side, next_kind);
                let unpromoted_value = nnue.piece_value(next_piece);
                let promoted_value = nnue.piece_value(promoted);
                gains[depth] += promoted_value - unpromoted_value;
                piece_value = promoted_value;
            }
        }
        // 逆算結果は min(g0, max(-g1, X))。g0 >= -marginかつg1 <= marginなら
        // -margin以上である。成り益を含むg1で判定する。
        // 後続が獅子規則に依存しても枝刈りしないので、この終了は安全である。
        if depth == 1 && gains[0] >= -margin && gains[1] <= margin {
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
    gains[0] < -margin
}

/// 捕獲手について、到達升での駒の取り合いを手番側視点で見積もる。
///
/// 経由升を持つ着手、麒麟が獅子を取って成る着手、および到達升の
/// 獅子を獅子または成れる麒麟が取る段階を含む場合は`None`を返す。
/// これらは第14条、第15条第7項、および第16条の獅子捕獲規則に依存する。
/// その他の捕獲は最も安い攻撃駒で交互に取り返す利得列を作り、末尾から取り返さない選択肢と比較して逆算する。
/// 初手と取り返しの成り益はモジュールの契約に従って利得へ加える。
#[cfg(test)]
fn see_reference(position: &Position, rules: MoveRules, nnue: &Nnue, mv: Move) -> Option<i32> {
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
    gains[0] = nnue.piece_value(captured_piece) + nnue.piece_value(piece_after_move)
        - nnue.piece_value(moving_piece);
    let mut piece_value = nnue.piece_value(piece_after_move);
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

        let next = least_valuable_attacker(position, nnue, attackers);
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
                piece_value = nnue.piece_value(next_piece);
            }
            PromotionChoice::PromotionOptional => {
                let promoted = promoted_piece(side, next_kind);
                let unpromoted_value = nnue.piece_value(next_piece);
                let promoted_value = nnue.piece_value(promoted);
                gains[depth] += (promoted_value - unpromoted_value).max(0);
                piece_value = unpromoted_value.max(promoted_value);
            }
            PromotionChoice::PromotionForced => {
                let promoted = promoted_piece(side, next_kind);
                let unpromoted_value = nnue.piece_value(next_piece);
                let promoted_value = nnue.piece_value(promoted);
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
    target: minase_core::Square,
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
fn least_valuable_attacker(
    position: &Position,
    nnue: &Nnue,
    attackers: Bitboard,
) -> minase_core::Square {
    attackers
        .into_iter()
        .min_by_key(|&square| (nnue.piece_value(piece_at(position, square)), square.raw()))
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
fn piece_at(position: &Position, square: minase_core::Square) -> PieceCode {
    position
        .piece_at(square)
        .expect("SEE square must contain a piece")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::weights;
    use minase_core::MoveGenerator;
    use minase_core::position::PositionBuilder;
    use minase_core::rules::Rules;
    use minase_core::test_util::{
        bench_positions, position_from_codes, sampled_random_positions, sq,
    };

    use core::cell::Cell;

    thread_local! {
        pub(super) static LOOKUPS: Cell<usize> = const { Cell::new(0) };
    }

    /// 余裕値0・200と負の期待値の境界で契約を検査し、
    /// 余裕値0の判定と参照実装の逆引き回数を返す。
    fn assert_prune_contract(
        board: &Position,
        rules: MoveRules,
        nnue: &Nnue,
        mv: Move,
        expected: Option<i32>,
    ) -> (usize, usize) {
        LOOKUPS.set(0);
        let reference = see_reference(board, rules, nnue, mv);
        let reference_lookups = LOOKUPS.get();
        assert_eq!(reference, expected, "move={mv:?}");
        let mut zero_margin_lookups = 0;
        for margin in [0, 200] {
            LOOKUPS.set(0);
            assert_eq!(
                see_prunes(board, rules, nnue, mv, margin),
                reference.is_some_and(|v| v < -margin),
                "move={mv:?}, margin={margin}"
            );
            if margin == 0 {
                zero_margin_lookups = LOOKUPS.get();
            }
        }
        if let Some(v) = expected.filter(|&v| v < 0) {
            assert!(
                !see_prunes(board, rules, nnue, mv, -v),
                "move={mv:?}, margin={}, expected={v}",
                -v
            );
            assert!(
                see_prunes(board, rules, nnue, mv, -v - 1),
                "move={mv:?}, margin={}, expected={v}",
                -v - 1
            );
        }
        (zero_margin_lookups, reference_lookups)
    }

    // 全捕獲の枝刈り判断を
    // benchの15局面と各規則32局面の固定シード対局で参照値と照合する。
    #[test]
    fn see_prunes_matches_reference_on_bench_and_random_captures() {
        let nnue = weights().unwrap();
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
                    for margin in [0, 200] {
                        assert_eq!(
                            see_prunes(&board, rules, &nnue, mv, margin),
                            see_reference(&board, rules, &nnue, mv).is_some_and(|v| v < -margin),
                            "rules={rules:?}, position={index}, move={mv:?}, margin={margin}"
                        );
                    }
                    checked += 1;
                }
            }
            assert!(checked > 0);
        }
    }

    // 飛車で角行を取り、歩兵に
    // 取り返される損失の直前、等号、直後で厳密な境界を検査する。
    #[test]
    fn see_prunes_respects_margin_and_strict_boundary() {
        let nnue = Nnue::decode(include_bytes!("../../../nets/nnue.bin")).unwrap();
        let board = position(
            Color::Black,
            &[
                (sq(5, 4), unpromoted(Color::Black, PieceKind::Rook)),
                (sq(5, 5), unpromoted(Color::White, PieceKind::Bishop)),
                (sq(5, 6), unpromoted(Color::White, PieceKind::Pawn)),
            ],
        );
        let mv = capture(sq(5, 4), sq(5, 5));
        let rules = MoveRules::standard();
        let loss = value(&nnue, unpromoted(Color::Black, PieceKind::Rook))
            - value(&nnue, unpromoted(Color::White, PieceKind::Bishop));
        assert!(loss > 0);
        assert_prune_contract(&board, rules, &nnue, mv, Some(-loss));
        for (margin, prunes) in [
            (0, true),
            (loss - 1, true),
            (loss, false),
            (loss + 1, false),
        ] {
            LOOKUPS.set(0);
            assert_eq!(see_prunes(&board, rules, &nnue, mv, margin), prunes);
            // 境界以上の余裕値では最初の取り返しだけで終了できる。
            assert_eq!(LOOKUPS.get(), if prunes { 2 } else { 1 });
        }
    }

    /// 捕獲価値と成り益の上限から交換の符号が確定するときだけ、攻撃駒の逆引きを省く。
    #[test]
    fn see_prunes_skips_lookup_at_the_promotion_gain_bound() {
        use crate::eval::{PIECE_STATE_COUNT, piece_state_of};
        use sha2::{Digest, Sha256};

        for captured_value in [799_i32, 800, 801, 999, 1000, 1001] {
            // MNUEの駒価値表を直接指定する。最大の成り益は歩100→金1000の900。
            let mut bytes = include_bytes!("../../../nets/nnue.bin").to_vec();
            let base = 192 + 13_680 * 4;
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
            let checksum = Sha256::digest(&bytes[192..]);
            bytes[144..176].copy_from_slice(&checksum);
            let nnue = Nnue::decode(&bytes).unwrap();
            assert_eq!(nnue.max_promotion_gain(), 900);
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
                &nnue,
                capture(sq(5, 4), sq(5, 5)),
                Some(captured_value - 100),
            );
            assert_eq!(counts, (usize::from(captured_value < 1000), 2));
            LOOKUPS.set(0);
            assert!(!see_prunes(
                &board,
                MoveRules::standard(),
                &nnue,
                capture(sq(5, 4), sq(5, 5)),
                200,
            ));
            assert_eq!(LOOKUPS.get(), usize::from(captured_value < 800));
        }
    }

    // 値0を枝刈りせず、
    // 最初の取り返し後の逆引きを実際に省く。
    #[test]
    fn see_prunes_stops_at_zero_and_skips_remaining_lookups() {
        let nnue = weights().unwrap();
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
            &nnue,
            capture(sq(5, 4), sq(5, 5)),
            Some(0),
        );
        assert_eq!(counts, (1, 2));
    }

    // 相手の正の成り益を
    // 加えてから判定する。同価値の捕獲でも成り益分の損を枝刈りする。
    #[test]
    fn see_prunes_includes_recapture_promotion_before_early_exit() {
        let nnue = Nnue::decode(include_bytes!("../../../nets/nnue.bin")).unwrap();
        let mover = unpromoted(Color::White, PieceKind::Rook);
        let attacker = unpromoted(Color::Black, PieceKind::DragonHorse);
        let bonus =
            value(&nnue, promoted(Color::Black, PieceKind::HornedFalcon)) - value(&nnue, attacker);
        assert!(bonus > 0);
        for kind in [PieceKind::Rook, PieceKind::FreeKing] {
            let victim = unpromoted(Color::Black, kind);
            let board = position(
                Color::White,
                &[(sq(5, 10), mover), (sq(5, 8), victim), (sq(4, 7), attacker)],
            );
            let expected = value(&nnue, victim) - value(&nnue, mover) - bonus;
            let counts = assert_prune_contract(
                &board,
                MoveRules::standard(),
                &nnue,
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

    // P6の強制成りで
    // 価値が下がる初手もg0へ反映する。取り返しの有無を両方検査する。
    #[test]
    fn see_prunes_accounts_for_value_loss_on_forced_initial_promotion() {
        use crate::eval::piece_state_of;
        use sha2::{Digest, Sha256};

        // 検証用の重みでは成香の価値だけを1へ下げ、強制成りの負の差額を作る。
        let mut bytes = include_bytes!("../../../nets/nnue.bin").to_vec();
        let promoted_lance = promoted(Color::Black, PieceKind::WhiteHorse);
        let offset = 192 + 13_680 * 4 + piece_state_of(promoted_lance) * 4;
        bytes[offset..offset + 4].copy_from_slice(&1_i32.to_le_bytes());
        let checksum = Sha256::digest(&bytes[192..]);
        bytes[144..176].copy_from_slice(&checksum);
        let nnue = Nnue::decode(&bytes).unwrap();
        let mover = unpromoted(Color::Black, PieceKind::Lance);
        let victim = unpromoted(Color::White, PieceKind::Pawn);
        assert!(value(&nnue, promoted_lance) < value(&nnue, mover));
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
            let expected = value(&nnue, victim) - value(&nnue, mover)
                + if defended {
                    0
                } else {
                    value(&nnue, promoted_lance)
                };
            assert!(expected < 0);
            let counts = assert_prune_contract(&board, rules, &nnue, mv, Some(expected));
            assert_eq!(counts, if defended { (2, 2) } else { (1, 1) });
        }
    }

    // 後続で判定不能に
    // なる列も枝刈りしない。負の中間利得による早期枝刈りは禁止する。
    #[test]
    fn see_prunes_preserves_late_rule_dependence_with_and_without_early_exit() {
        let nnue = weights().unwrap();
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
                &nnue,
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

    fn position(side: Color, pieces: &[(minase_core::Square, PieceCode)]) -> Position {
        position_from_codes(side, pieces)
    }

    fn capture(from: minase_core::Square, to: minase_core::Square) -> Move {
        Move {
            from,
            mid: None,
            to,
            promote: false,
        }
    }

    fn value(nnue: &Nnue, piece: PieceCode) -> i32 {
        nnue.piece_value(piece)
    }

    // 初手から判定不能なら枝刈りしない。
    // 2段階移動と、麒麟が
    // 獅子を取って成る初手は規則依存なので判定不能とする。
    #[test]
    fn see_returns_none_for_rule_dependent_initial_moves() {
        let nnue = weights().unwrap();
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
            assert_prune_contract(&two_stage, rules, &nnue, attached_capture, None),
            (0, 0)
        );

        let igui = Move {
            from: sq(5, 7),
            mid: Some(sq(5, 6)),
            to: sq(5, 7),
            promote: false,
        };
        assert_eq!(
            assert_prune_contract(&two_stage, rules, &nnue, igui, None),
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
        assert_eq!(
            assert_prune_contract(&kirin, rules, &nnue, mv, None),
            (0, 0)
        );
    }

    // 到達升の獅子を、獅子または
    // 成れる麒麟が取る段階を含む交換列は判定不能とする。
    #[test]
    fn see_returns_none_for_rule_dependent_recaptures() {
        let nnue = weights().unwrap();
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
            assert_prune_contract(&board, rules, &nnue, mv, None);
        }
    }

    // 守りなし、損な高価駒の捕獲、x-ray、
    // および王駒による取り返しを手計算した交換列と照合する。
    #[test]
    fn see_values_match_hand_calculated_exchange_sequences() {
        let nnue = Nnue::decode(include_bytes!("../../../nets/nnue.bin")).unwrap();
        let rules = MoveRules::standard();
        let target = sq(5, 5);

        let black_pawn = unpromoted(Color::Black, PieceKind::Pawn);
        let white_go_between = unpromoted(Color::White, PieceKind::GoBetween);
        let unguarded = position(
            Color::Black,
            &[(sq(5, 4), black_pawn), (target, white_go_between)],
        );
        assert_prune_contract(
            &unguarded,
            rules,
            &nnue,
            capture(sq(5, 4), target),
            Some(value(&nnue, white_go_between)),
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
            assert_prune_contract(
                &lion_capture,
                rules,
                &nnue,
                capture(sq(5, 0), target),
                Some(value(&nnue, white_lion)),
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
        assert_prune_contract(
            &defended,
            rules,
            &nnue,
            capture(sq(5, 0), target),
            Some(value(&nnue, white_pawn) - value(&nnue, black_rook)),
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
        // 金で仲人を取って歩に取り返されたところで交換を止める。
        // 銀で歩を取り返すと飛車の遮蔽が外れ、さらに銀と歩の価値差だけ損する。
        assert!(value(&nnue, black_silver) > value(&nnue, white_pawn));
        assert_prune_contract(
            &xray,
            rules,
            &nnue,
            capture(sq(4, 4), target),
            Some(value(&nnue, white_go_between) - value(&nnue, black_gold)),
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
        assert_prune_contract(
            &king_recapture,
            rules,
            &nnue,
            capture(sq(4, 4), target),
            Some(value(&nnue, free_king)),
        );
    }

    // RULES.md第14条第5項: 獅子が非獅子を取る段階は通常の交換とし、
    // 非獅子の取り返しと獅子による取り返しの価値を反映する。
    #[test]
    fn see_values_lion_captures_of_non_lions() {
        let nnue = Nnue::decode(include_bytes!("../../../nets/nnue.bin")).unwrap();
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
        assert_prune_contract(
            &lion_is_recaptured,
            rules,
            &nnue,
            capture(sq(4, 4), target),
            Some(value(&nnue, white_pawn) - value(&nnue, black_lion)),
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
        assert_prune_contract(
            &lion_recaptures,
            rules,
            &nnue,
            capture(sq(4, 4), target),
            Some(100),
        );
    }

    // RULES.md第11条: 角鷹と飛鷲は、2升目への特殊到達範囲だけで
    // 届く場合も、非獅子を1枚取る取り返しとして評価する。
    #[test]
    fn see_values_lion_like_special_recaptures() {
        let nnue = weights().unwrap();
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
        let expected = value(&nnue, victim) - value(&nnue, mover);

        for (index, board) in cases.into_iter().enumerate() {
            let mv = capture(sq(4, 4), target);
            assert_eq!(
                see_reference(&board, rules, &nnue, mv),
                Some(expected),
                "case={index}"
            );
            for margin in [0, 200, -expected, -expected - 1] {
                assert_eq!(
                    see_prunes(&board, rules, &nnue, mv, margin),
                    expected < -margin,
                    "case={index}, margin={margin}"
                );
            }
        }
    }

    // 初手の成駒が失われる価値と、取り返す
    // 駒の任意成りによる正の差額を交換列へ含める。
    #[test]
    fn see_accounts_for_promotions_in_initial_and_recapture_moves() {
        let nnue = weights().unwrap();
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
        assert_prune_contract(
            &first_promotion,
            rules,
            &nnue,
            promoting_capture,
            Some(value(&nnue, white_pawn) - value(&nnue, dragon_horse)),
        );
        assert!(value(&nnue, horned_falcon) > value(&nnue, dragon_horse));

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
        let promotion_bonus = (value(&nnue, horned_falcon) - value(&nnue, dragon_horse)).max(0);
        assert_prune_contract(
            &promoted_recapture,
            rules,
            &nnue,
            capture(sq(5, 10), target),
            Some(value(&nnue, black_pawn) - value(&nnue, white_rook) - promotion_bonus),
        );
    }

    // 第32条のP5: 成りを保留した歩兵は、最奥段以外で取り返す場合に
    // 成れないため、交換列へ成りの差額を加えない。
    #[test]
    fn see_respects_p5_deferred_pawn_on_recapture() {
        let nnue = weights().unwrap();
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

        assert_prune_contract(
            &board,
            rules,
            &nnue,
            capture(sq(5, 10), target),
            Some(value(&nnue, black_go_between) - value(&nnue, white_rook)),
        );
    }

    // RULES.md第11条: 角鷹と飛鷲がmidなしで1枚だけ取る初手は、隣接移動でも
    // 2升目への直接跳びでも通常の交換として扱う。
    #[test]
    fn see_accepts_single_capture_lion_like_moves_without_mid() {
        let nnue = weights().unwrap();
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
            assert_prune_contract(&board, rules, &nnue, mv, Some(value(&nnue, victim)));
        }
    }
}

//! 設計書movegen-speedup-2.md「段階2」「検証」に基づく通常駒の利きの同値性テスト。

use std::num::{NonZeroU64, NonZeroUsize};

use crate::core::board::Bitboard;
use crate::core::movegen::MoveGenerator;
use crate::core::movegen::control::piece_control_without_special;
use crate::core::movegen::tests::{TWO_STAGE_KINDS, capture_test_positions};
use crate::core::position::Position;
use crate::core::rules::{LionRule, MoveRules, PromotionRule, Rules};
use crate::rng::XorShift64;

/// 空集合、王駒、全相手駒、各1升、および固定シードの部分集合で利きを照合する。
fn assert_ordinary_capturer_matches_control(
    generator: &MoveGenerator,
    position: &Position,
    rng: &mut XorShift64,
) {
    let color = position.side_to_move();
    let enemy = position.pieces_of(color.opposite());
    let mut masks = vec![
        Bitboard::EMPTY,
        position.royal_pieces(color.opposite()),
        enemy,
    ];
    masks.extend(enemy.into_iter().map(|s| Bitboard::from_squares([s])));
    for _ in 0..8 {
        masks.push(Bitboard::from_squares(
            enemy.into_iter().filter(|_| rng.next() & 1 != 0),
        ));
    }

    for from in position.pieces_of(color) {
        let piece = position.piece_at(from).unwrap();
        let kind = piece.kind().unwrap();
        if TWO_STAGE_KINDS.contains(&kind) {
            continue;
        }
        let control = piece_control_without_special(
            generator.tables(),
            position.occupied(),
            color,
            kind,
            from,
        );
        for &allowed in &masks {
            let expected = control & allowed;
            let actual = generator
                .ordinary_capturer(position, from, piece, allowed)
                .map(|capturer| capturer.captures);
            assert_eq!(
                actual,
                (!expected.is_empty()).then_some(expected),
                "color={color:?}, kind={kind:?}, from={from:?}, allowed={allowed:?}"
            );
        }
    }
}

// 設計書movegen-speedup-2.md「段階2」「検証」: 通常駒の捕獲可能升は、
// 特殊移動を除いた利きと対象升の積に一致し、積が空ならNoneを返す。
#[test]
fn ordinary_capturer_matches_control_for_capture_test_positions() {
    let generator = MoveGenerator::standard();
    let mut rng = XorShift64::new(NonZeroU64::new(0x5241_5953_5441_4702).unwrap());
    for position in capture_test_positions() {
        assert_ordinary_capturer_matches_control(&generator, &position, &mut rng);
    }
}

// 設計書movegen-speedup-2.md「段階2」「検証」: properties.rsの捕獲生成一致テストと
// 同じ規則・シード・手数の局面群で、対象升を絞っても利きの同値性を保つ。
#[test]
fn ordinary_capturer_matches_control_in_seeded_playouts() {
    let rule_sets = [
        MoveRules::standard(),
        Rules::LISHOGI.moves,
        MoveRules {
            promotion: PromotionRule::P1,
            p3: true,
            p4: true,
            ..MoveRules::standard()
        },
        MoveRules {
            promotion: PromotionRule::P2,
            p5: true,
            p6: true,
            ..MoveRules::standard()
        },
        MoveRules {
            lion: LionRule::L0 { l4: true },
            l3: true,
            ..MoveRules::standard()
        },
    ];
    let mut masks_rng = XorShift64::new(NonZeroU64::new(0x5241_5953_5441_4702).unwrap());
    for (index, rules) in rule_sets.into_iter().enumerate() {
        let generator = MoveGenerator::new(rules);
        let mut rng =
            XorShift64::new(NonZeroU64::new(0x4341_5054_5552_4501 + index as u64).unwrap());
        let mut position = Position::initial();
        for _ in 0..128 {
            assert_ordinary_capturer_matches_control(&generator, &position, &mut masks_rng);
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            if moves.is_empty() {
                break;
            }
            position.make_move_unchecked(
                moves[rng.index(NonZeroUsize::new(moves.len()).unwrap())],
                rules,
            );
        }
    }
}

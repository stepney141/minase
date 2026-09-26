//! 横断的性質の試験。

use super::*;
use crate::core::rules::{ExhaustionRule, RepetitionRule};
use crate::rng::XorShift64;
use std::num::NonZeroU64;

#[test]
fn plan_referee_7_piece_exhaustion_evaluates_before_mate() {
    // D3-PRP-02: 駒枯れと詰みが同一着手で同時に成立する場合、勝者はどちら
    // でも余分な駒を持つ側で一致し、駒枯れを先に評価するため終局理由は
    // 駒枯れとなる(game-referee.md 7節)。
    let mut game = game_with_codes(
        position(
            Color::White,
            &[
                (sq(0, 0), piece(Color::Black, PieceKind::King)),
                (sq(2, 1), piece(Color::White, PieceKind::King)),
                (sq(5, 5), piece(Color::White, PieceKind::Rook)),
            ],
        ),
        &[RuleCode::R1],
    );

    // 飛車の(0,5)への移動は黒王将の詰みを作ると同時に、黒が飛車を取れない
    // 駒枯れの勝ち条件も満たす。
    assert_eq!(
        game.play(step(sq(5, 5), sq(0, 5))),
        win(Color::White, WinReason::PieceExhaustion)
    );
    // E3の裸玉と詰みが同時に成立しても、裸玉を先に裁定する。
    let (position, mv) = mate_predecessor();
    let mut bare_king = game_with_codes(position, &[RuleCode::R1, RuleCode::E3]);
    assert_eq!(bare_king.play(mv), win(Color::White, WinReason::BareKing));
}

// 終局理由が採用規則で発動し得る裁定かを検査する(D3-PRP-01)。
fn assert_result_reason_allowed(rules: Rules, result: GameResult, name: &str) {
    let allowed = match result {
        GameResult::Win { reason, .. } => match reason {
            WinReason::RoyalCapture | WinReason::Stalemate => true,
            WinReason::Repetition => rules.repetition == RepetitionRule::R1,
            WinReason::Mate => !rules.e1,
            WinReason::PieceExhaustion => rules.exhaustion == ExhaustionRule::E0,
            WinReason::BareKing => rules.exhaustion == ExhaustionRule::E3,
            WinReason::Resignation => false,
        },
        GameResult::Draw { reason } => match reason {
            DrawReason::Repetition => rules.repetition == RepetitionRule::R1,
            DrawReason::PieceExhaustion => rules.exhaustion == ExhaustionRule::E0,
            DrawReason::BareKing => rules.exhaustion == ExhaustionRule::E3,
            DrawReason::Agreement => false,
        },
    };
    assert!(allowed, "rule_set={name}: unexpected result {result:?}");
}

#[test]
fn representative_rule_sets_random_self_play_accepts_legal_moves_and_valid_results() {
    // D3-PRP-01横断: 代表規則セット群の決定的シードのランダム自己対局で、
    // 対局合法手はすべて受理され、終局理由は発動し得る裁定に限られる。
    const PLY_CAP: u32 = 1_500;
    const RULE_SETS: [(&str, &[RuleCode], u64); 10] = [
        (
            "engine-default",
            &[RuleCode::L0, RuleCode::P0, RuleCode::R1, RuleCode::E0],
            0x4741_4d45_5f53_4f41,
        ),
        (
            "L1+L2+P3+R1+E1",
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P0,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E0,
            ],
            0x5255_4c45_4741_4d01,
        ),
        (
            "L3+P4+R1",
            &[
                RuleCode::L0,
                RuleCode::L3,
                RuleCode::P0,
                RuleCode::P4,
                RuleCode::R1,
                RuleCode::E0,
            ],
            0x5255_4c45_4741_4d02,
        ),
        (
            "P1+R1",
            &[RuleCode::L0, RuleCode::P1, RuleCode::R1, RuleCode::E0],
            0x5255_4c45_4741_4d03,
        ),
        (
            "P2+R1",
            &[RuleCode::L0, RuleCode::P2, RuleCode::R1, RuleCode::E0],
            0x5255_4c45_4741_4d04,
        ),
        (
            "L1+L2+L3+P1+P3+P4+R2+E1+E2",
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::L3,
                RuleCode::P1,
                RuleCode::P3,
                RuleCode::P4,
                RuleCode::R2,
                RuleCode::E1,
                RuleCode::E2,
            ],
            0x5255_4c45_4741_4d05,
        ),
        (
            "L1+L2+L3+P1+P3+P4+R3+E1+E2",
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::L3,
                RuleCode::P1,
                RuleCode::P3,
                RuleCode::P4,
                RuleCode::R3,
                RuleCode::E1,
                RuleCode::E2,
            ],
            0x5255_4c45_4741_4d06,
        ),
        (
            "L1+L2+P3+R1+E1+E3",
            &[
                RuleCode::L1,
                RuleCode::L2,
                RuleCode::P0,
                RuleCode::P3,
                RuleCode::R1,
                RuleCode::E1,
                RuleCode::E3,
            ],
            0x5255_4c45_4741_4d07,
        ),
        (
            "L1+L3+P5+P6+R2+E1+E2",
            &[
                RuleCode::L1,
                RuleCode::L3,
                RuleCode::P0,
                RuleCode::P5,
                RuleCode::P6,
                RuleCode::R2,
                RuleCode::E1,
                RuleCode::E2,
            ],
            0x5255_4c45_4741_4d08,
        ),
        (
            "L4+P2+P5+P6+R2+E1+E2",
            &[
                RuleCode::L0,
                RuleCode::L4,
                RuleCode::P2,
                RuleCode::P5,
                RuleCode::P6,
                RuleCode::R2,
                RuleCode::E1,
                RuleCode::E2,
            ],
            0x5255_4c45_4741_4d09,
        ),
    ];

    for (rule_set_name, codes, seed) in RULE_SETS {
        let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
        let mut game = Game::new(Rules::from_codes(codes).unwrap());

        for _ in 0..PLY_CAP {
            // 継続中の対局には対局合法手が必ず残る(第23条により空なら終局済み)。
            let moves = game.legal_moves();
            assert!(!moves.is_empty(), "rule_set={rule_set_name}");
            let selected = moves[(rng.next() as usize) % moves.len()];
            let status = game.play(selected).unwrap_or_else(|error| {
                panic!("rule_set={rule_set_name}: unexpected rejection: {error}")
            });
            if let GameStatus::Finished(result) = status {
                assert_result_reason_allowed(game.rules(), result, rule_set_name);
                break;
            }
        }
    }
}

#[test]
fn deterministic_random_r2_self_play_never_forbids_captures_or_promotions() {
    // D3-031-08性質: R2のランダム自己対局で拒否される着手は決して捕獲・
    // 成りを含まない(補題)。D3-PRP-01: R2では反復を理由とする終局がない。
    const PLY_CAP: u32 = 1_500;
    // 拒否を実際に観測できる代表局を規則ごとに1局使う。
    for (seed, codes) in [
        (
            12,
            &[RuleCode::L0, RuleCode::P0, RuleCode::R2, RuleCode::E0][..],
        ),
        (
            19,
            &[
                RuleCode::L0,
                RuleCode::P0,
                RuleCode::R2,
                RuleCode::E1,
                RuleCode::E2,
            ][..],
        ),
    ] {
        let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
        let mut rejections = 0;
        let mut game = Game::new(Rules::from_codes(codes).unwrap());
        let generator = MoveGenerator::new(game.rules().moves);

        for _ in 0..PLY_CAP {
            // 局面合法手から選び、R2の受理前拒否(第27条4項)を観測する。
            let mut moves = Vec::new();
            generator.generate_moves(game.position(), &mut moves);
            assert!(!moves.is_empty(), "codes={codes:?}");
            let start = (rng.next() as usize) % moves.len();
            let mut played = None;
            for offset in 0..moves.len() {
                let selected = moves[(start + offset) % moves.len()];
                match game.play(selected) {
                    Ok(status) => {
                        played = Some(status);
                        break;
                    }
                    Err(GameError::IllegalMove {
                        mv,
                        cause: IllegalMoveCause::Repetition,
                    }) => {
                        rejections += 1;
                        assert_eq!(mv, selected);
                        assert!(!mv.promote, "R2 rejected a promotion: {mv:?}");
                        assert!(
                            game.position()
                                .captured_squares(mv)
                                .into_iter()
                                .all(|capture| capture.is_none()),
                            "R2 rejected a capture: {mv:?}"
                        );
                    }
                    Err(error) => panic!("unexpected self-play error: {error}"),
                }
            }
            let status =
                played.expect("Article 23 must end the game before all moves are forbidden");
            if let GameStatus::Finished(result) = status {
                assert_result_reason_allowed(game.rules(), result, "R2");
                break;
            }
        }
        assert!(
            rejections > 0,
            "seed={seed}, codes={codes:?}: fixture must exercise R2 rejection"
        );
    }
}

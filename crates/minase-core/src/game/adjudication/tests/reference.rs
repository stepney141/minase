//! 全件生成を複製した参照実装と本番の裁定を、局面標本と乱数対局で照合する。

use super::super::bare_king::can_capture_any_royal;
use super::super::exhaustion::piece_exhaustion_outcome;
use super::super::mate::can_capture_last_royal;
use super::oracle::*;
use super::*;
use crate::game::repetition::{repetition_is_forbidden, retain_repetition_allowed_moves};
use crate::game::result::DrawReason;
use crate::position::PositionBuilder;
use crate::rng::{XorShift64, derive_seed};
use crate::rules::RuleCode;
use crate::test_util::{bench_positions, capture_test_rules, sampled_random_positions};

pub(super) fn production_is_mate(
    position: &mut Position,
    context: &AdjudicationContext<'_>,
) -> bool {
    // 参照版の詰み判定はE1による省略を含まないため、E1を外した文脈で照合する。
    let context = AdjudicationContext {
        rules: Rules {
            e1: false,
            ..context.rules
        },
        ..*context
    };
    mate::stalemate_or_mate(position, &context) == Some(WinReason::Mate)
}

pub(super) fn production_has_no_legal_move(
    position: &mut Position,
    context: &AdjudicationContext<'_>,
) -> bool {
    mate::stalemate_or_mate(position, context) == Some(WinReason::Stalemate)
}

fn compare_mate_and_legal_moves(position: &mut Position, context: &AdjudicationContext<'_>) {
    let before = position.clone();
    let actual = production_is_mate(position, context);
    assert_eq!(
        *position, before,
        "production mate must restore the position"
    );
    assert_eq!(actual, oracle_is_mate(position, context));
    assert_eq!(
        *position, before,
        "reference mate must restore the position"
    );
    let actual = production_has_no_legal_move(position, context);
    assert_eq!(*position, before);
    assert_eq!(
        actual,
        oracle_has_no_legal_move(position, context.generator, context.state.repetition())
    );
    assert_eq!(*position, before);
}

/// RULES.md第8条・第20条・第22条・第31条R1について、両手番と着手後を照合する。
#[test]
fn articles_8_22_31_function_results_match_full_generation() {
    let mut positions = 0;
    let mut moves_compared = 0;
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for sample in bench_positions()
            .into_iter()
            .chain(sampled_random_positions(rules))
        {
            for mover in Color::ALL {
                let position = sample.clone_with_side_to_move(mover);
                assert_eq!(
                    can_capture_last_royal(&position, &generator),
                    oracle_can_capture_last_royal(&position, &generator),
                    "rules={rules:?}, mover={mover:?}"
                );
                for attacker in Color::ALL {
                    assert_eq!(
                        can_capture_any_royal(&position, attacker),
                        oracle_can_capture_any_royal(&position, &generator, attacker),
                        "rules={rules:?}, attacker={attacker:?}"
                    );
                }
                for grace in [false, true] {
                    assert_eq!(
                        piece_exhaustion_outcome(&position, &generator, grace),
                        oracle_piece_exhaustion_outcome(&position, &generator, grace),
                        "rules={rules:?}, grace={grace}"
                    );
                }
                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);
                for mv in moves {
                    let mut after = position.clone();
                    after.make_move_unchecked(mv, rules);
                    assert_eq!(
                        move_was_attacking(&after, &generator, mover, mv),
                        oracle_move_was_attacking(&after, &generator, mover, mv),
                        "rules={rules:?}, mv={mv:?}"
                    );
                    assert_eq!(
                        can_capture_last_royal(&after, &generator),
                        oracle_can_capture_last_royal(&after, &generator),
                        "rules={rules:?}, mv={mv:?}"
                    );
                    moves_compared += 1;
                }
                positions += 1;
            }
        }
    }
    eprintln!("adjudication function reference: positions={positions}, moves={moves_compared}");
    assert!(positions > 0 && moves_compared > 0);
}

// 第33条に従って全群を明示し、R1〜R3、E0・E2・E3、E1の有無、
// L0〜L4、P0〜P6を含める。R2・R3にもE1なしの組合せを置く。
const RULE_SETS: [&[RuleCode]; 10] = {
    use RuleCode::*;
    [
        &[L0, P0, R1, E0],
        &[L1, L2, P0, P3, R1, E1, E0],
        &[L0, L3, P0, P4, R1, E2],
        &[L0, P1, R2, E0],
        &[L0, P2, R3, E0],
        &[L1, L2, L3, P1, P3, P4, R2, E1, E2],
        &[L1, L2, L3, P1, P3, P4, R3, E2],
        &[L1, L2, P0, P3, R1, E1, E3],
        &[L1, L3, P0, P5, P6, R2, E1, E2],
        &[L0, L4, P2, P5, P6, R3, E3],
    ]
};

/// 双方の王将と任意の太子、各側0〜6枚の駒を置く。
/// 4分の1は片側を最奥の隅の2×2升へ集め、死に駒と合法手なしも標本に含める。
/// 重複配置や不変条件違反の局面は捨て、上限内に構築できなければ失敗する。
fn sparse_random_position(rng: &mut XorShift64, generator: &MoveGenerator) -> Position {
    const PIECES: [(PieceKind, bool); 12] = [
        (PieceKind::Lion, false),
        (PieceKind::Kirin, false),
        (PieceKind::HornedFalcon, true),
        (PieceKind::SoaringEagle, true),
        (PieceKind::Rook, false),
        (PieceKind::Bishop, false),
        (PieceKind::Pawn, false),
        (PieceKind::Lance, false),
        (PieceKind::GoBetween, false),
        (PieceKind::GoldGeneral, false),
        (PieceKind::Lion, true),
        (PieceKind::DrunkElephant, false),
    ];
    'attempt: for _ in 0..1_000 {
        let side = Color::ALL[rng.next() as usize % 2];
        let cluster = (rng.next().is_multiple_of(4)).then(|| Color::ALL[rng.next() as usize % 2]);
        let mut builder = PositionBuilder::new(side);
        for color in Color::ALL {
            let last_rank = if color == Color::Black { 11 } else { 0 };
            let king = if cluster == Some(color) {
                sq(0, last_rank)
            } else {
                random_square(rng)
            };
            if builder.put(king, piece(color, PieceKind::King)).is_err() {
                continue 'attempt;
            }
            if rng.next().is_multiple_of(4)
                && builder
                    .put(
                        random_square(rng),
                        PieceCode::new_promoted(color, PieceKind::CrownPrince).unwrap(),
                    )
                    .is_err()
            {
                continue 'attempt;
            }
            let count = rng.next() % if cluster == Some(color) { 4 } else { 7 };
            for _ in 0..count {
                let (square, code) = if cluster == Some(color) {
                    let rank = if color == Color::Black {
                        11 - (rng.next() % 2) as u8
                    } else {
                        (rng.next() % 2) as u8
                    };
                    let kind = if rng.next().is_multiple_of(2) {
                        PieceKind::Pawn
                    } else {
                        PieceKind::Lance
                    };
                    (sq((rng.next() % 2) as u8, rank), piece(color, kind))
                } else {
                    let (kind, promoted) = PIECES[rng.next() as usize % PIECES.len()];
                    let code = if promoted {
                        PieceCode::new_promoted(color, kind).unwrap()
                    } else {
                        piece(color, kind)
                    };
                    let square = if matches!(kind, PieceKind::Pawn | PieceKind::Lance)
                        && rng.next().is_multiple_of(3)
                    {
                        sq((rng.next() % 12) as u8, last_rank)
                    } else {
                        random_square(rng)
                    };
                    (square, code)
                };
                if builder.put(square, code).is_err() {
                    continue 'attempt;
                }
            }
        }
        if let Ok(position) = builder.finish() {
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            if !moves.is_empty() {
                return position;
            }
        }
    }
    panic!("could not construct a sparse starting position with a legal move");
}

fn random_square(rng: &mut XorShift64) -> Square {
    sq((rng.next() % 12) as u8, (rng.next() % 12) as u8)
}

#[derive(Default, Debug)]
struct Outcomes {
    royal_capture: usize,
    mate: usize,
    stalemate: usize,
    exhaustion_win: usize,
    exhaustion_draw: usize,
    bare_king_win: usize,
    bare_king_draw: usize,
    repetition_win: usize,
    repetition_draw: usize,
    capped: usize,
    plies: usize,
}

impl Outcomes {
    fn record(&mut self, result: GameResult) {
        let count = match result {
            GameResult::Win { reason, .. } => match reason {
                WinReason::RoyalCapture => &mut self.royal_capture,
                WinReason::Mate => &mut self.mate,
                WinReason::Stalemate => &mut self.stalemate,
                WinReason::PieceExhaustion => &mut self.exhaustion_win,
                WinReason::BareKing => &mut self.bare_king_win,
                WinReason::Repetition => &mut self.repetition_win,
                WinReason::Resignation => panic!("random play cannot resign"),
            },
            GameResult::Draw { reason } => match reason {
                DrawReason::PieceExhaustion => &mut self.exhaustion_draw,
                DrawReason::BareKing => &mut self.bare_king_draw,
                DrawReason::Repetition => &mut self.repetition_draw,
                DrawReason::Agreement => panic!("random play cannot agree a draw"),
            },
        };
        *count += 1;
    }
}

fn compare_random_game(
    mut position: Position,
    rules: Rules,
    seed: u64,
    ply_cap: usize,
    outcomes: &mut Outcomes,
) {
    let generator = MoveGenerator::new(rules.moves);
    let mut rng = XorShift64::new(std::num::NonZeroU64::new(seed).unwrap());
    let mut state = AdjudicationState::new(rules.repetition, &position);
    let mut reference_state = state.clone();
    for ply in 0..ply_cap {
        if !rules.e1 {
            compare_mate_and_legal_moves(
                &mut position,
                &AdjudicationContext::new(rules, &state, &generator),
            );
        }
        let mut legal = Vec::new();
        generator.generate_moves(&position, &mut legal);
        retain_repetition_allowed_moves(
            &mut position,
            &generator,
            reference_state.repetition(),
            &mut legal,
        );
        // 終局時に下で空列を確認してから戻るので、継続中には必ず合法手がある。
        assert!(!legal.is_empty(), "seed={seed}, ply={ply}, rules={rules:?}");
        let mv = legal[rng.next() as usize % legal.len()];
        let mover = position.side_to_move();
        let undo = position
            .try_make_move_with_undo(mv, &generator)
            .unwrap_or_else(|error| panic!("seed={seed}, ply={ply}, rules={rules:?}: {error}"));
        assert!(!repetition_is_forbidden(state.repetition(), &position));
        let repetition = state.record_move(&position, &generator, mover, mv, &undo);
        let reference_repetition = oracle_record_move(
            &mut reference_state,
            &position,
            &generator,
            mover,
            mv,
            &undo,
        );
        assert_eq!(
            repetition, reference_repetition,
            "seed={seed}, ply={ply}, mv={mv:?}"
        );
        assert_eq!(state, reference_state, "seed={seed}, ply={ply}, mv={mv:?}");
        let before = position.clone();
        let actual = adjudicate_after_move(
            &mut position,
            AdjudicationContext::new(rules, &state, &generator),
            mover,
            repetition,
        );
        assert_eq!(
            position, before,
            "production adjudication must restore the position"
        );
        let expected = oracle_adjudicate_after_move(
            &mut position,
            &reference_state,
            rules,
            &generator,
            mover,
            reference_repetition,
        );
        assert_eq!(
            position, before,
            "reference adjudication must restore the position"
        );
        assert_eq!(
            actual, expected,
            "seed={seed}, ply={ply}, rules={rules:?}, mv={mv:?}"
        );
        state.set_piece_exhaustion_grace(actual.piece_exhaustion_grace());
        reference_state.set_piece_exhaustion_grace(expected.piece_exhaustion_grace());
        assert_eq!(state, reference_state);
        outcomes.plies += 1;
        if let Some(result) = actual.result() {
            if matches!(
                result,
                GameResult::Win {
                    reason: WinReason::Stalemate,
                    ..
                }
            ) {
                assert!(oracle_has_no_legal_move(
                    &mut position,
                    &generator,
                    reference_state.repetition()
                ));
                assert_eq!(position, before);
            }
            outcomes.record(result);
            return;
        }
    }
    outcomes.capped += 1;
}

/// RULES.md第21条から第23条、第31条・第32条の裁定順序と履歴更新を照合する。
#[test]
fn articles_21_23_31_32_random_games_match_full_generation() {
    let mut outcomes = Outcomes::default();
    for (index, codes) in RULE_SETS.into_iter().enumerate() {
        let rules = Rules::from_codes(codes).unwrap();
        let generator = MoveGenerator::new(rules.moves);
        let base = 0x5245_4645_5245_4500 + index as u64 * 1_000;
        compare_random_game(
            Position::initial(),
            rules,
            derive_seed(base, 0).get(),
            300,
            &mut outcomes,
        );
        for game in 1..=32 {
            let seed = derive_seed(base, game);
            let mut rng = XorShift64::new(seed);
            let start = sparse_random_position(&mut rng, &generator);
            compare_random_game(start, rules, rng.next(), 200, &mut outcomes);
        }
    }
    eprintln!("adjudication random games: {outcomes:?}");
    assert!(outcomes.royal_capture > 0);
    assert!(outcomes.mate > 0);
    assert!(outcomes.stalemate > 0);
    assert!(outcomes.exhaustion_win + outcomes.exhaustion_draw > 0);
    assert!(outcomes.bare_king_win + outcomes.bare_king_draw > 0);
}

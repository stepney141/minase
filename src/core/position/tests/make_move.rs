//! 着手の適用と巻き戻し、およびその記録の試験。

use super::{position_after_non_lion_captures_lion, rebuild_board_with_side};
use crate::MoveGenerator;
use crate::core::board::Square;
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::{MoveRules, PromotionRule};
use crate::rng::XorShift64;
use crate::test_util::{position as position_with_pieces, position_from_codes, sq};
use core::num::NonZeroU64;

// 先手から着手し、1手ごとに手番が相手へ移る(第6条1項、D4-006-01)。
// 獅子の2段階移動もじっとも全体で1手であり、手番移動は1回だけである
// (第3条7項・13項、第6条4項、第12条11項)。
#[test]
fn article_6_1_black_moves_first_and_the_turn_passes_once_per_move() {
    let generator = MoveGenerator::standard();
    let mut game_position = Position::initial();
    assert_eq!(game_position.side_to_move(), Color::Black);

    let mut moves = Vec::new();
    generator.generate_moves(&game_position, &mut moves);
    game_position.try_make_move(moves[0], &generator).unwrap();
    assert_eq!(game_position.side_to_move(), Color::White);

    moves.clear();
    generator.generate_moves(&game_position, &mut moves);
    game_position.try_make_move(moves[0], &generator).unwrap();
    assert_eq!(game_position.side_to_move(), Color::Black);

    // 獅子の2段階移動(第1段階で捕獲する経由升つきの手)は全体で1手である。
    let lion_home = sq(5, 5);
    let capture_position = position_with_pieces(
        Color::Black,
        &[
            (lion_home, Color::Black, PieceKind::Lion),
            (sq(5, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let mut capture_moves = Vec::new();
    generator.generate_moves(&capture_position, &mut capture_moves);
    let two_stage = capture_moves
        .iter()
        .copied()
        .find(|mv| mv.mid == Some(sq(5, 6)))
        .expect("経由升で捕獲する2段階移動が生成される");
    let mut after_two_stage = capture_position.clone();
    after_two_stage
        .try_make_move(two_stage, &generator)
        .unwrap();
    assert_eq!(after_two_stage.side_to_move(), Color::White);

    // じっとも合法な1手であり、適用後に手番が相手へ移る。
    let lone_lion =
        position_with_pieces(Color::Black, &[(lion_home, Color::Black, PieceKind::Lion)]);
    let mut lone_moves = Vec::new();
    generator.generate_moves(&lone_lion, &mut lone_moves);
    let jitto = lone_moves
        .iter()
        .copied()
        .find(|mv| mv.to == lion_home)
        .expect("じっとが生成される(第12条9項)");
    let mut after_jitto = lone_lion.clone();
    after_jitto.try_make_move(jitto, &generator).unwrap();
    assert_eq!(after_jitto.side_to_move(), Color::White);
}

// 取った駒は盤上から除かれ再使用されず、持ち駒に相当する観測状態は存在しない
// (第4条4項・5項、D4-004-04)。2段階移動の2枚捕獲では総駒数が2減る(第12条4項)。
#[test]
fn article_4_4_captured_pieces_are_removed_and_never_reused() {
    let generator = MoveGenerator::standard();

    // 単独捕獲: 総駒数がちょうど1減る。
    let mut single = position_with_pieces(
        Color::Black,
        &[
            (sq(4, 4), Color::Black, PieceKind::Rook),
            (sq(4, 9), Color::White, PieceKind::Pawn),
        ],
    );
    assert_eq!(single.occupied().popcount(), 2);
    single
        .try_make_move(
            Move {
                from: sq(4, 4),
                mid: None,
                to: sq(4, 9),
                promote: false,
            },
            &generator,
        )
        .unwrap();
    assert_eq!(single.occupied().popcount(), 1);
    assert!(single.pieces_of(Color::White).is_empty());
    assert_eq!(
        single.piece_at(sq(4, 9)).and_then(PieceCode::kind),
        Some(PieceKind::Rook)
    );

    // 獅子の2枚捕獲: 総駒数がちょうど2減る。
    let mut double = position_with_pieces(
        Color::Black,
        &[
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(5, 6), Color::White, PieceKind::Pawn),
            (sq(5, 7), Color::White, PieceKind::Pawn),
        ],
    );
    assert_eq!(double.occupied().popcount(), 3);
    double
        .try_make_move(
            Move {
                from: sq(5, 5),
                mid: Some(sq(5, 6)),
                to: sq(5, 7),
                promote: false,
            },
            &generator,
        )
        .unwrap();
    assert_eq!(double.occupied().popcount(), 1);
    assert!(double.pieces_of(Color::White).is_empty());

    // 取られた駒は配置・キーのどの観測にも痕跡を残さない: 捕獲後の局面は
    // 同じ配置を直接構築した局面と完全一致する(持ち駒集合は存在しない)。
    let rebuilt = position_with_pieces(Color::White, &[(sq(5, 7), Color::Black, PieceKind::Lion)]);
    assert_eq!(double, rebuilt);
}

/// 増分更新とundoの検査に使う代表シナリオ(局面・規則・着手列)を返す。
/// 2枚捕獲・居喰い・じっと・成り・王駒捕獲・成駒の捕獲・先獅子・P1保留を覆う
/// (D4-IMP-03・D4-IMP-04の境界事例)。
fn make_unmake_scenarios() -> Vec<(Position, MoveRules, Vec<Move>)> {
    let mv = |from, mid, to, promote| Move {
        from,
        mid,
        to,
        promote,
    };
    let standard = MoveRules::standard();
    let p1 = MoveRules {
        promotion: PromotionRule::P1,
        ..MoveRules::standard()
    };
    vec![
        // 獅子の2段階移動による2枚捕獲(第12条4項)。
        (
            position_with_pieces(
                Color::Black,
                &[
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(5, 6), Color::White, PieceKind::Pawn),
                    (sq(5, 7), Color::White, PieceKind::Pawn),
                ],
            ),
            standard,
            vec![mv(sq(5, 5), Some(sq(5, 6)), sq(5, 7), false)],
        ),
        // 居喰い(第3条14項)。
        (
            position_with_pieces(
                Color::Black,
                &[
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(5, 6), Color::White, PieceKind::Pawn),
                ],
            ),
            standard,
            vec![mv(sq(5, 5), Some(sq(5, 6)), sq(5, 5), false)],
        ),
        // じっと(第3条13項)。
        (
            position_with_pieces(Color::Black, &[(sq(5, 5), Color::Black, PieceKind::Lion)]),
            standard,
            vec![mv(sq(5, 5), None, sq(5, 5), false)],
        ),
        // 成りを伴う手(第18条1項)。
        (
            position_with_pieces(Color::Black, &[(sq(4, 7), Color::Black, PieceKind::Pawn)]),
            standard,
            vec![mv(sq(4, 7), None, sq(4, 8), true)],
        ),
        // 成駒を取る手(undoで取られた駒の成否も復元される)。
        (
            position_from_codes(
                Color::Black,
                &[
                    (
                        sq(4, 4),
                        PieceCode::new(Color::Black, PieceKind::Rook).unwrap(),
                    ),
                    (
                        sq(4, 9),
                        PieceCode::new_promoted(Color::White, PieceKind::FreeBoar).unwrap(),
                    ),
                ],
            ),
            standard,
            vec![mv(sq(4, 4), None, sq(4, 9), false)],
        ),
        // 王駒を取る手(第21条1項)。
        (
            position_with_pieces(
                Color::Black,
                &[
                    (sq(4, 4), Color::Black, PieceKind::Rook),
                    (sq(4, 9), Color::White, PieceKind::King),
                ],
            ),
            standard,
            vec![mv(sq(4, 4), None, sq(4, 9), false)],
        ),
        // 非獅子による獅子捕獲(先獅子トリガー、第15条)と、その消滅(第15条5項)。
        (
            position_with_pieces(
                Color::Black,
                &[
                    (sq(0, 0), Color::Black, PieceKind::Bishop),
                    (sq(1, 1), Color::White, PieceKind::Lion),
                    (sq(10, 10), Color::White, PieceKind::Pawn),
                ],
            ),
            standard,
            vec![
                mv(sq(0, 0), None, sq(1, 1), false),
                mv(sq(10, 10), None, sq(10, 9), false),
            ],
        ),
        // 麒麟が獅子を取って成る手(第15条7項)。
        (
            position_with_pieces(
                Color::Black,
                &[
                    (sq(4, 7), Color::Black, PieceKind::Kirin),
                    (sq(5, 8), Color::White, PieceKind::Lion),
                ],
            ),
            standard,
            vec![mv(sq(4, 7), None, sq(5, 8), true)],
        ),
        // 成り権保留の設定と消滅(第30条P1、第24条1項d)。
        (
            position_with_pieces(
                Color::Black,
                &[
                    (sq(4, 7), Color::Black, PieceKind::SilverGeneral),
                    (sq(10, 10), Color::White, PieceKind::GoldGeneral),
                ],
            ),
            p1,
            vec![
                mv(sq(4, 7), None, sq(4, 8), false),
                mv(sq(10, 10), None, sq(10, 9), false),
                mv(sq(4, 8), None, sq(4, 9), false),
                mv(sq(10, 9), None, sq(10, 10), false),
                mv(sq(4, 9), None, sq(4, 10), false),
                mv(sq(10, 10), None, sq(10, 9), false),
            ],
        ),
    ]
}

// 実装契約(D4-IMP-03・04): 前進状態は全再計算と一致し、着手を取り消すと全観測可能状態
// (盤面・手番・キー・先獅子状態・成り権保留)が完全一致で復元される。
#[test]
fn unmake_restores_every_observable_component() {
    for (mut position, rules, moves) in make_unmake_scenarios() {
        let generator = MoveGenerator::new(rules);
        let mut trail = Vec::new();
        for mv in moves {
            let snapshot = position.clone();
            let undo = position.try_make_move_with_undo(mv, &generator).unwrap();
            assert_eq!(position.zobrist(), position.recompute_zobrist(), "{mv:?}");
            assert_eq!(
                position.rights_zobrist(),
                position.recompute_rights_zobrist(),
                "{mv:?}"
            );
            assert_eq!(position.validate(), Ok(()), "{mv:?}");
            trail.push((snapshot, undo));
        }
        while let Some((snapshot, undo)) = trail.pop() {
            position.unmake_move(undo);
            assert_eq!(position, snapshot);
        }
    }
}

// D4-IMP-10[実装契約] 手番パスは先獅子の一時状態を消滅させ、
// 禁止なしで手番だけを反転した同一配置のキーへ移る。
#[test]
fn d4_imp_10_null_move_clears_lion_trigger_and_round_trips() {
    let (mut position, _) = position_after_non_lion_captures_lion();
    let before = position.clone();
    assert!(position.lion_taken_by_non_lion().is_some());
    let expected = rebuild_board_with_side(&position, position.side_to_move().opposite());

    let undo = position.make_null_move();

    assert_eq!(position.side_to_move(), before.side_to_move().opposite());
    for square in Square::all() {
        assert_eq!(position.piece_at(square), before.piece_at(square));
    }
    assert_ne!(position.zobrist(), before.zobrist());
    assert_eq!(position.lion_taken_by_non_lion(), None);
    assert_eq!(position.zobrist(), expected.recompute_zobrist());
    assert_eq!(position.zobrist(), position.recompute_zobrist());

    position.unmake_null_move(undo);
    assert_eq!(position, before);
}

// D4-IMP-10[実装契約] 成り権保留と権利キーは手番パスで変化せず、
// 巻き戻すと局面全体が復元される。
#[test]
fn d4_imp_10_null_move_preserves_promotion_rights() {
    let deferred_square = sq(4, 9);
    let mut builder = PositionBuilder::new(Color::Black);
    builder
        .put(
            deferred_square,
            PieceCode::new(Color::Black, PieceKind::SilverGeneral).unwrap(),
        )
        .unwrap();
    builder.mark_promotion_deferred(deferred_square).unwrap();
    let mut position = builder.finish().unwrap();
    let before = position.clone();

    let undo = position.make_null_move();

    assert_eq!(position.promotion_deferred(), before.promotion_deferred());
    assert_eq!(position.rights_zobrist(), before.rights_zobrist());
    assert_eq!(position.zobrist(), position.recompute_zobrist());

    position.unmake_null_move(undo);
    assert_eq!(position, before);
}

// 実装契約(D4-IMP-09): 固定シードの一様ランダムプレイアウトで、毎手、
// 増分キー＝全再計算、総駒数=92−累計捕獲枚数、手番の交替則(第6条1項)を検証する。
// 終了後は全undoで初期局面へ完全復帰する。
#[test]
fn seeded_random_playouts_uphold_conservation_and_undo_invariants() {
    let generator = MoveGenerator::standard();
    let seeds = [0x5a4f_4252_4953_5401_u64, 0x6d69_6e61_7365_4434];
    let games_per_seed = 4;
    let max_plies = 72;
    let mut captures_seen = 0_u32;
    let mut promotions_seen = 0_u32;
    let mut double_moves_seen = 0_u32;

    for seed in seeds {
        let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
        for game in 0..games_per_seed {
            let initial = Position::initial();
            let mut position = initial.clone();
            let mut history = Vec::new();
            let mut captured_total = 0_u32;

            for ply in 0..max_plies {
                // n手適用後の手番は、nが偶数なら先手、奇数なら後手である(第6条1項)。
                let expected_side = if ply % 2 == 0 {
                    Color::Black
                } else {
                    Color::White
                };
                assert_eq!(position.side_to_move(), expected_side);

                let mut moves = Vec::new();
                generator.generate_moves(&position, &mut moves);
                if moves.is_empty() {
                    break;
                }
                let mv = moves[rng.next() as usize % moves.len()];
                captured_total += position.captured_squares(mv).iter().flatten().count() as u32;
                if mv.mid.is_some() {
                    double_moves_seen += 1;
                }
                if mv.promote {
                    promotions_seen += 1;
                }
                history.push(position.make_move_unchecked(mv, MoveRules::standard()));

                let context = format!("seed={seed:#x} game={game} ply={ply}");
                assert_eq!(
                    position.zobrist(),
                    position.recompute_zobrist(),
                    "{context}"
                );
                assert_eq!(
                    position.rights_zobrist(),
                    position.recompute_rights_zobrist(),
                    "{context}"
                );
                assert_eq!(position.validate(), Ok(()), "{context}");
                // 盤上総駒数＝92−累計捕獲枚数(第4条4項)。
                assert_eq!(
                    position.occupied().popcount(),
                    92 - captured_total,
                    "{context}"
                );
            }
            captures_seen += captured_total;

            // 全着手を逆順にundoすると初期局面(第5条・先手番・キー)へ完全復帰する。
            while let Some(undo) = history.pop() {
                position.unmake_move(undo);
            }
            assert_eq!(position, initial, "seed={seed:#x} game={game}");
        }
    }

    // 検査力の担保: 捕獲・成り・2段階移動がプレイアウト中に実際に出現した。
    assert!(captures_seen > 0, "捕獲が出現しないシードは検査力が弱い");
    assert!(promotions_seen > 0, "成りが出現しないシードは検査力が弱い");
    assert!(
        double_moves_seen > 0,
        "2段階移動が出現しないシードは検査力が弱い"
    );
}

/// 設計書movegen-speedup-2.md「段階5」「検証」で指定された旧実装の参照。
fn reference_captured_squares(position: &Position, mv: Move) -> [Option<Square>; 2] {
    let moving_color = position
        .piece_at(mv.from)
        .and_then(PieceCode::color)
        .unwrap();
    mv.capture_candidates().map(|candidate| {
        candidate.filter(|&square| {
            position
                .piece_at(square)
                .is_some_and(|piece| piece.color() == Some(moving_color.opposite()))
        })
    })
}

// 同節: 全規則セットの固定シード局面と特殊移動の境界局面の全合法手で、
// 捕獲升の順序、成り、居喰い、2枚取り、および適用・復元の同値性を守る。
#[test]
fn captured_squares_match_reference_for_all_rules_and_seeded_positions() {
    use crate::core::movegen::tests::{capture_test_positions, capture_test_rules};
    for rules in capture_test_rules() {
        let generator = crate::MoveGenerator::new(rules);
        let positions = capture_test_positions()
            .into_iter()
            .chain(crate::test_util::sampled_random_positions(rules));
        for position in positions {
            let mut moves = Vec::new();
            generator.generate_moves(&position, &mut moves);
            for mv in moves {
                let expected = reference_captured_squares(&position, mv);
                let actual = position.captured_squares(mv);
                assert_eq!(actual, expected, "rules={rules:?}, mv={mv:?}");
                let mut with_captures = position.clone();
                let mut ordinary = position.clone();
                let undo_with_captures =
                    with_captures.make_move_with_captures_unchecked(mv, rules, expected);
                let undo_ordinary = ordinary.make_move_unchecked(mv, rules);
                assert_eq!(with_captures, ordinary, "rules={rules:?}, mv={mv:?}");
                assert_eq!(undo_with_captures, undo_ordinary);
                with_captures.unmake_move(undo_with_captures);
                assert_eq!(with_captures, position);
            }
        }
    }
}

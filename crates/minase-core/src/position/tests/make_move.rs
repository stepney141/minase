//! 着手の適用と巻き戻し、およびその記録の試験。

use super::{position_after_non_lion_captures_lion, rebuild_board_with_side};
use crate::MoveGenerator;
use crate::board::Square;
use crate::mv::Move;
use crate::piece::{Color, PieceCode, PieceKind};
use crate::position::{Position, PositionBuilder};
use crate::rules::{MoveRules, PromotionRule};
use crate::test_util::{position as position_with_pieces, position_from_codes, sq};

// 先手から着手し、1手ごとに手番が相手へ移る(第6条1項)。
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
// (第4条4項・5項)。2段階移動の2枚捕獲では総駒数が2減る(第12条4項)。
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
/// 2枚捕獲・居喰い・じっと・成り・王駒捕獲・成駒の捕獲・先獅子・P1保留を覆う。
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

// 実装上の契約として、着手を取り消すと全観測可能状態
// (盤面・手番・キー・先獅子状態・成り権保留)が完全一致で復元される。
#[test]
fn unmake_restores_every_observable_component() {
    for (mut position, rules, moves) in make_unmake_scenarios() {
        let generator = MoveGenerator::new(rules);
        let mut trail = Vec::new();
        for mv in moves {
            let snapshot = position.clone();
            let undo = position.try_make_move_with_undo(mv, &generator).unwrap();
            trail.push((snapshot, undo));
        }
        while let Some((snapshot, undo)) = trail.pop() {
            position.unmake_move(undo);
            assert_eq!(position, snapshot);
        }
    }
}

// 実装上の契約として、手番パスは先獅子の一時状態を消滅させ、
// 禁止なしで手番だけを反転した同一配置のキーへ移る。
#[test]
fn null_move_clears_lion_trigger_and_round_trips() {
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

    position.unmake_null_move(undo);
    assert_eq!(position, before);
}

// 実装上の契約として、成り権保留と権利キーは手番パスで変化せず、
// 巻き戻すと局面全体が復元される。
#[test]
fn null_move_preserves_promotion_rights() {
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

    position.unmake_null_move(undo);
    assert_eq!(position, before);
}

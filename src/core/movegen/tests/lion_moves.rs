//! 第11条（角鷹・飛鷲の2段階移動）・第12条（獅子の基本動作）のテスト。
//!
//! 獅子の捕獲制限（第13〜16条）は領域D2が検証する。本ファイルの局面には
//! 取られる側の獅子を置かず、制限規則が発動しない構成だけを使う。

use std::collections::BTreeSet;

use super::dir::{B, BL, BR, F, FL, FR, L, R};
use super::{generated, jitto_moves, moves_from, msq, mv, same_board, step_squares};
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::Position;
use crate::core::rules::MoveRules;
use crate::core::square::Square;
use crate::test_util::{position, position_from_codes};

/// 先手の角鷹または飛鷲を含む局面を作る（成駒コードで配置する）。
fn with_promoted(
    side_to_move: Color,
    promoted: (u8, u8, PieceKind),
    others: &[((u8, u8), Color, PieceKind)],
) -> Position {
    let mut pieces = vec![(
        msq(promoted.0, promoted.1),
        PieceCode::new_promoted(Color::Black, promoted.2).unwrap(),
    )];
    for &((file, rank), color, kind) in others {
        pieces.push((msq(file, rank), PieceCode::new(color, kind)));
    }
    position_from_codes(side_to_move, &pieces)
}

// ---------------------------------------------------------------------------
// 第11条　角鷹・飛鷲の2段階移動
// ---------------------------------------------------------------------------

// D1-011-01: 角鷹の前方4動作（第11条1項a〜d）。
#[test]
fn article_11_1_falcon_has_four_forward_actions() {
    let board = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[
            ((6, 5), Color::White, PieceKind::Pawn),
            ((6, 4), Color::White, PieceKind::CopperGeneral),
        ],
    );
    let moves = generated(&board);
    // a: 1升進んで停止（歩兵を捕獲）。
    assert!(moves.contains(&mv(msq(6, 6), None, msq(6, 5), false)));
    // b: 居喰い（1升進んで元の升へ戻る）。
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 6), false)));
    // c: 1升進んだ後さらに1升（2枚取り）。
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 4), false)));
    // d: 中間駒を跳び越して2升目へ直接移動。
    assert!(moves.contains(&mv(msq(6, 6), None, msq(6, 4), false)));

    // 前方2升が空なら、正準形では a の非捕獲版と d の跳びだけが前方に現れる
    // （空升経由の2段階は跳びへ正準化される。move-canonicalization.md 決定1）。
    let empty_forward = with_promoted(Color::Black, (6, 6, PieceKind::HornedFalcon), &[]);
    let forward: Vec<_> = moves_from(&generated(&empty_forward), msq(6, 6))
        .into_iter()
        .filter(|m| m.to == msq(6, 5) || m.to == msq(6, 4))
        .collect();
    assert_eq!(forward.len(), 2);
    assert!(forward.iter().all(|m| m.mid.is_none()));
}

// D1-011-02: 飛鷲の左右前斜め2段階（第11条2項）。
#[test]
fn article_11_2_eagle_two_stage_on_both_forward_diagonals() {
    // 左前斜め（先手基準で (5,5)→(4,4) の筋）。
    let left = with_promoted(
        Color::Black,
        (6, 6, PieceKind::SoaringEagle),
        &[
            ((5, 5), Color::White, PieceKind::Pawn),
            ((4, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&left);
    assert!(moves.contains(&mv(msq(6, 6), None, msq(5, 5), false)));
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(5, 5)), msq(6, 6), false)));
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(5, 5)), msq(4, 4), false)));
    assert!(moves.contains(&mv(msq(6, 6), None, msq(4, 4), false)));
    // mid になり得るのは左右前斜めの隣接升だけ（この局面では (5,5)）。
    for m in moves_from(&moves, msq(6, 6)) {
        if let Some(mid) = m.mid {
            assert_eq!(mid, msq(5, 5), "move={m:?}");
        }
    }

    // 右前斜め ((7,5)→(8,4)) も独立に同じ4動作が成立する。
    let right = with_promoted(
        Color::Black,
        (6, 6, PieceKind::SoaringEagle),
        &[
            ((7, 5), Color::White, PieceKind::Pawn),
            ((8, 4), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&right);
    assert!(moves.contains(&mv(msq(6, 6), None, msq(7, 5), false)));
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(7, 5)), msq(6, 6), false)));
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(7, 5)), msq(8, 4), false)));
    assert!(moves.contains(&mv(msq(6, 6), None, msq(8, 4), false)));
}

// D1-011-03: 各段階での捕獲、1手で最大2枚（第11条3項）。
#[test]
fn article_11_3_two_stage_move_captures_up_to_two_pieces() {
    let mut board = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[
            ((6, 5), Color::White, PieceKind::Pawn),
            ((6, 4), Color::White, PieceKind::CopperGeneral),
        ],
    );
    board.make_move_unchecked(
        mv(msq(6, 6), Some(msq(6, 5)), msq(6, 4), false),
        MoveRules::standard(),
    );
    // 歩兵と銅将の両方が消え、角鷹が (6,4) にある。
    assert!(board.pieces_of(Color::White).is_empty());
    assert_eq!(
        board.piece_at(msq(6, 4)),
        PieceCode::new_promoted(Color::Black, PieceKind::HornedFalcon)
    );
}

// D1-011-04: 跳びは中間駒を取らない（第11条4項、第7条7項）。
#[test]
fn article_11_4_direct_jump_does_not_capture_the_intermediate() {
    for (kind, middle, target) in [
        (PieceKind::HornedFalcon, (6, 5), (6, 4)),
        (PieceKind::SoaringEagle, (5, 5), (4, 4)),
    ] {
        for middle_owner in [Color::Black, Color::White] {
            let board = with_promoted(
                Color::Black,
                (6, 6, kind),
                &[(middle, middle_owner, PieceKind::Pawn)],
            );
            let jump = mv(msq(6, 6), None, msq(target.0, target.1), false);
            // 中間升の所有者にかかわらず跳びは生成される。
            assert!(
                generated(&board).contains(&jump),
                "kind={kind:?}, owner={middle_owner:?}"
            );
            let mut after = board.clone();
            after.make_move_unchecked(jump, MoveRules::standard());
            assert_eq!(
                after.piece_at(msq(middle.0, middle.1)),
                board.piece_at(msq(middle.0, middle.1)),
                "kind={kind:?}, owner={middle_owner:?}"
            );
        }

        // 跳び先が自駒なら生成されない（D1-007-01）。
        let blocked = with_promoted(
            Color::Black,
            (6, 6, kind),
            &[(target, Color::Black, PieceKind::Pawn)],
        );
        assert!(
            !generated(&blocked).contains(&mv(msq(6, 6), None, msq(target.0, target.1), false)),
            "kind={kind:?}"
        );
    }
}

// D1-011-05: 角鷹の居喰い（第11条5項、第3条14号）。
#[test]
fn article_11_5_falcon_igui_captures_and_returns() {
    let board = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[((6, 5), Color::White, PieceKind::Pawn)],
    );
    let moves = generated(&board);
    let igui = mv(msq(6, 6), Some(msq(6, 5)), msq(6, 6), false);
    // 居喰いはちょうど1回現れる。
    assert_eq!(moves.iter().filter(|&&m| m == igui).count(), 1);
    // 前方隣接升が空でないため、じっとは生成されない（D1-011-07）。
    assert!(jitto_moves(&moves, msq(6, 6)).is_empty());

    let mut after = board.clone();
    after.make_move_unchecked(igui, MoveRules::standard());
    assert_eq!(
        after.piece_at(msq(6, 6)),
        PieceCode::new_promoted(Color::Black, PieceKind::HornedFalcon)
    );
    assert!(after.pieces_of(Color::White).is_empty());
    assert_eq!(after.side_to_move(), Color::White);
}

// D1-011-06: じっとの成立と一意性（第11条6項）。
#[test]
fn article_11_6_jitto_is_generated_exactly_once() {
    let falcon = with_promoted(Color::Black, (6, 6, PieceKind::HornedFalcon), &[]);
    let moves = generated(&falcon);
    assert_eq!(jitto_moves(&moves, msq(6, 6)).len(), 1);
    let mut after = falcon.clone();
    after.make_move_unchecked(mv(msq(6, 6), None, msq(6, 6), false), MoveRules::standard());
    assert!(same_board(&falcon, &after));
    assert_eq!(after.side_to_move(), Color::White);

    // 飛鷲は左右前斜めの両方が空でも、じっとは全体で1手だけ生成される。
    let eagle = with_promoted(Color::Black, (6, 6, PieceKind::SoaringEagle), &[]);
    assert_eq!(jitto_moves(&generated(&eagle), msq(6, 6)).len(), 1);
}

// D1-011-07: 角鷹のじっと不能条件（第11条7項）。
#[test]
fn article_11_7_falcon_jitto_requires_empty_forward_square() {
    // 前方隣接升が自駒: じっと不可。
    let own_forward = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[((6, 5), Color::Black, PieceKind::Pawn)],
    );
    assert!(jitto_moves(&generated(&own_forward), msq(6, 6)).is_empty());

    // 前方隣接升が相手駒: じっと不可だが居喰いは可（捕獲になる方向はじっとの根拠にならない）。
    let enemy_forward = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[((6, 5), Color::White, PieceKind::Pawn)],
    );
    let moves = generated(&enemy_forward);
    assert!(jitto_moves(&moves, msq(6, 6)).is_empty());
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 6), false)));

    // 段1（先手の最前）では前方が盤外なのでじっと不可。
    let at_edge = with_promoted(Color::Black, (6, 1, PieceKind::HornedFalcon), &[]);
    assert!(jitto_moves(&generated(&at_edge), msq(6, 1)).is_empty());
}

// D1-011-08: 飛鷲のじっと不能条件（第11条8項）。
#[test]
fn article_11_8_eagle_jitto_requires_an_empty_forward_diagonal() {
    // 両前斜めが駒（所有者不問）: じっと不可。
    let both_blocked = with_promoted(
        Color::Black,
        (6, 6, PieceKind::SoaringEagle),
        &[
            ((5, 5), Color::Black, PieceKind::Pawn),
            ((7, 5), Color::White, PieceKind::Pawn),
        ],
    );
    assert!(jitto_moves(&generated(&both_blocked), msq(6, 6)).is_empty());

    // 片方だけ塞がれた場合はじっと1手。
    let one_blocked = with_promoted(
        Color::Black,
        (6, 6, PieceKind::SoaringEagle),
        &[((5, 5), Color::Black, PieceKind::Pawn)],
    );
    assert_eq!(jitto_moves(&generated(&one_blocked), msq(6, 6)).len(), 1);

    // 片方盤外＋片方駒: じっと不可。
    let edge_file = with_promoted(
        Color::Black,
        (1, 6, PieceKind::SoaringEagle),
        &[((2, 5), Color::Black, PieceKind::Pawn)],
    );
    assert!(jitto_moves(&generated(&edge_file), msq(1, 6)).is_empty());

    // 段1では両前斜めが盤外なのでじっと不可。
    let edge_rank = with_promoted(Color::Black, (6, 1, PieceKind::SoaringEagle), &[]);
    assert!(jitto_moves(&generated(&edge_rank), msq(6, 1)).is_empty());
}

// D1-011-09: 第1段階捕獲後の盤面で第2段階を判定（第11条9項）。
#[test]
fn article_11_9_second_stage_uses_board_after_first_capture() {
    let board = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[
            ((6, 5), Color::White, PieceKind::Pawn),
            ((6, 4), Color::Black, PieceKind::SilverGeneral),
        ],
    );
    let moves = generated(&board);
    // 第2段階の到達先 (6,4) には除去後も自駒があるため生成されない。
    assert!(!moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 4), false)));
    // 跳びも (6,4) の自駒により生成されない。
    assert!(!moves.contains(&mv(msq(6, 6), None, msq(6, 4), false)));
    // 居喰いは from を離れた後の空升への帰還として合法。
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 6), false)));

    // 対照: (6,4) を相手駒に変えると2枚取りが生成される。
    let enemy_second = with_promoted(
        Color::Black,
        (6, 6, PieceKind::HornedFalcon),
        &[
            ((6, 5), Color::White, PieceKind::Pawn),
            ((6, 4), Color::White, PieceKind::SilverGeneral),
        ],
    );
    assert!(generated(&enemy_second).contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 4), false)));
}

// D1-011-10: 角鷹・飛鷲は1手の途中でも成らない（第11条10項、第17条4項）。
#[test]
fn article_11_10_falcon_and_eagle_moves_never_promote() {
    for (kind, first) in [
        (PieceKind::HornedFalcon, (6, 5)),
        (PieceKind::SoaringEagle, (5, 5)),
    ] {
        // 敵陣直前から敵陣内の駒を取る2段階移動でも成り変種は存在しない。
        let board = with_promoted(
            Color::Black,
            (6, 5, kind),
            &[((first.0, first.1 - 1), Color::White, PieceKind::Pawn)],
        );
        let moves = moves_from(&generated(&board), msq(6, 5));
        assert!(moves.iter().any(|m| m.mid.is_some()), "kind={kind:?}");
        assert!(moves.iter().all(|m| !m.promote), "kind={kind:?}");
    }
}

// ---------------------------------------------------------------------------
// 第12条　獅子の基本動作
// ---------------------------------------------------------------------------

// D1-012-01: 1升移動（第12条1項・3項）。王将と同じ8方向。
#[test]
fn article_12_1_3_lion_single_steps() {
    let board = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::Lion)]);
    let moves = generated(&board);
    for square in step_squares((6, 6), &[F, B, L, R, FL, FR, BL, BR]) {
        assert!(
            moves.contains(&mv(msq(6, 6), None, square, false)),
            "{square:?}"
        );
    }
}

// D1-012-02: 2升への直接跳び。ナイト位置を含む16升（第12条5〜7項・2項）。
#[test]
fn article_12_5_7_lion_jumps_two_squares_including_knight_positions() {
    let board = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::Lion)]);
    let moves = generated(&board);
    // max(|df|,|dr|)=2 の16升（縦横斜めの2升先8＋ナイト位置8）。
    let jump_deltas: Vec<(i16, i16)> = (-2..=2_i16)
        .flat_map(|df| (-2..=2_i16).map(move |dr| (df, dr)))
        .filter(|&(df, dr)| df.abs().max(dr.abs()) == 2)
        .collect();
    assert_eq!(jump_deltas.len(), 16);
    for square in step_squares((6, 6), &jump_deltas) {
        assert!(
            moves.contains(&mv(msq(6, 6), None, square, false)),
            "{square:?}"
        );
    }
    // 3升以上先への着手は存在しない。
    assert!(!moves.iter().any(|m| m.to == msq(6, 3) || m.to == msq(3, 6)));

    // 中間升に駒があっても跳べ、その駒は取らない（第12条7項）。
    let occupied_middle = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let jump = mv(msq(6, 6), None, msq(6, 4), false);
    assert!(generated(&occupied_middle).contains(&jump));
    let mut after = occupied_middle.clone();
    after.make_move_unchecked(jump, MoveRules::standard());
    assert_eq!(
        after.piece_at(msq(6, 5)),
        Some(PieceCode::new(Color::White, PieceKind::Pawn))
    );
}

// D1-012-03: 経路捕獲つき2段階移動と最大2枚取り（第12条4項・12項）。
#[test]
fn article_12_4_lion_path_capture_double_move() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
            (msq(5, 4), Color::White, PieceKind::CopperGeneral),
        ],
    );
    let moves = generated(&board);
    // mid=(6,5) の第2段階は (6,5) の周囲8升すべて（居喰い (6,6) を含む）。
    let second_stage: BTreeSet<Square> = moves
        .iter()
        .filter(|m| m.mid == Some(msq(6, 5)))
        .map(|m| m.to)
        .collect();
    assert_eq!(
        second_stage,
        step_squares((6, 5), &[F, B, L, R, FL, FR, BL, BR])
    );

    // 2枚取りの適用（第12条4項）。
    let mut after = board.clone();
    after.make_move_unchecked(
        mv(msq(6, 6), Some(msq(6, 5)), msq(5, 4), false),
        MoveRules::standard(),
    );
    assert!(after.pieces_of(Color::White).is_empty());
    assert_eq!(
        after.piece_at(msq(5, 4)),
        Some(PieceCode::new(Color::Black, PieceKind::Lion))
    );

    // mid になり得るのは from に隣接する相手駒の升だけ。2升先の銅将 (5,4) は
    // mid にならない。
    assert!(!moves.iter().any(|m| m.mid == Some(msq(5, 4))));
}

// D1-012-04: 跳び捕獲と経路捕獲は別の着手（第12条4項・7項）。
#[test]
fn article_12_4_7_jump_capture_and_path_capture_are_distinct_moves() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&board);
    let jump = mv(msq(6, 6), None, msq(6, 4), false);
    let path = mv(msq(6, 6), Some(msq(6, 5)), msq(6, 4), false);
    // 同一の (from, to) でも mid の有無が異なる2手が共存する。
    assert!(moves.contains(&jump));
    assert!(moves.contains(&path));

    // 跳びは歩兵を残し、経路捕獲は歩兵を取る。
    let mut jumped = board.clone();
    jumped.make_move_unchecked(jump, MoveRules::standard());
    assert_eq!(
        jumped.piece_at(msq(6, 5)),
        Some(PieceCode::new(Color::White, PieceKind::Pawn))
    );
    let mut captured = board.clone();
    captured.make_move_unchecked(path, MoveRules::standard());
    assert!(captured.pieces_of(Color::White).is_empty());
}

// D1-012-05: 獅子の居喰い（第12条8項、第3条14号）。
#[test]
fn article_12_8_lion_igui() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
        ],
    );
    let moves = generated(&board);
    let igui = mv(msq(6, 6), Some(msq(6, 5)), msq(6, 6), false);
    assert_eq!(moves.iter().filter(|&&m| m == igui).count(), 1);
    let mut after = board.clone();
    after.make_move_unchecked(igui, MoveRules::standard());
    assert_eq!(
        after.piece_at(msq(6, 6)),
        Some(PieceCode::new(Color::Black, PieceKind::Lion))
    );
    assert!(after.pieces_of(Color::White).is_empty());

    // 隣接する相手駒ごとに独立の居喰いが1手ずつ生成される。
    let two_neighbors = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
            (msq(7, 6), Color::White, PieceKind::Pawn),
        ],
    );
    let iguis: Vec<_> = generated(&two_neighbors)
        .into_iter()
        .filter(|m| m.from == msq(6, 6) && m.to == msq(6, 6) && m.mid.is_some())
        .collect();
    assert_eq!(iguis.len(), 2);
}

// D1-012-06: 獅子のじっと（第12条9項・11項）。
#[test]
fn article_12_9_lion_jitto() {
    // 空の隣接升が複数あっても、じっとは1手だけ生成される。
    let board = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::Lion)]);
    let moves = generated(&board);
    assert_eq!(jitto_moves(&moves, msq(6, 6)).len(), 1);
    let mut after = board.clone();
    after.make_move_unchecked(mv(msq(6, 6), None, msq(6, 6), false), MoveRules::standard());
    assert!(same_board(&board, &after));
    assert_eq!(after.side_to_move(), Color::White);
}

// D1-012-07: じっと不能条件（第12条10項・9項）。
#[test]
fn article_12_10_lion_jitto_requires_an_empty_adjacent_square() {
    // 盤隅 (1,1) の獅子。隣接3升がすべて駒（所有者不問）で埋まっている。
    let board = position(
        Color::Black,
        &[
            (msq(1, 1), Color::Black, PieceKind::Lion),
            (msq(1, 2), Color::Black, PieceKind::Pawn),
            (msq(2, 1), Color::White, PieceKind::Pawn),
            (msq(2, 2), Color::White, PieceKind::CopperGeneral),
        ],
    );
    let moves = generated(&board);
    assert!(jitto_moves(&moves, msq(1, 1)).is_empty());
    // じっと不能でも居喰いと2升先への跳びは生成され得る。
    assert!(moves.contains(&mv(msq(1, 1), Some(msq(2, 1)), msq(1, 1), false)));
    assert!(moves.contains(&mv(msq(1, 1), None, msq(1, 3), false)));
}

// D1-012-08: 逐次判定（第12条12項）。第1段階の除去後の盤面で第2段階を判定する。
#[test]
fn article_12_12_second_stage_judged_sequentially() {
    let board = position(
        Color::Black,
        &[
            (msq(6, 6), Color::Black, PieceKind::Lion),
            (msq(6, 5), Color::White, PieceKind::Pawn),
            (msq(7, 4), Color::Black, PieceKind::GoldGeneral),
        ],
    );
    let moves = generated(&board);
    // from=(6,6) は駒が離れた後なので居喰いが合法。
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(6, 6), false)));
    // 除去後も自駒が残る (7,4) へは行けない。
    assert!(!moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(7, 4), false)));
    // 空いた (5,4) へは行ける。
    assert!(moves.contains(&mv(msq(6, 6), Some(msq(6, 5)), msq(5, 4), false)));
}

// D1-012-09 / D1-009-21: 裸獅子の正準形25手（第12条1〜9項、
// move-canonicalization.md）。内訳は1升移動8手＋2升跳び16手＋じっと1手。
// 正準形の手数はimplementation contract（move-canonicalization.md 決定1）。
#[test]
fn article_12_lone_lion_generates_25_canonical_moves() {
    let board = position(Color::Black, &[(msq(6, 6), Color::Black, PieceKind::Lion)]);
    let moves = generated(&board);
    assert_eq!(moves.len(), 25);
    assert_eq!(moves.iter().filter(|m| m.to != m.from).count(), 24);
    assert_eq!(jitto_moves(&moves, msq(6, 6)).len(), 1);
    // 捕獲対象がないため mid ありの着手は0手。
    assert!(moves.iter().all(|m| m.mid.is_none()));
}

//! 横断性質のテスト。
//!
//! D1-009-24（先後の180度回転対称）と、move-canonicalization.md 由来の
//! 正準形不変条件（D1-MC-01）・適用と復元の往復（D1-MC-02）を、シード固定の
//! 決定的プレイアウトで検査する。正準形の符号化そのものは実装契約
//! （implementation contract: move-canonicalization.md）である。

use std::collections::{BTreeSet, HashSet};
use std::num::{NonZeroU64, NonZeroUsize};

use super::{
    MoveGenerator, PROMOTION_PAIRS, TWO_STAGE_KINDS, forward_rank_delta, generated_captures_with,
    generated_with, msq, same_board,
};
use crate::core::mv::Move;
use crate::core::piece::{Color, PieceCode, PieceKind};
use crate::core::position::{Position, PositionBuilder};
use crate::core::rules::{LionRule, MoveRules, PromotionRule, Rules};
use crate::core::square::Square;
use crate::rng::XorShift64;

/// 盤の180度回転写像（第5条の盤は12×12）。
fn rotate_square(square: Square) -> Square {
    Square::new(11 - square.file(), 11 - square.rank()).unwrap()
}

/// 着手全体への180度回転写像。
fn rotate_move(mv: Move) -> Move {
    Move {
        from: rotate_square(mv.from),
        mid: mv.mid.map(rotate_square),
        to: rotate_square(mv.to),
        promote: mv.promote,
    }
}

/// 単駒の局面を作る。
fn single_piece(side_to_move: Color, square: Square, code: PieceCode) -> Position {
    let mut builder = PositionBuilder::new(side_to_move);
    builder.put(square, code).unwrap();
    builder.finish().unwrap()
}

// D1-009-24: 後手駒の到達升集合は、先手駒を180度回転した升に置いた集合の回転像と
// 一致する（第9条、第3条3号）。成駒にも適用される（D1-010系の性質）。
#[test]
fn article_9_black_and_white_move_sets_are_180_degree_symmetric() {
    let promoted_only = [
        PieceKind::WhiteHorse,
        PieceKind::Whale,
        PieceKind::FlyingStag,
        PieceKind::FreeBoar,
        PieceKind::FlyingOx,
        PieceKind::CrownPrince,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ];
    let generator = MoveGenerator::standard();
    for kind in PieceKind::ALL {
        // 未成コード（初期駒21種）と成駒コード（成り先18種）の両方を検査する。
        let mut codes: Vec<(PieceCode, PieceCode)> = Vec::new();
        if !promoted_only.contains(&kind) {
            codes.push((
                PieceCode::new(Color::White, kind)
                    .expect("generated fixture uses an unpromoted-capable kind"),
                PieceCode::new(Color::Black, kind)
                    .expect("generated fixture uses an unpromoted-capable kind"),
            ));
        }
        if let (Some(white), Some(black)) = (
            PieceCode::new_promoted(Color::White, kind),
            PieceCode::new_promoted(Color::Black, kind),
        ) {
            codes.push((white, black));
        }

        for (white_code, black_code) in codes {
            for square in Square::all() {
                let white_moves: HashSet<Move> =
                    generated_with(&generator, &single_piece(Color::White, square, white_code))
                        .into_iter()
                        .collect();
                let black_moves: HashSet<Move> = generated_with(
                    &generator,
                    &single_piece(Color::Black, rotate_square(square), black_code),
                )
                .into_iter()
                .collect();
                let rotated: HashSet<Move> = white_moves.into_iter().map(rotate_move).collect();
                assert_eq!(
                    rotated,
                    black_moves,
                    "kind={kind:?}, square={square:?}, promoted={}",
                    white_code.is_promoted()
                );
            }
        }
    }
}

/// 着手が捕獲する升の期待集合（mid と、相手駒がいる到達升。第7条3項・第12条4項）。
fn expected_captures(position: &Position, mv: Move) -> BTreeSet<Square> {
    let enemy = position.pieces_of(position.side_to_move().opposite());
    let mut captures = BTreeSet::new();
    if let Some(mid) = mv.mid {
        captures.insert(mid);
    }
    if mv.to != mv.from && enemy.contains(mv.to) {
        captures.insert(mv.to);
    }
    captures
}

/// 全手列から規則上の捕獲手だけを残した列と、捕獲専用生成の列が一致することを検査する。
fn assert_capture_generation_matches(generator: &MoveGenerator, position: &Position) {
    let expected: Vec<_> = generated_with(generator, position)
        .into_iter()
        .filter(|&mv| !expected_captures(position, mv).is_empty())
        .collect();
    let actual = generated_captures_with(generator, position);
    assert_eq!(actual, expected);
}

/// 指定駒だけを置いた局面と、残りの全升を相手駒で埋めた局面を作る。
fn capture_extreme_position(from: Square, code: PieceCode, dense: bool) -> Position {
    let mut builder = PositionBuilder::new(Color::Black);
    builder.put(from, code).unwrap();
    if dense {
        let enemy = PieceCode::new(Color::White, PieceKind::Pawn).unwrap();
        for square in Square::all().filter(|&square| square != from) {
            builder.put(square, enemy).unwrap();
        }
    }
    builder.finish().unwrap()
}

// strength-stage1.md「捕獲専用生成」: 全駒種と全升について、空升だけの局面では
// 捕獲を生成せず、他の全升が相手駒の局面では全手列の捕獲部分列と一致する。
#[test]
fn capture_generation_matches_full_generation_for_every_kind_and_square() {
    let generator = MoveGenerator::standard();
    let promoted_only = [
        PieceKind::WhiteHorse,
        PieceKind::Whale,
        PieceKind::FlyingStag,
        PieceKind::FreeBoar,
        PieceKind::FlyingOx,
        PieceKind::CrownPrince,
        PieceKind::HornedFalcon,
        PieceKind::SoaringEagle,
    ];

    for kind in PieceKind::ALL {
        let mut codes = Vec::new();
        if !promoted_only.contains(&kind) {
            codes.push(PieceCode::new(Color::Black, kind).unwrap());
        }
        if let Some(promoted) = PieceCode::new_promoted(Color::Black, kind) {
            codes.push(promoted);
        }

        for code in codes {
            for from in Square::all() {
                let sparse = capture_extreme_position(from, code, false);
                assert!(
                    generated_captures_with(&generator, &sparse).is_empty(),
                    "kind={kind:?}, from={from:?}, promoted={}",
                    code.is_promoted()
                );

                let dense = capture_extreme_position(from, code, true);
                assert_capture_generation_matches(&generator, &dense);
            }
        }
    }
}

// strength-stage1.md「捕獲専用生成」: 獅子、角鷹、飛鷲の経由升捕獲、
// 到達升捕獲、2枚取り、居喰い、および捕獲を伴う成りを同じ列で返す。
#[test]
fn capture_generation_matches_full_generation_for_special_and_promoting_captures() {
    let mut builder = PositionBuilder::new(Color::Black);
    let black = |kind| PieceCode::new(Color::Black, kind).unwrap();
    let promoted = |kind| PieceCode::new_promoted(Color::Black, kind).unwrap();
    let enemy = PieceCode::new(Color::White, PieceKind::Pawn).unwrap();

    builder.put(msq(6, 7), black(PieceKind::Lion)).unwrap();
    for square in [msq(6, 6), msq(6, 5), msq(7, 5)] {
        builder.put(square, enemy).unwrap();
    }

    builder
        .put(msq(3, 7), promoted(PieceKind::HornedFalcon))
        .unwrap();
    for square in [msq(3, 6), msq(3, 5)] {
        builder.put(square, enemy).unwrap();
    }

    builder
        .put(msq(10, 7), promoted(PieceKind::SoaringEagle))
        .unwrap();
    for square in [msq(9, 6), msq(8, 5), msq(12, 5)] {
        builder.put(square, enemy).unwrap();
    }

    builder
        .put(msq(1, 3), black(PieceKind::FerociousLeopard))
        .unwrap();
    builder.put(msq(1, 2), enemy).unwrap();

    let position = builder.finish().unwrap();
    assert_capture_generation_matches(&MoveGenerator::standard(), &position);

    let captures = generated_captures_with(&MoveGenerator::standard(), &position);
    assert!(
        captures
            .iter()
            .any(|mv| mv.mid.is_some() && mv.to == mv.from)
    );
    assert!(
        captures
            .iter()
            .any(|mv| { expected_captures(&position, *mv).len() == 2 })
    );
    assert!(
        captures
            .iter()
            .any(|mv| mv.mid.is_none() && mv.to != mv.from)
    );
    assert!(captures.iter().any(|mv| mv.promote));

    let sentinel = Move {
        from: msq(12, 12),
        mid: None,
        to: msq(12, 12),
        promote: false,
    };
    let mut appended = vec![sentinel];
    MoveGenerator::standard().generate_captures(&position, &mut appended);
    assert_eq!(appended[0], sentinel);
    assert_eq!(&appended[1..], captures);
}

/// 正準形の不変条件（D1-MC-01）と第6〜7条・第17〜18条の全域性質を1局面分検査する。
fn assert_canonical_invariants(position: &Position, moves: &[Move], ply: usize) {
    let side = position.side_to_move();
    let own = position.pieces_of(side);
    let enemy = position.pieces_of(side.opposite());
    let forward = forward_rank_delta(side);
    let move_set: HashSet<Move> = moves.iter().copied().collect();
    // (4) 同一の (from, mid, to, 成り選択) は高々1回（D1-019-05の性質も兼ねる）。
    assert_eq!(move_set.len(), moves.len(), "duplicate move at ply {ply}");

    for &m in moves {
        let piece = position
            .piece_at(m.from)
            .unwrap_or_else(|| panic!("empty from at ply {ply}: {m:?}"));
        let kind = piece.kind().unwrap();
        // 手番側の駒だけが動く（第6条1項・2項、第26条1項の裏面）。
        assert_eq!(piece.color(), Some(side), "ply={ply}, move={m:?}");
        // 到達升は自駒升でない（第7条2項）。
        if m.to != m.from {
            assert!(!own.contains(m.to), "ply={ply}, move={m:?}");
        }
        // (3) to=from かつ mid なしはじっとで、駒は獅子・角鷹・飛鷲（第6条5項）。
        if m.to == m.from && m.mid.is_none() {
            assert!(TWO_STAGE_KINDS.contains(&kind), "ply={ply}, move={m:?}");
        }
        if let Some(mid) = m.mid {
            // (1) mid には適用前に相手駒がある。
            assert!(enemy.contains(mid), "ply={ply}, move={m:?}");
            // 同じ駒を2回取らない（第7条9項）。
            assert_ne!(mid, m.from, "ply={ply}, move={m:?}");
            assert_ne!(mid, m.to, "ply={ply}, move={m:?}");
            // mid あり ⇒ 成り選択なし（第18条6項・7項、D1-018-08）。
            assert!(!m.promote, "ply={ply}, move={m:?}");
            // (2) mid を持てるのは獅子・角鷹・飛鷲だけで、mid は駒種ごとの
            // 第1段階の隣接升に限る（第11条1項・2項、第12条）。
            let df = i16::from(mid.file()) - i16::from(m.from.file());
            let dr = i16::from(mid.rank()) - i16::from(m.from.rank());
            match kind {
                PieceKind::Lion => {
                    assert_eq!(df.abs().max(dr.abs()), 1, "ply={ply}, move={m:?}");
                }
                PieceKind::HornedFalcon => {
                    assert_eq!((df, dr), (0, i16::from(forward)), "ply={ply}, move={m:?}");
                }
                PieceKind::SoaringEagle => {
                    assert_eq!(df.abs(), 1, "ply={ply}, move={m:?}");
                    assert_eq!(dr, i16::from(forward), "ply={ply}, move={m:?}");
                }
                other => panic!("mid on non two-stage kind {other:?}: ply={ply}, move={m:?}"),
            }
        }
        if m.promote {
            // 成り選択ありには不成の対がある（第18条5項）。
            assert!(
                move_set.contains(&Move {
                    promote: false,
                    ..m
                }),
                "ply={ply}, move={m:?}"
            );
            // 成れるのは未成の18種だけ（第17条1項・2項・4項）。
            assert!(!piece.is_promoted(), "ply={ply}, move={m:?}");
            assert!(
                PROMOTION_PAIRS.iter().any(|&(base, _)| base == kind),
                "ply={ply}, move={m:?}"
            );
        }
    }
}

/// 着手適用後の盤面変化が第6条2項・第7条3項・第9条（成り先18組）に従うことを検査する。
fn assert_application_effects(before: &Position, mv: Move, rules: MoveRules, ply: usize) {
    let side = before.side_to_move();
    let captures = expected_captures(before, mv);
    let mut after = before.clone();
    after.make_move_unchecked(mv, rules);

    // 手番は必ず相手へ移る（第6条1項、第12条11項）。
    assert_eq!(after.side_to_move(), side.opposite(), "ply={ply}");
    // 自駒数は不変、相手駒数は捕獲分だけ減る（第4条4項、第6条2項）。
    assert_eq!(
        before.pieces_of(side).popcount(),
        after.pieces_of(side).popcount(),
        "ply={ply}, move={mv:?}"
    );
    assert_eq!(
        before.pieces_of(side.opposite()).popcount(),
        after.pieces_of(side.opposite()).popcount() + captures.len() as u32,
        "ply={ply}, move={mv:?}"
    );
    // 変化する升は from・to・捕獲升に限る。
    for square in Square::all() {
        if before.piece_at(square) != after.piece_at(square) {
            assert!(
                square == mv.from || square == mv.to || captures.contains(&square),
                "ply={ply}, move={mv:?}, square={square:?}"
            );
        }
    }
    // じっとなら盤面は不変（第6条4項）。
    if mv.to == mv.from && mv.mid.is_none() {
        assert!(same_board(before, &after), "ply={ply}, move={mv:?}");
    }
    // 駒種の変化は成り対応18組の向きに限る（D1-009-23の性質、第17条3項）。
    let moved_before = before.piece_at(mv.from).unwrap();
    let moved_after = after.piece_at(mv.to).unwrap();
    if mv.promote {
        let promoted_kind = PROMOTION_PAIRS
            .iter()
            .find(|&&(base, _)| base == moved_before.kind().unwrap())
            .map(|&(_, promoted)| promoted)
            .unwrap();
        assert_eq!(
            Some(moved_after),
            PieceCode::new_promoted(side, promoted_kind),
            "ply={ply}, move={mv:?}"
        );
    } else {
        assert_eq!(moved_after, moved_before, "ply={ply}, move={mv:?}");
    }
}

/// シード固定プレイアウトで正準形不変条件と適用効果を検査する（D1-MC-01）。
fn playout_canonical(rules: MoveRules, seed: u64, plies: usize) {
    let generator = MoveGenerator::new(rules);
    let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
    let mut position = Position::initial();
    for ply in 0..plies {
        let moves = generated_with(&generator, &position);
        if moves.is_empty() {
            break;
        }
        assert_canonical_invariants(&position, &moves, ply);
        let mv = moves[rng.index(NonZeroUsize::new(moves.len()).unwrap())];
        assert_application_effects(&position, mv, rules, ply);
        position.make_move_unchecked(mv, rules);
    }
}

// D1-MC-01: 正準形の不変条件（move-canonicalization.md。implementation contract）。
// 併せて第6〜7条・第17〜18条の全域性質を到達可能局面で検査する。
#[test]
fn mc_canonical_move_invariants_hold_in_seeded_playouts() {
    playout_canonical(MoveRules::standard(), 0x5255_4c45_5345_5401, 96);
    playout_canonical(
        MoveRules {
            promotion: PromotionRule::P1,
            p3: true,
            p4: true,
            ..MoveRules::standard()
        },
        0x5255_4c45_5345_5402,
        96,
    );
}

/// シード固定プレイアウトの各局面で捕獲専用生成の同値性を検査する。
fn playout_capture_generation(rules: MoveRules, seed: u64, plies: usize) {
    let generator = MoveGenerator::new(rules);
    let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
    let mut position = Position::initial();
    for _ in 0..plies {
        assert_capture_generation_matches(&generator, &position);
        let moves = generated_with(&generator, &position);
        assert!(
            moves
                .iter()
                .all(|&mv| generator.is_legal_move(&position, mv))
        );
        let expected_quiets: Vec<_> = moves
            .iter()
            .copied()
            .filter(|&mv| expected_captures(&position, mv).is_empty())
            .collect();
        let mut quiets = Vec::new();
        generator.generate_quiets(&position, &mut quiets);
        assert_eq!(quiets, expected_quiets);
        if moves.is_empty() {
            break;
        }
        position.make_move_unchecked(
            moves[rng.index(NonZeroUsize::new(moves.len()).unwrap())],
            rules,
        );
    }
}

// strength-stage1.md「捕獲専用生成」: 到達可能局面でも、捕獲専用生成は
// 全手列から規則上の捕獲だけを残した部分列と一致する。
#[test]
fn capture_generation_matches_full_generation_in_seeded_playouts() {
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
    for (index, rules) in rule_sets.into_iter().enumerate() {
        playout_capture_generation(rules, 0x4341_5054_5552_4501 + index as u64, 128);
    }
}

/// シード固定プレイアウトで全生成手の適用・復元の往復を検査する（D1-MC-02）。
fn playout_round_trip(rules: MoveRules, seed: u64, plies: usize) {
    let generator = MoveGenerator::new(rules);
    let mut rng = XorShift64::new(NonZeroU64::new(seed).unwrap());
    let mut position = Position::initial();
    for ply in 0..plies {
        let moves = generated_with(&generator, &position);
        if moves.is_empty() {
            break;
        }
        let before = position.clone();
        for &mv in &moves {
            // 2枚取り・居喰い・じっと・成りを含む全種の着手で完全に復元される。
            let undo = position.make_move_unchecked(mv, rules);
            position.unmake_move(undo);
            assert_eq!(position, before, "ply={ply}, move={mv:?}");
        }
        position.make_move_unchecked(
            moves[rng.index(NonZeroUsize::new(moves.len()).unwrap())],
            rules,
        );
    }
}

// D1-MC-02: 着手の適用と復元の往復（move-canonicalization.md 検証戦略。
// implementation contract。局面ハッシュ等の内部表現の検証は領域D4）。
#[test]
fn mc_make_unmake_round_trips_every_generated_move() {
    playout_round_trip(MoveRules::standard(), 0x5255_4c45_5345_5403, 64);
    // P1の成り権状態（第24条1項dに関わる一時的権利）も含めて復元されることを、
    // P1採用プレイアウトで確認する。
    playout_round_trip(
        MoveRules {
            promotion: PromotionRule::P1,
            ..MoveRules::standard()
        },
        0x5255_4c45_5345_5404,
        64,
    );
}

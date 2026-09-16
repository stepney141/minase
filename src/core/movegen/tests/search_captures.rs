//! 設計書movegen-speedup.md「捕獲対象を生成前に除外する」の契約テスト。
use super::*;
use crate::Bitboard;
use crate::core::movegen::{CaptureCache, CaptureCandidate};

// 公開捕獲の集合・順序・捕獲升を保存する。期待値は公開APIの契約による。
#[test]
fn split_captures_preserve_public_order_and_captured_squares() {
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for position in capture_test_positions() {
            let mut split = Vec::new();
            let enemy = position.pieces_of(position.side_to_move().opposite());
            let mut capturers = Vec::new();
            generator.collect_ordinary_capturers(&position, enemy, &mut capturers);
            generator.emit_ordinary_captures(&position, &capturers, enemy, &mut |c| split.push(c));
            generator.generate_special_captures(&position, &mut |c| split.push(c));
            // 安定整列は公開生成順を表すキーによる、テスト専用の独立したマージ。
            split.sort_by_key(|c| {
                (
                    position
                        .piece_at(c.mv.from)
                        .unwrap()
                        .kind()
                        .unwrap()
                        .index(),
                    c.mv.from.raw_index(),
                )
            });
            let mut reference = Vec::new();
            generator.generate_captures(&position, &mut reference);
            let reference: Vec<_> = reference
                .into_iter()
                .map(|mv| CaptureCandidate {
                    mv,
                    piece: position.piece_at(mv.from).unwrap(),
                    captured: position.captured_squares(mv),
                })
                .collect();
            assert_eq!(split, reference, "rules={rules:?}");
        }
    }
}

// 同節: 通常駒の部分生成は、全捕獲の到達升による抽出と順序まで一致する。
#[test]
fn ordinary_targets_select_exactly_the_public_subsequence() {
    let mut state = 0x6275_6666_6572_u64;
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        for position in capture_test_positions() {
            let enemy = position.pieces_of(position.side_to_move().opposite());
            let mut all = Vec::new();
            let mut capturers = Vec::new();
            generator.collect_ordinary_capturers(&position, enemy, &mut capturers);
            generator.emit_ordinary_captures(&position, &capturers, enemy, &mut |c| all.push(c));
            let mut masks = vec![Bitboard::EMPTY, enemy];
            masks.extend(enemy.into_iter().map(|s| Bitboard::from_squares([s])));
            for _ in 0..8 {
                masks.push(Bitboard::from_squares(enemy.into_iter().filter(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state & 1 != 0
                })));
            }
            for targets in masks {
                let mut actual = Vec::new();
                generator.emit_ordinary_captures(&position, &capturers, targets, &mut |c| {
                    actual.push(c)
                });
                let mut restricted = Vec::new();
                generator.collect_ordinary_capturers(&position, targets, &mut restricted);
                let mut recollected = Vec::new();
                generator.emit_ordinary_captures(&position, &restricted, targets, &mut |c| {
                    recollected.push(c)
                });
                assert_eq!(
                    actual, recollected,
                    "collecting a subset preserves its captures"
                );
                let expected: Vec<_> = all
                    .iter()
                    .copied()
                    .filter(|c| targets.contains(c.mv.to))
                    .collect();
                assert_eq!(actual, expected, "rules={rules:?}, targets={targets:?}");
            }
        }
    }
}

// 単位B2「置換表の手の検査を捕獲生成だけで行う」: 合法性の正は公開生成。
// 全合法手に加え、成り・経由升を変えた候補と、空升・相手駒からの候補を照合する。
#[test]
fn tt_capture_validation_matches_filtered_public_captures() {
    for rules in capture_test_rules() {
        let generator = MoveGenerator::new(rules);
        let mut raw = CaptureCache::default();
        for position in capture_test_positions() {
            let mut captures = Vec::new();
            generator.generate_captures(&position, &mut captures);
            let mut candidates = Vec::new();
            generator.generate_moves(&position, &mut candidates);
            let originals = candidates.clone();
            for mv in originals {
                candidates.push(Move {
                    promote: !mv.promote,
                    ..mv
                });
                candidates.push(Move {
                    mid: Some(mv.from),
                    ..mv
                });
            }
            if let Some(&mv) = captures.first() {
                for from in Square::all() {
                    candidates.push(Move { from, ..mv });
                }
            }
            for mv in candidates {
                assert_eq!(
                    generator.is_legal_capture(&position, mv, &mut raw),
                    captures.contains(&mv)
                        && !MoveGenerator::is_excluded_lion_capture(&position, mv),
                    "rules={rules:?}, mv={mv:?}",
                );
            }
        }
    }
}

// 設計書movegen-speedup-2.md「段階8」: 除外条件の各境界と対象外の特殊駒を固定する。
#[test]
fn lion_capture_exclusion_keeps_igui_double_stop_and_jump() {
    use crate::test_util::{position_from_codes, sq};
    for color in Color::ALL {
        for kind in [
            PieceKind::Lion,
            PieceKind::HornedFalcon,
            PieceKind::SoaringEagle,
        ] {
            let from = sq(5, 5);
            let mid = sq(5, 6);
            let victim = sq(6, 6);
            let empty = sq(4, 6);
            let friendly = sq(4, 5);
            let piece = match kind {
                PieceKind::Lion => crate::PieceCode::new(color, kind).unwrap(),
                _ => crate::PieceCode::new_promoted(color, kind).unwrap(),
            };
            let position = position_from_codes(
                color,
                &[
                    (from, piece),
                    (
                        mid,
                        crate::PieceCode::new(color.opposite(), PieceKind::Pawn).unwrap(),
                    ),
                    (
                        victim,
                        crate::PieceCode::new(color.opposite(), PieceKind::Pawn).unwrap(),
                    ),
                    (
                        friendly,
                        crate::PieceCode::new(color, PieceKind::Pawn).unwrap(),
                    ),
                ],
            );
            for (via, to, excluded) in [
                (Some(mid), empty, kind == PieceKind::Lion),
                (Some(mid), from, false),
                (Some(mid), victim, false),
                (None, mid, false),
                (None, victim, false),
                (Some(empty), victim, false),
                (Some(friendly), empty, false),
                (Some(mid), friendly, false),
            ] {
                let mv = Move {
                    from,
                    mid: via,
                    to,
                    promote: false,
                };
                assert_eq!(
                    MoveGenerator::is_excluded_lion_capture(&position, mv),
                    excluded,
                    "color={color:?}, kind={kind:?}, mv={mv:?}"
                );
            }
        }
    }
}

//! 獅子力の全移動形式と同一局面の重複排除を検査する。

use super::*;

/// 中央の獅子力の着手を、在庫を満たした相手と着手側の麒麟で囲む。
fn fixture(color: Color, kind: PieceKind, from: Square, captured: &[Square]) -> Position {
    let mut pieces = vec![
        (from, piece(color, kind)),
        (sq(11, 6), piece(color, PieceKind::Kirin)),
    ];
    if kind != PieceKind::Lion {
        pieces.push((sq(0, 6), piece(color, PieceKind::Lion)));
    }
    pieces.extend(
        captured
            .iter()
            .map(|&s| (s, piece(color.opposite(), PieceKind::Pawn))),
    );
    stocked_position(color, &pieces)
}

/// 獅子の1段階移動と中間の駒を残す直接跳びを検査する。
#[test]
fn lion_steps_and_jumps_with_and_without_capture() {
    // RULES.md第12条1・5・7項: 1段階移動と直接跳びの捕獲先は到達升だけ。
    let from = sq(5, 5);
    for to in [sq(6, 5), sq(7, 5)] {
        for capture in [false, true] {
            let captures = if capture { vec![to] } else { vec![] };
            let mut p = fixture(Color::Black, PieceKind::Lion, from, &captures);
            if to == sq(7, 5) {
                let mut pieces: Vec<_> = p
                    .occupied()
                    .iter()
                    .map(|s| (s, p.piece_at(s).unwrap()))
                    .collect();
                pieces.push((sq(6, 5), piece(Color::Black, PieceKind::Pawn)));
                p = position_from_codes(Color::Black, &pieces);
            }
            round_trip(MoveRules::standard(), &p, mv(from, to, false));
        }
    }
}

/// 居喰い、曲がる2段階移動、および2枚捕獲を検査する。
#[test]
fn lion_igui_and_two_stage_captures() {
    // RULES.md第12条2・4・8項: 経由升で必ず捕獲し、到達升では0枚または1枚取る。
    let from = sq(5, 5);
    let mid = sq(6, 6);
    for to in [from, sq(5, 6), sq(7, 5)] {
        for double in [false, true] {
            if double && to == from {
                continue;
            }
            let mut captures = vec![mid];
            if double {
                captures.push(to);
            }
            let p = fixture(Color::Black, PieceKind::Lion, from, &captures);
            round_trip(
                MoveRules::standard(),
                &p,
                Move {
                    from,
                    mid: Some(mid),
                    to,
                    promote: false,
                },
            );
        }
    }
}

/// 角鷹と飛鷲の向き付き獅子力を先後両方で検査する。
#[test]
fn falcon_and_eagle_all_special_moves_for_both_sides() {
    // RULES.md第11条: 後手の前は南。飛鷲は左右両方の前斜めで2段階移動できる。
    for color in Color::ALL {
        let dr = if color == Color::Black { 1 } else { -1 };
        for kind in [PieceKind::HornedFalcon, PieceKind::SoaringEagle] {
            let directions: &[i8] = if kind == PieceKind::HornedFalcon {
                &[0]
            } else {
                &[-1, 1]
            };
            for &df in directions {
                let from = sq(5, if color == Color::Black { 4 } else { 7 });
                let first = from.offset(df, dr).unwrap();
                let second = first.offset(df, dr).unwrap();
                for (to, mid, captures) in [
                    (first, None, vec![]),
                    (first, None, vec![first]),
                    (second, None, vec![first]),
                    (second, None, vec![second]),
                    (from, None, vec![]),
                    (from, Some(first), vec![first]),
                    (second, Some(first), vec![first]),
                    (second, Some(first), vec![first, second]),
                ] {
                    let p = fixture(color, kind, from, &captures);
                    round_trip(
                        MoveRules::standard(),
                        &p,
                        Move {
                            from,
                            mid,
                            to,
                            promote: false,
                        },
                    );
                }
            }
        }
    }
}

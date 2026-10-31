//! 設計書「入力の利きの面」の駒種と本数を、利きの逆引きと規則の境界で照合する。

use super::*;
use crate::nn::{INPUT_PLANES, INPUT_SIZE, encode};
use minase_core::notation::sfen::parse_extended_sfen;
use minase_core::test_util::{position, sq};
use minase_core::{PieceCode, PieceKind, PositionBuilder, Rules, Square};

/// 面番号と回転は仕様の式から求め、符号化の補助関数は使わない。
fn value(input: &[f32], side: Color, color: Color, plane: usize, square: Square) -> f32 {
    let square = match side {
        Color::Black => square.dense_index(),
        Color::White => 143 - square.dense_index(),
    };
    input[(32 * usize::from(side != color) + plane) * 144 + square]
}

fn assert_matches_attackers(position: &Position) {
    let mut input = vec![7.0; CONTROL_SIZE];
    encode_control(position, &mut input);
    for color in Color::ALL {
        for target in Square::all() {
            let attackers = position.attackers_to_by(color, target, position.occupied());
            for kind in PieceKind::ALL {
                let expected = attackers
                    .into_iter()
                    .any(|from| position.piece_at(from).unwrap().kind() == Some(kind));
                assert_eq!(
                    value(&input, position.side_to_move(), color, kind.index(), target),
                    f32::from(expected),
                    "side={:?}, color={color:?}, kind={kind:?}, target={target:?}, attackers={attackers:?}",
                    position.side_to_move()
                );
            }
            let count = attackers.into_iter().count();
            for threshold in 1..=3 {
                assert_eq!(
                    value(
                        &input,
                        position.side_to_move(),
                        color,
                        29 + threshold - 1,
                        target
                    ),
                    f32::from(count >= threshold),
                    "color={color:?}, target={target:?}, threshold={threshold}, count={count}"
                );
            }
        }
    }
    let mut full = vec![7.0; INPUT_SIZE];
    encode(position, &mut full).unwrap();
    assert_eq!(&full[95 * 144..], input);
}

#[test]
fn all_fixture_controls_match_attackers_by_kind_and_count() {
    let fixtures = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/train/zero/fixtures/nn-positions.jsonl"
    ));
    assert_eq!(INPUT_PLANES, 159);
    assert_eq!(fixtures.lines().count(), 288);
    for line in fixtures.lines() {
        let fixture: serde_json::Value = serde_json::from_str(line).unwrap();
        let position = parse_extended_sfen(
            fixture["sfen"].as_str().unwrap(),
            Rules::ENGINE_DEFAULT.moves,
        )
        .unwrap()
        .restore_position()
        .unwrap();
        assert_matches_attackers(&position);
    }
}

#[test]
fn lion_jumps_over_own_pieces_and_controls_a_defended_lion() {
    // RULES第12条: 周囲2升へ跳べる。疑似利きでは第14条の獅子捕獲制限を判定しない。
    for side in Color::ALL {
        let mut pieces = vec![
            (sq(5, 5), Color::Black, PieceKind::Lion),
            (sq(7, 7), Color::White, PieceKind::Lion),
            (sq(7, 10), Color::White, PieceKind::Rook),
        ];
        for file in 4..=6 {
            for rank in 4..=6 {
                if (file, rank) != (5, 5) {
                    pieces.push((sq(file, rank), Color::Black, PieceKind::Pawn));
                }
            }
        }
        let position = position(side, &pieces);
        assert_matches_attackers(&position);
        let mut input = vec![0.0; CONTROL_SIZE];
        encode_control(&position, &mut input);
        for target in Square::all() {
            let expected = target != sq(5, 5)
                && target.file().abs_diff(5) <= 2
                && target.rank().abs_diff(5) <= 2;
            assert_eq!(
                value(&input, side, Color::Black, PieceKind::Lion.index(), target),
                f32::from(expected)
            );
        }
        // 多数の2歩の経路が同じ升へ届いても、獅子1枚の利きは1本である。
        assert_eq!(value(&input, side, Color::Black, 29, sq(7, 5)), 1.0);
        assert_eq!(value(&input, side, Color::Black, 30, sq(7, 5)), 0.0);
    }
}

#[test]
fn falcon_and_eagle_reach_two_steps_in_their_forward_directions() {
    // RULES第11条: 角鷹は前、飛鷲は前斜めへ2升まで跳び、隣接する味方駒も越える。
    for color in Color::ALL {
        for (kind, directions) in [
            (PieceKind::HornedFalcon, &[0][..]),
            (PieceKind::SoaringEagle, &[-1, 1][..]),
        ] {
            let orient = |file, rank| match color {
                Color::Black => sq(file, rank),
                Color::White => sq(11 - file, 11 - rank),
            };
            for side in Color::ALL {
                let mut builder = PositionBuilder::new(side);
                builder
                    .put(orient(5, 5), PieceCode::new_promoted(color, kind).unwrap())
                    .unwrap();
                for &dx in directions {
                    builder
                        .put(
                            orient((5 + dx) as u8, 6),
                            PieceCode::new(color, PieceKind::Pawn).unwrap(),
                        )
                        .unwrap();
                }
                let position = builder.finish().unwrap();
                assert_matches_attackers(&position);
                let mut input = vec![0.0; CONTROL_SIZE];
                encode_control(&position, &mut input);
                for &dx in directions {
                    for distance in 1..=3 {
                        let target = orient((5 + dx * distance) as u8, (5 + distance) as u8);
                        assert_eq!(
                            value(&input, side, color, kind.index(), target),
                            f32::from(distance <= 2)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn sliding_control_includes_the_first_piece_and_stops_there() {
    // 疑似利きは最初の駒の升を含み、その駒が味方でも相手でも先へ走らない。
    for side in Color::ALL {
        let position = position(
            side,
            &[
                (sq(5, 5), Color::Black, PieceKind::Rook),
                (sq(5, 7), Color::Black, PieceKind::Pawn),
                (sq(8, 5), Color::White, PieceKind::Pawn),
            ],
        );
        assert_matches_attackers(&position);
        let mut input = vec![0.0; CONTROL_SIZE];
        encode_control(&position, &mut input);
        for (target, expected) in [
            (sq(5, 6), 1.0),
            (sq(5, 7), 1.0),
            (sq(5, 8), 0.0),
            (sq(7, 5), 1.0),
            (sq(8, 5), 1.0),
            (sq(9, 5), 0.0),
        ] {
            assert_eq!(
                value(&input, side, Color::Black, PieceKind::Rook.index(), target),
                expected
            );
        }
    }
}

#[test]
fn promoted_pieces_use_their_current_kind() {
    // 成った歩兵は金将の面に置き、歩兵の面には置かない。相対陣営と回転も両手番で検査する。
    for color in Color::ALL {
        for side in Color::ALL {
            let mut builder = PositionBuilder::new(side);
            builder
                .put(
                    sq(3, 4),
                    PieceCode::new_promoted(color, PieceKind::GoldGeneral).unwrap(),
                )
                .unwrap();
            let position = builder.finish().unwrap();
            assert_matches_attackers(&position);
            let mut input = vec![0.0; CONTROL_SIZE];
            encode_control(&position, &mut input);
            let target = sq(4, 4);
            assert_eq!(
                value(&input, side, color, PieceKind::GoldGeneral.index(), target),
                1.0
            );
            assert_eq!(
                value(&input, side, color, PieceKind::Pawn.index(), target),
                0.0
            );
        }
    }
}

#[test]
fn thresholds_count_pieces_even_when_their_kinds_are_equal() {
    // (5,5)へ届く駒を0〜4枚置く。飛車2枚は駒種の面を共有するが、利き数では2枚と数える。
    let pieces = [
        (sq(5, 0), Color::Black, PieceKind::Rook),
        (sq(5, 10), Color::Black, PieceKind::Rook),
        (sq(3, 5), Color::Black, PieceKind::Lion),
        (sq(6, 5), Color::Black, PieceKind::GoldGeneral),
    ];
    for side in Color::ALL {
        let mut input = vec![7.0; CONTROL_SIZE];
        for count in (0..=4).rev() {
            let position = position(side, &pieces[..count]);
            assert_matches_attackers(&position);
            encode_control(&position, &mut input);
            for threshold in 1..=3 {
                assert_eq!(
                    value(&input, side, Color::Black, 29 + threshold - 1, sq(5, 5)),
                    f32::from(count >= threshold)
                );
            }
            if count == 0 {
                assert!(input.iter().all(|&value| value == 0.0));
            }
        }
    }
}

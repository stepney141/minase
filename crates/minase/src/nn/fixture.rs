//! Pythonの学習器の符号化と照合する固定局面集の参照値の書き出し。
//!
//! 各局面について、拡張SFEN、規則セット、入力平面の非零要素の（面, 相対升）、
//! および全合法手の（USI表記, 方策ラベル）をJSONで書く（docs/plans/alphazero.md「検証」）。

use std::io::{self, Write};

use minase_core::notation::sfen::parse_extended_sfen;
use minase_core::notation::usi::text_generated;
use minase_core::rules::parse_rule_set;
use minase_core::{BOARD_SQUARE_COUNT, MoveGenerator, Rules};
use serde_json::json;

use super::INPUT_PLANES;
use super::encoding::encode_into;
use super::policy::label;

/// （拡張SFEN, 規則セット）の局面集の参照値をJSONで`out`へ書く。
///
/// 局面集は試験が用意する固定値なので、解析できない入力ではパニックする。
pub(super) fn write_fixture(positions: &[(&str, &str)], out: impl Write) -> io::Result<()> {
    let mut entries = Vec::new();
    for &(sfen, rule_set) in positions {
        let rules = Rules::from_codes(&parse_rule_set(rule_set).expect("fixture rule set"))
            .expect("fixture rules");
        let position = parse_extended_sfen(sfen, rules.moves)
            .expect("fixture SFEN")
            .restore_position()
            .expect("fixture position");

        let mut planes = vec![0.0; INPUT_PLANES * BOARD_SQUARE_COUNT];
        encode_into(&position, &mut planes);
        let nonzero: Vec<_> = planes
            .iter()
            .enumerate()
            .filter(|&(_, &value)| value != 0.0)
            .map(|(index, _)| [index / BOARD_SQUARE_COUNT, index % BOARD_SQUARE_COUNT])
            .collect();

        let mut moves = Vec::new();
        MoveGenerator::new(rules.moves).generate_moves(&position, &mut moves);
        let mut labeled: Vec<_> = moves
            .iter()
            .map(|&mv| {
                let value = label(mv, position.side_to_move()).expect("legal move has a label");
                (text_generated(&position, mv), value)
            })
            .collect();
        labeled.sort_by_key(|&(_, value)| value);

        entries.push(json!({
            "sfen": sfen,
            "rules": rule_set,
            "planes": nonzero,
            "moves": labeled,
        }));
    }
    let mut out = out;
    serde_json::to_writer_pretty(&mut out, &json!({ "positions": entries }))?;
    writeln!(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use minase_core::notation::sfen::{SetupPosition, to_extended_sfen};
    use minase_core::test_util::{BENCH_SFENS, sq};
    use minase_core::{Color, Move, PieceCode, PieceKind, Position, PositionBuilder, Square};

    /// 成り権の保留を追跡する固定局面の規則セット。
    const DEFERRED_RULE_SET: &str = "L0,P2,R1,E0";

    /// 駒の配置と先獅子の升から拡張SFENを作る。
    fn extended_sfen(
        side_to_move: Color,
        pieces: &[(Square, Color, PieceKind)],
        lion_capture: Option<Square>,
    ) -> String {
        let mut builder = PositionBuilder::new(side_to_move);
        for &(square, color, kind) in pieces {
            builder
                .put(square, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        let position: Position = builder.finish().unwrap();
        to_extended_sfen(&SetupPosition::new(position, lion_capture, 1).unwrap())
    }

    /// 規則セット[`DEFERRED_RULE_SET`]で敵陣へ不成で入った直後の局面の拡張SFENを返す。
    ///
    /// 先手の歩兵が不成で敵陣へ入った直後の後手番の局面と、続けて後手の銀将が不成で敵陣へ入った
    /// 直後の先手番の局面からなる。
    fn deferred_sfens() -> Vec<String> {
        let rules = Rules::from_codes(&parse_rule_set(DEFERRED_RULE_SET).unwrap())
            .unwrap()
            .moves;
        let mut builder = PositionBuilder::new(Color::Black);
        let pieces = [
            (sq(0, 0), Color::Black, PieceKind::King),
            (sq(11, 11), Color::White, PieceKind::King),
            (sq(5, 7), Color::Black, PieceKind::Pawn),
            (sq(6, 5), Color::Black, PieceKind::Lion),
            (sq(3, 9), Color::White, PieceKind::GoldGeneral),
            (sq(8, 4), Color::White, PieceKind::SilverGeneral),
        ];
        for (square, color, kind) in pieces {
            builder
                .put(square, PieceCode::new(color, kind).unwrap())
                .unwrap();
        }
        let mut position = builder.finish().unwrap();
        let mut sfens = Vec::new();
        for (from, to) in [(sq(5, 7), sq(5, 8)), (sq(8, 4), sq(8, 3))] {
            let mut moves = Vec::new();
            MoveGenerator::new(rules).generate_moves(&position, &mut moves);
            let mv = Move {
                from,
                mid: None,
                to,
                promote: false,
            };
            assert!(moves.contains(&mv), "non-promotion into the zone is legal");
            position.make_move_unchecked(mv, rules);
            assert!(position.promotion_deferred().contains(to));
            sfens.push(to_extended_sfen(
                &SetupPosition::new(position.clone(), None, 1).unwrap(),
            ));
        }
        sfens
    }

    /// 固定局面集の（拡張SFEN, 規則セット）を返す。
    ///
    /// bench局面（初期局面と後手番の局面を含む）、先獅子状態のある先手番と後手番の局面、
    /// 獅子の2段階移動・居喰い・じっとが合法な先手番と後手番の局面、
    /// および成り権を保留した駒を持つ後手番と先手番の局面からなる。
    fn fixture_positions() -> Vec<(String, &'static str)> {
        let mut sfens: Vec<_> = BENCH_SFENS
            .iter()
            .map(|sfen| format!("{sfen} - 1"))
            .collect();
        sfens.push(
            "l1c1gekgsc1l/a1bst2t1b1a/1vr1dq1dhrv1/mf1p1poxp1fm/pppi2ppippp/4pn6/1Ph9/\
             P1PIPPPPIPPP/MF1PX1C1P3/2RHD1Q1HRVM/ACB1TODT1B1A/LV1SGKEGS1FL b 10g 4 -"
                .to_owned(),
        );
        sfens.push(extended_sfen(
            Color::White,
            &[
                (sq(0, 0), Color::Black, PieceKind::King),
                (sq(11, 11), Color::White, PieceKind::King),
                (sq(4, 5), Color::Black, PieceKind::GoldGeneral),
                (sq(8, 9), Color::White, PieceKind::Lion),
                (sq(3, 5), Color::Black, PieceKind::Lion),
            ],
            Some(sq(4, 5)),
        ));
        for side_to_move in Color::ALL {
            sfens.push(extended_sfen(
                side_to_move,
                &[
                    (sq(0, 0), Color::Black, PieceKind::King),
                    (sq(11, 11), Color::White, PieceKind::King),
                    (sq(5, 5), Color::Black, PieceKind::Lion),
                    (sq(5, 6), Color::White, PieceKind::GoldGeneral),
                    (sq(6, 7), Color::White, PieceKind::Lion),
                    (sq(7, 6), Color::Black, PieceKind::Pawn),
                ],
                None,
            ));
        }
        let mut positions: Vec<_> = sfens
            .into_iter()
            .map(|sfen| (sfen, "engine-default"))
            .collect();
        positions.extend(
            deferred_sfens()
                .into_iter()
                .map(|sfen| (sfen, DEFERRED_RULE_SET)),
        );
        positions
    }

    /// 固定局面集を[`write_fixture`]の引数の形へ借用する。
    fn borrowed<'a>(positions: &'a [(String, &'static str)]) -> Vec<(&'a str, &'static str)> {
        positions
            .iter()
            .map(|(sfen, rule_set)| (sfen.as_str(), *rule_set))
            .collect()
    }

    /// 固定局面集の参照値を書き出せること、および各局面が後手番・先獅子・獅子の特殊な着手・
    /// 成り権保留を含むことを検査する。
    #[test]
    fn fixture_covers_the_required_positions() {
        let positions = fixture_positions();
        let mut buffer = Vec::new();
        write_fixture(&borrowed(&positions), &mut buffer).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&buffer).unwrap();
        let entries = value["positions"].as_array().unwrap();
        assert_eq!(entries.len(), positions.len());

        let has_label_plane = |plane: u64| {
            entries.iter().any(|entry| {
                entry["moves"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|mv| mv[1].as_u64().unwrap() % 249 == plane)
            })
        };
        assert!(has_label_plane(248), "no jitto");
        assert!(has_label_plane(240), "no igui");
        assert!((184..240).any(has_label_plane), "no two-step capture");
        assert!((176..184).any(has_label_plane), "no knight jump");
        assert!(positions.iter().any(|(sfen, _)| sfen.contains(" w ")));
        let has_input_plane = |plane: u64| {
            entries.iter().any(|entry| {
                entry["planes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|pair| pair[0].as_u64() == Some(plane))
            })
        };
        assert!(has_input_plane(94), "no lion-capture square");
        assert!(has_input_plane(95), "no promotion-deferred square");
    }

    /// 固定局面集の参照値を`crates/minase/tests/fixtures/nn_encoding.json`へ書き出す。
    #[test]
    #[ignore = "writes the fixture file consumed by the Python encoder test"]
    fn write_nn_encoding_fixture() {
        let positions = fixture_positions();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/nn_encoding.json"
        );
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        write_fixture(&borrowed(&positions), file).unwrap();
    }
}

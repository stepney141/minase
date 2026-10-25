//! 基準MNPTから探索用の固定値を読み、駒価値からNNUEの線形初期値を作る。

use minase::eval::{
    features::{BOARD_FEATURE_COUNT, FEATURE_COUNT, PIECE_STATE_COUNT},
    pst::Pst,
};
use minase_core::PieceKind;
use std::{error::Error, path::Path};

/// 線形初期値と書き出しに使う、基準MNPTの固定値。
#[derive(Debug)]
pub struct Base {
    /// 駒状態番号順の探索用駒価値。単位はセンチポーン。
    pub piece_values: [i32; PIECE_STATE_COUNT],
    /// 勝率尺度（1ロジットあたりのcp）。
    pub cp_per_logit: f32,
    /// 元のMNPTが指定する規則セット名。
    pub rule_set: String,
}

impl Base {
    /// 非王駒の自駒行に探索用駒価値、相手駒行にその負値をセンチポーンで置く。
    /// 各駒状態の144升と序中盤・終盤の2端点は同じ値とし、王駒と先獅子の行は0とする。
    pub fn initial_linear(&self) -> Vec<[f32; 2]> {
        let mut initial = vec![[0.0; 2]; FEATURE_COUNT];
        for (feature, endpoints) in initial[..BOARD_FEATURE_COUNT].iter_mut().enumerate() {
            let state = (feature / 144) % PIECE_STATE_COUNT;
            if state == PieceKind::King.index() || state == PieceKind::CrownPrince.index() {
                continue;
            }
            let value = self.piece_values[state] as f32;
            let side = feature / (PIECE_STATE_COUNT * 144);
            *endpoints = [if side == 0 { value } else { -value }; 2];
        }
        initial
    }
}

/// MNPT全体をminaseの読み手で検証し、NNUEで引き継ぐ値を返す。
pub fn read(path: &Path) -> Result<Base, Box<dyn Error>> {
    decode(&std::fs::read(path)?)
}

fn decode(bytes: &[u8]) -> Result<Base, Box<dyn Error>> {
    let pst = Pst::decode(bytes)?;
    Ok(Base {
        piece_values: std::array::from_fn(|state| pst.piece_value_of_state(state)),
        cp_per_logit: pst.cp_per_logit(),
        rule_set: pst.rule_set().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::{
        data::{BatchSlot, InputFeatureSet, Observations},
        model::{HeadKind, Masks, Model, Shape},
        reference::{self, v0_piece_values},
    };
    use minase::eval::{
        features::mirror_feature,
        pst::{FM_RANK, FmWeights},
    };
    use minase_core::{Color, PieceCode, Position, PositionBuilder, Square};

    /// MNPT版3の配置で作る。
    /// 両端点、規則名、勝率尺度は基準の埋め込み値と異なる既知の値を使う。
    fn fixture() -> Vec<u8> {
        Pst::from_weights_with_fm(
            &vec![[123, -456]; FEATURE_COUNT],
            v0_piece_values(),
            317.25,
            "L1,P0,R1,E0",
            FmWeights {
                embeddings: &vec![[0; FM_RANK]; FEATURE_COUNT],
                signs: &[1; FM_RANK],
                exponent: 7,
            },
        )
        .unwrap()
        .encode()
    }

    #[test]
    fn mnpt_base_preserves_fixed_metadata() {
        let base = decode(&fixture()).unwrap();
        assert_eq!(base.piece_values, v0_piece_values());
        assert_eq!(base.cp_per_logit.to_bits(), 317.25_f32.to_bits());
        assert_eq!(base.rule_set, "L1,P0,R1,E0");
    }

    #[test]
    fn material_initialization_assigns_signed_values_and_zeroes_royals_and_lion_features() {
        // 設計書の初期値仕様: 47駒状態×144升を各陣営に並べ、先獅子の144行を続ける。
        let base = decode(&fixture()).unwrap();
        let initial = base.initial_linear();
        assert_eq!(initial.len(), FEATURE_COUNT);
        for (side, rows) in initial[..BOARD_FEATURE_COUNT]
            .as_chunks::<{ PIECE_STATE_COUNT * 144 }>()
            .0
            .iter()
            .enumerate()
        {
            for (state, (squares, value)) in rows
                .as_chunks::<144>()
                .0
                .iter()
                .zip(v0_piece_values())
                .enumerate()
            {
                let expected =
                    if [PieceKind::King.index(), PieceKind::CrownPrince.index()].contains(&state) {
                        [0.0; 2]
                    } else if side == 0 {
                        [value as f32; 2]
                    } else {
                        [-(value as f32); 2]
                    };
                for (square, &endpoints) in squares.iter().enumerate() {
                    assert_eq!(
                        endpoints, expected,
                        "side={side}, state={state}, square={square}"
                    );
                }
            }
        }
        assert_eq!(&initial[BOARD_FEATURE_COUNT..], &[[0.0; 2]; 144]);
        for (feature, &endpoints) in initial.iter().enumerate() {
            assert_eq!(
                endpoints,
                initial[mirror_feature(feature)],
                "feature={feature}"
            );
        }
    }

    #[test]
    fn initialized_linear_forward_matches_non_royal_material_balance() {
        fn position(side: Color, pieces: &[(Square, PieceCode)]) -> Position {
            let mut builder = PositionBuilder::new(side);
            for &(square, piece) in pieces {
                builder.put(square, piece).unwrap();
            }
            builder.finish().unwrap()
        }

        let base = decode(&fixture()).unwrap();
        let initial_position = Position::initial();
        let mut reduced: Vec<_> = initial_position
            .occupied()
            .into_iter()
            .map(|square| (square, initial_position.piece_at(square).unwrap()))
            .collect();
        for (color, kind) in [
            (Color::Black, PieceKind::Pawn),
            (Color::Black, PieceKind::Rook),
            (Color::White, PieceKind::Lion),
        ] {
            let piece = PieceCode::new(color, kind).unwrap();
            let index = reduced.iter().position(|&(_, code)| code == piece).unwrap();
            reduced.remove(index);
        }
        // v0表では歩兵100、飛車750、獅子2500。初期局面からの除去で先手に2500−750−100=1650。
        let mut cases = vec![
            ("initial", initial_position, 0.0),
            ("removed, black", position(Color::Black, &reduced), 1650.0),
            ("removed, white", position(Color::White, &reduced), -1650.0),
        ];
        let promoted: Vec<_> = [
            (4, 0, Color::Black, PieceKind::King, false),
            (7, 11, Color::White, PieceKind::King, false),
            (3, 0, Color::Black, PieceKind::CrownPrince, true),
            (2, 4, Color::Black, PieceKind::GoldGeneral, true),
            (9, 3, Color::Black, PieceKind::FlyingOx, true),
            (8, 7, Color::White, PieceKind::Rook, false),
            (6, 9, Color::White, PieceKind::DrunkElephant, false),
        ]
        .into_iter()
        .map(|(file, rank, color, kind, promoted)| {
            let piece = if promoted {
                PieceCode::new_promoted(color, kind)
            } else {
                PieceCode::new(color, kind)
            };
            (Square::new(file, rank).unwrap(), piece.unwrap())
        })
        .collect();
        // 成った金将378と飛牛1000、相手の飛車750と醉象503で378+1000−750−503=125。
        // 先手だけが持つ太子は0。先獅子の有無でも和は変わらない。
        for (side, expected, target) in [
            (Color::Black, 125.0, Square::new(8, 7).unwrap()),
            (Color::White, -125.0, Square::new(2, 4).unwrap()),
        ] {
            let mut promoted_position = position(side, &promoted);
            cases.push(("promoted", promoted_position.clone(), expected));
            promoted_position.set_lion_capture(Some(target)).unwrap();
            cases.push(("lion capture", promoted_position, expected));
        }

        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let observations = Observations::from_positions(
                cases.iter().map(|(_, position, _)| Ok(position.clone())),
                set,
            )
            .unwrap();
            let model = Model::initialize(
                Shape::new(set, 2, base.cp_per_logit, HeadKind::DirectScrelu).unwrap(),
                &base.initial_linear(),
                Masks::from_observations(set, &observations).unwrap(),
                1,
                0.05,
            )
            .unwrap();
            let weights = reference::fold(&model).unwrap();
            for (case, position, expected) in &cases {
                let batch = BatchSlot::from_evaluation_positions(
                    set,
                    std::slice::from_ref(position),
                    position.side_to_move(),
                )
                .unwrap();
                assert_eq!(
                    reference::forward(&model, &batch, &weights).unwrap().lin,
                    [[*expected; 2]],
                    "{set:?}, {case}, {:?}",
                    position.side_to_move()
                );
            }
        }
    }
}

//! 基準MNUEから探索用の固定値を読み、駒価値からNNUEの線形初期値を作る。

use minase::eval::{
    Nnue,
    features::{BOARD_FEATURE_COUNT, FEATURE_COUNT, PIECE_STATE_COUNT},
};
use minase_core::PieceKind;
use std::{error::Error, path::Path};

/// 線形初期値と書き出しに使う、基準MNUEの固定値。
#[derive(Debug)]
pub struct Base {
    /// 駒状態番号順の探索用駒価値。単位はセンチポーン。
    pub piece_values: [i32; PIECE_STATE_COUNT],
    /// 勝率尺度（1ロジットあたりのcp）。
    pub cp_per_logit: f32,
    /// 元のMNUEが指定する規則セット名。
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

/// MNUE全体をminaseの読み手で検証し、NNUEで引き継ぐ値を返す。
pub fn read(path: &Path) -> Result<Base, Box<dyn Error>> {
    decode(&std::fs::read(path)?)
}

fn decode(bytes: &[u8]) -> Result<Base, Box<dyn Error>> {
    let net = Nnue::decode(bytes)?;
    Ok(Base {
        piece_values: std::array::from_fn(|state| net.piece_value_of_state(state)),
        cp_per_logit: net.cp_per_logit(),
        rule_set: net.rule_set().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nnue::{
        data::{BatchSlot, InputFeatureSet, Observations},
        model::{HeadKind, Masks, Model, Shape},
        reference,
    };
    use minase::eval::{
        features::{mirror_feature, piece_state},
        nnue::{
            EncodeInput, encode,
            feature_set::P,
            head::{Head, PairwiseL16L32},
        },
    };
    use minase_core::{Color, PieceCode, Position, PositionBuilder, Square};

    fn piece_values() -> [i32; PIECE_STATE_COUNT] {
        let mut values = std::array::from_fn(|state| 40 + state as i32 * 3);
        let royal = values.iter().copied().max().unwrap()
            + values[piece_state(PieceCode::new(Color::Black, PieceKind::Pawn).unwrap())];
        values[PieceKind::King.index()] = royal;
        values[PieceKind::CrownPrince.index()] = royal;
        values
    }

    /// MNUE版5を、学習済みの埋め込み値に依存しない重みとメタデータから作る。
    fn fixture() -> Vec<u8> {
        let head = PairwiseL16L32::decode(&vec![0; PairwiseL16L32::byte_len(4)], 4, true).unwrap();
        encode::<P, PairwiseL16L32>(&EncodeInput {
            width: 4,
            cp_per_logit: 317.25,
            rule_set: "L1,P0,R1,E0",
            piece_values: &piece_values(),
            linear: &vec![[123, -456]; FEATURE_COUNT],
            weights: &vec![0; FEATURE_COUNT * 4],
            biases: &[0; 4],
            head: &head,
        })
        .unwrap()
    }

    #[test]
    fn mnue_base_preserves_fixed_metadata() {
        let base = decode(&fixture()).unwrap();
        assert_eq!(base.piece_values, piece_values());
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
                .zip(piece_values())
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
        let value = |kind, promoted| {
            let piece = if promoted {
                PieceCode::new_promoted(Color::Black, kind)
            } else {
                PieceCode::new(Color::Black, kind)
            }
            .unwrap();
            base.piece_values[piece_state(piece)] as f32
        };
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
        // 初期局面から自分の歩兵と飛車、相手の獅子を除いた駒割の差を使う。
        let removed_score = value(PieceKind::Lion, false)
            - value(PieceKind::Rook, false)
            - value(PieceKind::Pawn, false);
        let mut cases = vec![
            ("initial", initial_position, 0.0),
            (
                "removed, black",
                position(Color::Black, &reduced),
                removed_score,
            ),
            (
                "removed, white",
                position(Color::White, &reduced),
                -removed_score,
            ),
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
        // 成った金将と飛牛から相手の飛車と醉象を引く。太子と先獅子の寄与は0。
        let promoted_score = value(PieceKind::GoldGeneral, true) + value(PieceKind::FlyingOx, true)
            - value(PieceKind::Rook, false)
            - value(PieceKind::DrunkElephant, false);
        for (side, expected, target) in [
            (Color::Black, promoted_score, Square::new(8, 7).unwrap()),
            (Color::White, -promoted_score, Square::new(2, 4).unwrap()),
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
                Shape::new(set, 4, base.cp_per_logit, HeadKind::PairwiseL16L32).unwrap(),
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

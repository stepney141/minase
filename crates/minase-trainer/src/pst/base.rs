//! 基準MNPTから初期値の照合用PSTと書き出し用の固定値を読む。

use super::Result;
use minase::eval::{features::PIECE_STATE_COUNT, pst::Pst};
use std::path::Path;

/// 初期MNPTが保持するPSTと探索用の固定値。
#[derive(Debug)]
pub struct Base {
    /// 特徴番号順の序中盤・終盤重み。単位は1/8センチポーン。
    pub linear: Vec<[i16; 2]>,
    /// 駒状態番号順の探索用駒価値。単位はセンチポーン。
    pub piece_values: [i32; PIECE_STATE_COUNT],
    /// 勝率尺度（1ロジットあたりのcp）。
    pub cp_per_logit: f32,
    /// 元のMNPTが指定する規則セット名。
    pub rule_set: String,
}

/// MNPT全体をminaseの読み手で検証し、学習したPSTへ引き継ぐ値を返す。
pub fn read(path: &Path) -> Result<Base> {
    decode(&std::fs::read(path)?)
}

fn decode(bytes: &[u8]) -> Result<Base> {
    let pst = Pst::decode(bytes)?;
    Ok(Base {
        linear: pst.weights().to_vec(),
        piece_values: std::array::from_fn(|state| pst.piece_value_of_state(state)),
        cp_per_logit: pst.cp_per_logit(),
        rule_set: pst.rule_set().to_owned(),
    })
}

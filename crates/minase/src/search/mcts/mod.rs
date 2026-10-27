//! モンテカルロ木探索の駆動部、対局用エンジン、および選択規則。
//!
//! 駆動部は[`tree::Tree::select_leaf`]の経路で局面を再生し、合法手と終局の判定を
//! [`tree::Tree::create_child`]へ渡す。評価待ちの葉は別々の木から集めてもよく、
//! 推論の結果を[`tree::Tree::evaluate`]へ返すと、価値と確定した勝敗が根まで伝わる。
//! 既知の葉は[`tree::Tree::backup_leaf`]で処理し、推論や容量確保を中止した選択は
//! [`tree::Tree::cancel_leaf`]で取り消す。根の初回評価には空の経路を使う。
//!
//! [`Driver::select_leaf`]が返す[`EvaluationRequest`]は局面と合法手を所有する。
//! 複数局の依頼を[`EvaluationRequest::query`]で借用してまとめて推論し、
//! 各結果を元の駆動部の[`Driver::evaluate`]へ返せる。
//! 自己対局では[`DrawValue::SelfPlay`]と[`tree::SelectionRule::Gumbel`]を使う。
//! [`Engine`]は対局用PUCTを実行し、深さを主変化の長さ、ノード数を根の評価を含む
//! シミュレーション数として、終了時の[`crate::search::SearchEvent::Finished`]だけを送る。

pub mod driver;
mod engine;
mod error;
pub mod gumbel;
pub mod puct;
pub mod tree;

pub use driver::{DrawValue, Driver, EvaluationRequest, Step};
pub use engine::{Config, Engine};
pub use error::Error;

/// 最大値を引いてから指数を取る、単精度のsoftmax。
fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut probabilities: Vec<_> = logits.iter().map(|logit| (logit - max).exp()).collect();
    let sum: f32 = probabilities.iter().sum();
    for probability in &mut probabilities {
        *probability /= sum;
    }
    probabilities
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

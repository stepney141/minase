//! 局面を持たないモンテカルロ木探索の木と選択規則。
//!
//! 駆動部は[`tree::Tree::select_leaf`]の経路で局面を再生し、合法手と終局の判定を
//! [`tree::Tree::create_child`]へ渡す。評価待ちの葉は別々の木から集めてもよく、
//! 推論の結果を[`tree::Tree::evaluate`]へ返すと、価値と確定した勝敗が根まで伝わる。
//! 既知の葉は[`tree::Tree::backup_leaf`]で処理し、推論や容量確保を中止した選択は
//! [`tree::Tree::cancel_leaf`]で取り消す。根の初回評価には空の経路を使う。

pub mod gumbel;
pub mod puct;
pub mod tree;

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
mod tests;

//! AlphaZero型探索のネットが使う入力の符号化、方策のラベル、および推論。
//!
//! 入力と方策はどちらも手番側から見た盤で表す。後手番では盤を180度回転させ、
//! 升(筋, 段)を(11 − 筋, 11 − 段)へ写す（docs/plans/alphazero.md「入力とネットの構成」）。
//! 入力は先獅子と成り権保留の状態を面として持つので、どの規則セットの局面も符号化できる。

pub mod encoding;
pub mod evaluator;
pub mod mnzt;
pub mod ort_evaluator;
pub mod policy;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod test_support;

/// 入力平面の数。手番側の47状態、相手側の47状態、先獅子の升の1面、および成り権保留の升の1面からなる。
pub const INPUT_PLANES: usize = 96;

/// 方策の移動元の升ごとの面の数。
pub const POLICY_PLANES: usize = 249;

/// 方策の出力の要素数（移動元の升144個×249面）。
pub const POLICY_SIZE: usize = 144 * POLICY_PLANES;

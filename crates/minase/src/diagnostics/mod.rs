//! 対局状態を変更せずに実行する診断。出力先へ接頭辞のない行を渡す。

pub mod bench;
pub mod nn_eval;
pub mod nn_fixture;
pub mod perft;
pub mod random_play;

#[cfg(test)]
pub(crate) mod test_support;

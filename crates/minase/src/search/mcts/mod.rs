//! ネットの方策と価値で有望な手を選んで読むMCTS（Monte Carlo tree search）。
//!
//! 探索木（`tree`）、子の選択規則（`select`のPUCTとGumbel）、1回のシミュレーションと
//! バッチ評価（`simulate`）、探索の開始と停止（`engine`）、自己対局用に推論を呼び出し側へ任せる
//! Gumbel探索（`stepped`）、およびUSIオプションと推論のセッションの作成（`options`）からなる
//! （docs/plans/alphazero.md「共存の構造」）。
//! 勝率はすべて[0, 1]の値で扱い、辺の価値は親の手番側から見た勝率である。

mod engine;
mod options;
mod select;
mod simulate;
mod stepped;
#[cfg(test)]
mod tests;
mod tree;

pub use engine::{Config, DEFAULT_GUMBEL_SIMULATIONS, Engine, MAX_CONSIDERED_ACTIONS, MctsError};
pub use options::default_cache_dir;
pub use select::Selection;
pub use stepped::{SearchedMove, Step, SteppedGumbelSearch};

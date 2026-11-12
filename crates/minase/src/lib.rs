//! 中将棋の対局エンジンMinase。
//!
//! 盤、駒、合法手、裁定、および表記は[`minase_core`]が提供する。
//! 探索と評価の依存は[`search`]から[`eval`]、さらに`minase_core`への一方向である。
//! [`nn`]はモンテカルロ木探索（MCTS）のネットの入出力として、入力平面の符号化、方策のラベル、および評価器の抽象を持つ。
//! `nn`は`minase_core`と[`eval::features`]の駒の状態番号だけに依存し、`search`から参照されることはあっても`search`を参照しない。
//! [`training`]はMNSDの交換形式を持ち、プロジェクト内では`minase_core`だけに依存する。
//! `search`と`eval`は`training`を参照しない。
//! `datagen`はデータ生成と棋譜取り込みの共有処理を持ち、`training`と`minase_core`に依存し、探索と評価には依存しない。
//!
//! 探索として再利用できるコードは、実行ファイルのソースではなく`search`の下に置く。
//! たとえば自己対局生成器が呼ぶ王手限定の3手以内の勝ちの判定は[`search::mate_in_three`]にある。
//! `src/bin/minase/`は、起動引数、入出力、および開発者向けのサブコマンドの層である。
//! 試験専用のコードは本番の型と関数の中に置かず、試験用の子モジュールへ分ける。
//! 別の実行ファイルの試験から使う補助は、`test-util`フィーチャの`nn::test_util`のように公開する。
//! 理由は`docs/adr/keep-test-only-code-out-of-production-types-and-functions.md`にある。
//!
//! `harness`、`datagen`、`stats`は実行ファイル向けの内部層として`#[doc(hidden)]`で公開する。
//! 規則ライブラリの型は`minase_core::`から参照し、本crateから再公開しない。
//! 理由は`docs/adr/do-not-re-export-moved-items-at-old-paths.md`にある。

#[doc(hidden)]
pub mod datagen;
pub mod diagnostics;
pub mod eval;
#[doc(hidden)]
pub mod harness;
pub mod nn;
pub mod protocol;
pub mod search;
#[doc(hidden)]
pub mod stats;
pub mod training;

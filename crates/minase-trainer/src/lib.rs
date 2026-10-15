//! minaseの評価関数ごとの学習器と診断。
//! `nnue`は[`nnue`]、`pst train`は[`pst`]、`fm train/quarter/scale`は[`fm`]、`joint train`は[`joint`]、`diag`は[`diagnostics`]に対応する。
//! 特徴とバッチ、モデル、前進と後退、駒除去差分、チェックポイント、学習ループは各評価関数のモジュールが持つ。
//! 同時学習はPSTとFMの部品を組み合わせ、NNUEの部品は使わない。
//! 全学習器で共有するのは、MNSD、検証分割、先読み教師、教師用K、教師値を扱う[`data`]と、Adam、二値交差エントロピー、固定順の並列和、NPZ、早期終了、原子的書き込みを扱う[`nn`]である。
//! 理由は`docs/adr/split-training-pipelines-by-evaluation-function.md`にある。
//!
//! PST、FM、同時学習はCPUだけで動き、`--device`を持たず、記録のdeviceは常に`"cpu"`である。
//! 重み、勾配、AdamWの状態はf32で持ち、局面方向とバッチ方向の和や損失の集計はf64で局面順を固定する。
//! `--threads`は結果のビット列を変えず、省略時は物理コア数−2、最小1とする。
//! 理由は`docs/adr/train-pst-and-fm-on-the-cpu.md`にある。

pub mod data;
pub mod diagnostics;
pub mod error;
pub mod fm;
pub mod joint;
pub mod nn;
pub mod nnue;
pub mod pst;

pub mod identity;

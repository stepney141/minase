# Rust製NNUE学習器の検査と試走の記録

## 目的

[Rust製NNUE学習器（GPU）の設計書](../plans/nnue-trainer-rust.md)の各フェーズの完了条件に対する検査結果と、学習の試走の速度を記録する。
棋力測定ではないので、ペンタノミアル度数やLLRは持たない。

## 環境

学習機はRTX 3060 Ti（8 GB、CUDA 13.4、ドライバ615.71）、CPUはCore Ultra 7 265KF（20コア）、メモリ30 GB、Rust 1.98.0、cudarc 0.19.10である。
検査の間、同じ機械で教師データの生成（同時12対局）とlishogiのBotが動いていた。
Pythonの照合は`tools/train/.venv`（torch 2.13.0、numpy）で行った。

## フェーズ0　`minase`の公開APIとMNUE版5の符号化

| 検査 | 方法 | 結果 |
|---|---|---|
| 符号化と復号の往復 | 埋め込みの`nets/nnue.bin`を復号して符号化し、元のバイト列と比較。ランダム重みのHalfKP-hm-b8でも往復（`cargo test`） | バイト単位で一致 |
| 符号化の拒否 | 範囲外の値、鏡映の共有違反、奇数の幅、K=0、不正な配列長、規則セット名の長さ超過（`cargo test`、13件） | すべて拒否 |
| 推論への影響 | `minase dev bench --depth 9`の15局面のノード数、最善手、評価値を変更前と比較 | 一致（総ノード数5,229,890） |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（898件） | 通過 |

## フェーズ1a　データ経路（前処理とバッチ構築）

| 検査 | 方法 | 結果 |
|---|---|---|
| 検証分割と先読み教師と教師用K | Pythonの関数の値を写した単体テスト（`hash64`のベクトル、先読みの例、Kの小例） | 一致 |
| 特徴番号の照合（実データ） | `dump-batch`で世代3の1ファイル（`data/gen3/generated-1500000.bin`）の12局面（先頭3局面、末尾2局面、途中7局面）をP型とHalfKP-hm-b8でバッチにし、`tools/train/scripts/crosscheck_batch.py`で`minase_train.nnue.feature_sets.indices`と両視点145列および線形145列を比較 | 両方の型で全列一致 |
| 逆索引の性質 | 要素数、固定順、語の往復、番兵の除外（`cargo test`、統合テスト22件） | 通過 |
| 1ファイルの読み込みと先読みの前処理 | `dump-batch`の経過時間と最大常駐メモリ（`/usr/bin/time`） | 約1.5秒、約569 MB |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer` | 通過 |

## フェーズ1b　モデルとCPU参照実装

（実装後に書く。）

## フェーズ2　GPUカーネルと等価性テスト

（実装後に書く。）

## フェーズ3　学習ループと照合、1ファイルの試走

（実装後に書く。Pythonの試走との比較は[学習の記録](nnue-replacement-training.md)の値を使う。）

## フェーズ4　全データの試走

（実装後に書く。）

## 結論

（完了後に書く。）

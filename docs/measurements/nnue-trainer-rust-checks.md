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

| 検査 | 方法 | 結果 |
|---|---|---|
| 勾配の数値微分との一致 | 幅4、合成局面で、第1層、線形、バイアス、出力重み、出力バイアス、仮想行の勾配を中心差分と比較。補助損失の各区分が発火する例を含む（`cargo test`） | 通過 |
| P型の鏡映不変とエポック0の同一性 | 局面とその鏡映の評価の一致、同じシードのP型とHalfKP-hm-b8の評価の一致（`cargo test`） | 通過 |
| 量子化とMNUE版5の往復 | 量子化して符号化し、`minase`で復号して整数評価と浮動小数点の評価を比較（`cargo test`） | 通過 |
| PyTorchとの3更新の照合（実データ） | `dump-batch --width 4 --seed 71 --steps 3`で世代3の1ファイルの9局面を3バッチにし、`crosscheck_reference.py`でPyTorchの`minase_train.nnue.model`と`torch.optim.AdamW`に同じ初期状態を写して3更新し、損失、勾配、切り詰め後の重みを比較（相対10⁻⁴、絶対10⁻⁵） | 両方の型で一致。この標本では補助損失の罰則が発火しない（補助損失0）ため、線形の初期値を0倍にして除去差分を0にし下限側の罰則を全除去で発火させた標本（`--init-pst-scale 0 --init-std 0.5 --width 8`、12局面、補助損失0.17〜0.29）でも3更新を照合し、両方の型で一致した。上限側の罰則（参照の向きが駒の持ち主と逆の除去）は実データでは稀であり、数値微分の単体テストだけで検査している |
| J75のnpzの読み込み | FMの配列（`V`、`a`、`mask`）を含むnpzを読む | codexの初版は未知のメンバーを拒否したため、読み飛ばす形に直した |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（39件） | 通過 |

## フェーズ2　GPUカーネルと等価性テスト

### フェーズ2a　GPU基盤、第1層の前進、出力部、縮約、検証の前進

| 検査 | 方法 | 結果 |
|---|---|---|
| GPUとCPU参照の等価性 | `cargo test -p nnue-trainer -- --ignored gpu_`（幅2、6、8、128の両方の型と、面の環の転送）と`check-equivalence`（幅6のバッチ100、幅128のバッチ4,096と16,384、両方の型）。判定は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`、2回実行のビット単位一致 | すべて一致。幅128・バッチ16,384の最大絶対誤差はP型6.1×10⁻⁵、HalfKP-hm-b8で2.0×10⁻⁴（損失の部分和など大きな値で生じ、許容誤差の内側） |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし） | 通過 |

`unsafe`ブロックは`gpu`モジュールに10か所（ライブラリの存在検査2、ページ固定メモリの確保と初期化2、カーネル起動6）と、`data`のmmapに1か所である。

### フェーズ2b　補助損失の前進、第1層と線形経路の後退、仮想行の縮約

| 検査 | 方法 | 結果 |
|---|---|---|
| GPUとCPU参照の等価性 | `cargo test -p nnue-trainer -- --ignored gpu_`（幅2、6、8、128の両方の型）と`check-equivalence`（幅6のバッチ100、幅128のバッチ16,384、両方の型）。補助損失の下限と上限の罰則（上限は符号を反転した整数PSTの参照で発火させる）、不適格な除去、回転、空の逆索引の範囲を含む。判定は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`、2回実行のビット単位一致 | すべて一致。幅128・バッチ16,384の最大絶対誤差はP型6.1×10⁻⁵、HalfKP-hm-b8で2.0×10⁻⁴ |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし、50件） | 通過 |

カーネル起動の`unsafe`は4か所増えて、`gpu`モジュールに合計14か所である。

### フェーズ2c　AdamWと切り詰め、畳み込み、1更新の統合

| 検査 | 方法 | 結果 |
|---|---|---|
| 1更新と連続3更新の等価性 | `cargo test -p nnue-trainer -- --ignored gpu_`（19件）と`check-equivalence`（幅128、バッチ16,384、両方の型）。更新後の全パラメータと最適化器の状態、損失を比較。判定は`|GPU−CPU| ≤ 10⁻⁶ + 10⁻⁵·|CPU|`、2回実行のビット単位一致 | すべて一致 |
| 静的検査 | `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test -p nnue-trainer`（GPUなし、54件） | 通過 |

### 最初の速度測定（最適化前）

`bench-step --width 128 --batch 16384 --steps 50 --workers 6`を世代3の1ファイルで実行した（2026年10月12日4時ごろ。教師データの生成が同時12対局で並走し、負荷平均は約21）。

| 項目 | P型 | HalfKP-hm-b8 |
|---|---:|---:|
| 計算ストリームの1更新 | 150.4 ms | 141.3 ms |
| 転送ストリームの1バッチ | 2.7 ms（67 MB） | 2.7 ms（67 MB） |
| ワーカーの供給速度（6ワーカー） | 372,052局面/秒 | 388,047局面/秒 |
| 実効速度 | 105,676局面/秒 | 112,078局面/秒 |
| GPUメモリ | 575 MB | 877 MB |

GPUの1更新が律速であり、`nvidia-smi dmon`ではSM使用率100%、メモリ帯域使用率1〜8%だった。`--profile`によるカーネル別の内訳（P型）は、第1層の後退70.7 ms（47%）、線形経路の後退39.1 ms（26%）、補助損失の前進29.4 ms（19%）、出力部9.1 ms（6%）、第1層の前進1.3 msである。HalfKP-hm-b8では第1層の最適化器が26.4 msを加える。原因は、後退の2本が行ごとに1スレッドで逆索引の範囲を直列に走査して出現回数の多い行が尾を引くこと、補助損失と出力部でスレッド間の読み込みが合流しないこと、最適化器が要素ごとに倍精度の`pow`を計算することであり、いずれも並列化の形の問題である。

### フェーズ2d　カーネルの並列化（1回目）

第1層と線形経路の後退を固定長の区分に分けて2段で縮約し、補助損失は1ワープ1列、出力部は1ワープ1局面にし、AdamWの補正係数をホストで計算するようにした。等価性（`gpu_`テスト23件、`check-equivalence`の幅128・バッチ16,384、両方の型）は変更前と同じ判定で一致し、2回実行はビット単位で一致した。

| カーネル（P型、ms/更新） | 2d前 | 2d後 |
|---|---:|---:|
| first_layer_backward（2d後は部分和＋縮約） | 70.7 | 13.4 |
| linear_backward（同） | 39.1 | 0.4 |
| removal_forward | 29.4 | 45.1 |
| output_backward | 9.1 | 0.4 |
| forward_acc | 1.3 | 1.3 |
| 1更新の合計 | 150.4 | 61.6 |

HalfKP-hm-b8は141.3 msから66.3 msになり、`optimizer_first`は26.4 msから4.1 msになった。実効速度はP型248,557局面/秒、HalfKP-hm-b8で230,557局面/秒である。`removal_forward`が増えたのは、CPU参照の昇順の加算順を保つために32レーンの積をレーン0へ1つずつ送る直列のシャッフルにしたためであり、縮約順をGPUの固定の木に定義し直してCPU参照がそれに合わせる方式へ改める（フェーズ2e）。

## フェーズ3　学習ループと照合、1ファイルの試走

（実装後に書く。Pythonの試走との比較は[学習の記録](nnue-replacement-training.md)の値を使う。）

## フェーズ4　全データの試走

（実装後に書く。）

## 結論

（完了後に書く。）

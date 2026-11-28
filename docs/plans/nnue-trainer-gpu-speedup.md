# NNUE学習器のGPU高速化の設計書

## 先に読む要約

単一特徴変換層を持つNNUE構造の**Simple**と多層積み上げ型NNUE構造の**LayerStack**について、GPU学習カーネルおよび供給パイプラインから不要な全要素走査、GeForce系GPUで低速な倍精度浮動小数点数FP64の累積、および全行に対する勾配ゼロクリアを段階的に取り除き、学習スループットを引き上げる。
ここでNNUEはEfficiently Updatable Neural Network（差分更新型ニューラルネットワーク評価関数）、`HalfKP_vm`は自陣基準王駒72区分と駒配置13,392基底特徴から成る964,224行の特徴量集合を指す。
[ボトルネック調査報告](../audits/gpu-computation-bottlenecks-2026-11-28.md)の実測では、バッチサイズ16,384において`Simple`は1ステップのGPU計算46.795ミリ秒のうち最適化器**AdamW**（`simple_adamw`）が16.881ミリ秒、疎特徴勾配集約`simple_sparse_reduce`が12.606ミリ秒、および勾配`memset`ゼロクリアが2.338ミリ秒を占めた。
一方の`LayerStack`では1ステップ11.576ミリ秒のうち疎特徴の部分勾配算出`ls_sparse_partials`が3.761ミリ秒、順伝播`ls_forward`が3.838ミリ秒、出力ヘッドの部分勾配算出`ls_head_partials`が1.549ミリ秒、および`ls_adamw`が0.925ミリ秒を占めている。
まずAdamWカーネルで未観測行スキップと前バッチ触接行のみの局所ゼロクリアを厳密に導入し、次に疎特徴と出力ヘッドの勾配集約におけるFP64利用および非効率なブロック起動を見直し、各修正を兄弟構造である`Simple`と`LayerStack`の双方へ対称適用する。
すべての精度変更は対応するCPU参照実装の加算順序と同期させ、既存のGPU対CPU許容誤差基準`|GPU - CPU| <= 1e-6 + 1e-5 * |CPU|`を緩和せずに維持したうえで実対局ゲートで検証する。

## 状態

本設計書は2026年11月28日に起案された。
実装は未着手であり、レビュー後に利用者の着手の指示を待つ。

## 目的

この設計書の目的は、`crates/minase-trainer`における`Simple`と`LayerStack`の両NNUE学習器について、学習結果の数理的等価性および対局棋力を維持しつつ1ステップあたりのGPU計算時間とホストおよびデバイス間の同期待ちを短縮することにある。

具体的には、`lishogi`のBotおよび他対局プロセスを停止した単独のRTX 3060 Ti環境において、`Simple`の1スーパーバッチ（1チェックポイント間隔の更新まとまり）学習速度である基準317,181局面毎秒を引き上げるとともに、`LayerStack`の4スレッド基準686,613局面毎秒および16スレッド基準1,017,791局面毎秒の双方で速度を維持または向上させることを狙う。

## 適用範囲

本計画の対象は、`crates/minase-trainer`配下のCUDAカーネル、GPUステップ実行とバッファ管理、供給パイプライン、およびこれらに対応するCPU参照実装である。
具体的には`kernels/simple/*.cu`、`kernels/layerstack/*.cu`、`src/nnue/simple/gpu/*`、`src/nnue/layerstack/gpu/*`、`src/nnue/train/pipeline.rs`、`src/nnue/simple/reference.rs`、および`src/nnue/layerstack/reference.rs`を対象に含める。

一方で、NNUEのモデル構造（層数、隠れ層幅`H`、および[特徴量設計文書](../research/nnue-trainer-speed.md)に基づく特徴量集合Aと`HalfKP_vm`の定義）、量子化仕様、および`crates/minase`の推論エンジンは対象外とする。

## 依存関係

設計および検証は、次の既存ADR（Architecture Decision Record、アーキテクチャ決定記録）と教訓を前提条件とする。

- AdamW（Adaptive Moment Estimation with Decoupled Weight Decay）の数学的定義とバイアス補正は[PyTorchのAdamWと合わせるADR](../adr/use-pytorch-adamw-with-bias-correction.md)に従う。
- 第一層の疎逆伝播は[逆引きインデックスによる第一層逆伝播のADR](../adr/backpropagate-the-first-layer-through-a-reverse-index.md)を基盤とする。
- `unsafe`境界は[GPU学習器コードにのみunsafeを許可するADR](../adr/allow-unsafe-only-in-the-trainer-gpu-code.md)に限定し、`minase`本体へ広げない。
- 最適化の実装方針は[兄弟構造にも同修正を適用する教訓](../lessons/apply-a-kernel-speed-fix-to-sibling-structures.md)、[内側ループでf64を使わない教訓](../lessons/no-f64-in-gpu-kernel-inner-loops.md)、[共住負荷を停止し供給とGPU飽和を切り分けて比較する教訓](../lessons/compare-trainer-speed-with-co-resident-load-stopped-and-gpu-bound-threads.md)、および[CPU参照精度をGPUに合わせる教訓](../lessons/match-cpu-reference-precision-to-the-gpu-before-comparing-adamw-updates.md)に準拠する。

## 設計判断

### 未観測行スキップとゼロ勾配契約によるSimple AdamWの高速化

`HalfKP_vm`の特徴変換層（実特徴964,224行、総パラメータ250,287,457個）は1バッチ内の95.54%の行が未観測であるため、全要素を走査する単精度浮動小数点数FP32演算カーネル`simple_adamw`が16.881ミリ秒を要している。

しかし、すでに過去のステップで非ゼロの一次モーメントや二次モーメントを獲得した行、あるいはステップあたりの重み減衰乗数`decay = 1.0f - lr * weight_decay`が`1.0f`未満であるパラメータグループを更新から外すと、モーメント減衰と減衰項が脱落して`SparseAdam`へ変質し、[PyTorchのAdamWと合わせるADR](../adr/use-pytorch-adamw-with-bias-correction.md)に違反する。

したがって本設計書では、特徴変換層`ft`の重み減衰設定`ft.weight_decay == 0.0`により`decay == 1.0f`が成り立つパラメータ領域に限り、学習開始から一度も観測されていない行（ビットマップ`ever_observed[row] == 0`）だけを更新からスキップする方式を採る。
一度でも観測された行`ever_observed[row] == 1`が後続バッチに出現しない場合でも、その既観測行は`simple_adamw`が引き続き更新して一次および二次モーメントを減衰させる。
なお、既観測行とは異なり、更新をスキップする未観測行では勾配0、両モーメント0、かつ`weights[i] * 1.0f - 0.0f`であり、初期化および更新時の重みクランプ`[-limit, limit]`の範囲内に重みが収まっている限り既存の密`simple_adamw`と同一の値を保つ（クランプ境界の保持は不変条件として単体試験で確認する）。

また、チェックポイントからの再開時に従来版のビットマップを持たない古いチェックポイントを読み込む場合に備え、再開時には最適化器状態の一次モーメントまたは二次モーメントが非ゼロである行を走査して`ever_observed`を復元するか、ビットマップ自体を状態へ永続化する仕様とする。

さらに毎ステップ2.338ミリ秒を費やす全964,224行の`memset_zeros`を廃する際には、未出現の既観測行が前ステップの古い勾配を読み誤らないゼロ勾配契約を明確に定義する。
仮想特徴の勾配を実特徴行へ集約する`simple_virtual_reduce`、`simple_adamw`、および検証用`gradients()`のホスト読み出しは全行を読み得る。
そのため次ステップで未出現となった既観測行が常にゼロ勾配を読むよう、最適化器更新および勾配検証の後に当バッチで触れた行集合（仮想行を含む触接行）だけを局所クリアし、初期化時のみ全勾配バッファをゼロで埋める。

### 勾配集約とヘッド処理におけるFP64最小化およびブロック起動見直し

GeForce系GPUの倍精度浮動小数点数演算性能（RTX 3060 Tiでは0.253 TFLOPS）は単精度16.2 TFLOPSの64分の1に過ぎないため、カーネルの内側ループで`double`を使うと深刻な命令発行律速を起こす。

`simple_sparse_reduce`は全53,884セグメントから1ブロックずつ起動したうえで先頭セグメント以外は`if (segment != starts[row]) return;`で即座に終了させており、しかも内側ループで直接FP64累積を行っている。
そこで重複なしのアクティブ行配列`active_rows`（1バッチあたり約42,973行）と行ごとの出現区間配列を構築してアクティブ行単位でワープまたはブロックを割り当て、内側累積を`float`へ切り替える余地を検証する。

一方、兄弟構造である`LayerStack`の`ls_sparse_partials`自体はすでに特徴勾配を`float`で累積したうえで中間`partials`へFP64書き込みを行っており、内側FP64残存箇所は3箇所に限定される。
すなわち`ls_sparse_partials`内のPSQT（駒割位置表）部分、`ls_sparse_reduce`側の中間配列読み出し、および`ls_adamw`要素ループ内の`double`除算と平方根である。
したがって`LayerStack`では`ls_sparse_partials`の中間表現縮小、`ls_sparse_reduce`のFP32化、および`ls_adamw`におけるバイアス補正定数のホスト側事前算出をそれぞれ独立に計測し、効果と誤差を個別に確認したうえで採否を決める。

また`ls_head_partials`および`simple_head_partials`では共有メモリ上のブロック転置や複数局面同時処理によってグローバルメモリへのストライド読み出しを減らし、精度および加算順序を変えた際は`simple/reference.rs`と`layerstack/reference.rs`を同時に更新して既存の許容誤差基準`|GPU - CPU| <= 1e-6 + 1e-5 * |CPU|`を緩和せずに合格させる。

## 実装フェーズ

### フェーズ1: ベースラインの固定とカーネル単位ベンチマーク基盤の整備

[基礎測定記録](../measurements/adhoc/gpu-bottlenecks-20261128.md)で定めた`nnue bench-step --profile`およびCUDAイベント単体計測手順を基準線として固定し、各カーネル変更の前後比較を個別に分離して測定できる環境を整える。

### フェーズ2: SimpleのAdamW高速化とゼロ勾配契約の確立

`ever_observed`ビットマップ（チェックポイント再開復元を含む）と当バッチで触れた行のみを次更新前に消去する局所クリアを導入し、`memset_zeros`の全面クリアを廃止したうえで従来の密`simple_adamw`および`gradients()`との厳密一致テストを通過させる。

### フェーズ3: 疎特徴勾配集約とヘッド勾配集約およびLayerStackへの対称適用

`simple_sparse_reduce`のアクティブ行駆動化と内側ループFP32化、`simple_virtual_reduce`の走査圧縮、ならびに兄弟構造である`LayerStack`の`ls_sparse_partials`、`ls_sparse_reduce`、`ls_head_partials`、`simple_head_partials`、および`ls_adamw`の最適化を段階的に実装する。
各段階でCPU参照実装を同一加算順序へ更新し、既存のGPU対CPU一致テストに合格させる。

### フェーズ4: 供給パイプラインと統合検証

GPU計算短縮後の`pipeline.rs`を`4スレッド`および`16スレッド`で再計測してCPU供給側の詰まりを確認し、複数スーパーバッチ学習の一致と必要な対局検証を完了させる。

## 検証

検証は次の4水準で実施し、すべての変更は先行段階を通過したうえで次段階へ進む。

1. `crates/minase-trainer`の単体およびGPU一致テストを実行し、フェーズ2は従来の密AdamW対比で重みクランプを含め数値一致すること、フェーズ3以降はGPUとCPU参照実装の差が既存の許容幅（`|GPU - CPU| <= 1e-6 + 1e-5 * |CPU|`）に収まることを確かめる。
2. `lishogi`のBotおよび他対局プロセスを停止した状態で、`nnue bench-step --profile`（4スレッドおよび16スレッド）とCUDAイベント単体計測を実施し、`simple_adamw`、`simple_sparse_reduce`、`memset_zeros`、および`LayerStack`各カーネルの個別時間短縮を記録する。
3. `nnue train`を1スーパーバッチおよび複数スーパーバッチで走らせ、ビットマップを持たない旧形式のチェックポイントからの再開を含め、再開後の損失と学習曲線が一致することを確かめる。
4. 数値計算順序や勾配丸めを変えた成果物で学習したネットワークについて、現行比較対象網と`lishogi`対局規則での逐次確率比検定SPRTを実施し、強度の非劣性を確かめる（[棋力測定ガイド](../guides/sprt.md)）。

## 完了条件

- 特徴量`HalfKP_vm`およびバッチサイズ16,384における`Simple`の1ステップGPU計算時間が基準46.795ミリ秒から35%以上短縮され、16スレッド`nnue train`での実測速度向上を新しい測定記録として残している。
- `LayerStack`の該当変更が`Simple`と対称に実装され、4スレッド基準686,613局面毎秒および16スレッド基準1,017,791局面毎秒の双方に対して速度向上または非劣化を実測で確認している。
- 既存のCPU参照一致テスト（許容誤差の緩和なし）と旧チェックポイント互換を含む再開一致テストがすべて成功している。
- 測定記録および精度選択や最適化器条件に関するADRが整備されている。

## 参考資料

- [GPU計算ボトルネック調査報告](../audits/gpu-computation-bottlenecks-2026-11-28.md)
- [基礎測定記録（2026年11月28日）](../measurements/adhoc/gpu-bottlenecks-20261128.md)
- [NNUE学習器高速化調査メモ](../research/nnue-trainer-speed.md)
- [PyTorchのAdamWと合わせるADR](../adr/use-pytorch-adamw-with-bias-correction.md)
- [逆引きインデックスによる第一層逆伝播のADR](../adr/backpropagate-the-first-layer-through-a-reverse-index.md)
- [GPU学習器コードにのみunsafeを許可するADR](../adr/allow-unsafe-only-in-the-trainer-gpu-code.md)
- [兄弟構造にも同修正を適用する教訓](../lessons/apply-a-kernel-speed-fix-to-sibling-structures.md)
- [内側ループでf64を使わない教訓](../lessons/no-f64-in-gpu-kernel-inner-loops.md)
- [共住負荷を停止し供給とGPU飽和を切り分けて比較する教訓](../lessons/compare-trainer-speed-with-co-resident-load-stopped-and-gpu-bound-threads.md)
- [CPU参照精度をGPUに合わせる教訓](../lessons/match-cpu-reference-precision-to-the-gpu-before-comparing-adamw-updates.md)

# NNUE学習およびAlphaZero型PV-MCTSのGPU計算ボトルネック調査報告

## 文書の前提と位置づけ

この調査報告は、`master`のコミット`394b5583`（2026年11月28日時点）における中将棋プログラム`minase`のGPU計算ボトルネックを、実機計測とソースコード精査の両面から特定した記録である。

調査の対象は4つの処理系統にわたる。
第1は`crates/minase-trainer`のNNUE（Efficiently Updatable Neural Network、差分更新型ニューラルネットワーク評価関数）学習器における多層構造**LayerStack**および単一特徴変換構造**Simple**である。
第2は`tools/train/zero`のAlphaZero型強化学習トレーナーであり、強化学習パイプラインを以下では**RL**（Reinforcement Learning）と略記する。
第3は`crates/minase`の自己対局生成器`zero-selfplay`であり、第4は同クレートの対局用PV-MCTS（Policy-Value Monte Carlo Tree Search、方策と価値を用いるモンテカルロ木探索）である。

計測環境は単一GPUワークステーション（NVIDIA GeForce RTX 3060 TiおよびIntel Core Ultra 7 265KF）であり、Botおよび対局プロセスを停止した単独状態で実測した数値を[基礎測定記録](../measurements/adhoc/gpu-bottlenecks-20261128.md)および[results.json](../measurements/adhoc/gpu-bottlenecks-20261128/results.json)に保存している。

この調査報告は完了時点の観測を凍結した記録であり、事実確認を欠いたまま書き換えない。

## 観測した事実

### NNUE学習器におけるLayerStackとSimpleの計算内訳

バッチサイズ16,384および教師データ`book-0.mnsd`に対する`bench-step`の計測とNVIDIA Ampere世代のアーキテクチャ指定`sm_86`上のCUDAイベント計測により、`LayerStack`と後述する特徴量集合`HalfKP_vm`を用いる`Simple`の間で律速要因が明確に異なることを観測した。

`LayerStack`はCPUワーカー数16で1ステップ計算11.576ミリ秒、転送1.568ミリ秒であり、1ステップ計算の内訳は順伝播4.785ミリ秒、逆伝播5.942ミリ秒、および最適化器0.850ミリ秒であった。
`LayerStack`の1スーパーバッチ学習速度は4スレッドの686,613局面毎秒から16スレッドの1,017,791局面毎秒（学習総時間2.0605秒）へ向上した。

CUDAイベント計測の合計11.851ミリ秒のうち、`ls_forward`が3.838ミリ秒、`ls_sparse_partials`が3.761ミリ秒、`ls_head_partials`が1.549ミリ秒、`ls_statistics`が1.189ミリ秒、および`ls_adamw`が0.925ミリ秒であり、これら5本の合計11.262ミリ秒がイベント合計の95.0%を占めた。

他方の`Simple`は同じ16スレッドで1ステップ計算46.795ミリ秒（順伝播5.071ミリ秒、逆伝播24.826ミリ秒、および最適化器16.897ミリ秒）を要した。
`Simple`の1スーパーバッチ学習速度は317,181局面毎秒にとどまっており、CPUワーカーの供給速度は1,941,516局面毎秒までスケールしているため完全にGPU計算律速である。

`Simple`のCUDAイベント計測合計46.734ミリ秒の内訳では、重み減衰分離型適応モーメント推定の最適化器`simple_adamw`が16.881ミリ秒（全体の36.1%）を占める。
残りの主要処理は`simple_sparse_reduce`が12.606ミリ秒、`simple_head_partials`が4.876ミリ秒、`simple_forward`が4.368ミリ秒、`simple_virtual_reduce`が3.968ミリ秒、および毎ステップ実行される勾配`memset`ゼロ初期化が2.338ミリ秒であった。

72個の王バケットと1バケットあたり13,392個の基底特徴からなる特徴量集合`HalfKP_vm`の1バッチ（32,768視点）における統計では、964,224実特徴行のうち非空行は42,973行で観測率は4.46%に過ぎず、未観測率が95.54%を占めた。
出現行における出現回数の中央値は2.0回であり、出現回数4回以下の行が全体の75.4%を占めた。

### 対局用PV-MCTSの推論と探索バランス

20ブロック256チャンネルの残差ネットワーク`ZeroNet`（パラメータ数25,182,204、推論計算量7.24 GFLOPs毎局面）に対する`nn-eval`の288局面実測を行った。
ONNX Runtime上の実行プロバイダであるCUDA Execution Providerの単精度浮動小数点数FP32推論はバッチ幅16から256にかけて1,675.3〜1,740.6局面毎秒で頭打ちになった。
これに対し、TensorRT上の半精度浮動小数点数FP16推論はバッチ幅1で1,858.4局面毎秒、バッチ幅16で7,190.5局面毎秒、およびバッチ幅128で7,728.0局面毎秒まで伸びた（バッチ幅256は未計測）。

対局用の実探索を非確定中盤5局面とTensorRT条件で計測した。
800ノード指定時の所要時間とスループットは、バッチ幅1で毎手428.0ミリ秒と1,869.1シミュレーション毎秒、バッチ幅16で130.2ミリ秒と6,143.2シミュレーション毎秒、バッチ幅128で118.2ミリ秒と6,769.8シミュレーション毎秒であった。

3,200ノード指定時の所要時間とスループットは、バッチ幅16で毎手507.6ミリ秒と6,304.1シミュレーション毎秒、バッチ幅128で444.6ミリ秒と7,197.1シミュレーション毎秒であった。
バッチ幅16における実探索速度は推論単体スループットの85.4%〜87.7%を達成しているものの、バッチ幅16から128への実効スループットの伸び幅は800ノード時で10.2%、3,200ノード時で14.2%であった。

CPU側のダミー評価を用いたMCTS単体ベンチマーク（15局面各800ノード）はバッチ幅16で毎秒46,580シミュレーション、バッチ幅128で毎秒53,154シミュレーションを記録した。

### AlphaZero自己対局と学習パイプラインの構成

組み替え初期配置`shuffled-setup`、シード`9100000000`、各スレッド128同時対局スロット、および4シミュレーションの条件で`zero-selfplay`をRTX 3060 Ti上で実測した。
1スレッド128局では毎秒1,065.8局面を生成してGPU稼働率`utilization.gpu`が平均90.7%に達し、4スレッド512局では毎秒1,490.1局面を生成して`utilization.gpu`が平均98.0%（最大100%）に達した。
4スレッド時のメモリ制御器使用率`utilization.memory`は平均24.0%、最大VRAM占有は2,157 MiBであった。

なお、各シミュレーションで最大1回の葉評価に加えて根の評価が生じるため、1記録局面あたり最大5評価の積算で見積もった評価要求の上限は4スレッド時で約7,451局面毎秒に達する（各ワーカーは1バッチにつき1対局あたり最大1件の評価要求を送る）。

PyTorch製RLトレーナー`tools/train/zero/minase_zero/train.py`、`encoding.py`、および`runner.py`のソースコード検査から、前処理とコンパイル設定の実装を確かめた。
`accumulate_gradients`はミクロバッチごとの勾配蓄積中に`encode_planes`をコンパイル済み`model`の外側で呼び出しており、その内部の123行目で`boards.cpu().numpy()`および`sides.cpu().numpy()`を経由してCPU側Rust拡張`minase_mnsd_reader.control_planes`へ64枚の制御面の生成を依頼し、結果を再度GPUへ転送している。
その直後、`encoding.py`はターゲット機器上で159面全体の`float32`テンソルを組み立て、`train.py`の490行目が`torch.autocast("cuda", dtype=torch.bfloat16)`の下でBF16（Brain Floating Point 16-bit）へ丸めて順伝播へ渡している。

また、本番ランナー`runner.py`の125行目が定めるミクロバッチ既定値は256であり、`train.py`の315行目に記載されている毎秒990局面という値はRTX 3060 Tiでの再開一致試験時の過去の実測記録である。
`crates/minase/src/nn/hash_evaluator.rs`にある`HashEvaluator`は`bench`専用の状態を持たない評価器であり、自己対局ワーカー`zero_selfplay`および対局用MCTSの`OnnxEvaluator`の双方にはニューラルネットワーク（以下NNと略記）出力の実際のキャッシュとバッチ内重複除去が存在しない。

## 原因の仮説

観測した数値とソースコードの構造から、各系統のボトルネックについて次の原因仮説を立てた。

### NNUE学習器がGPU計算律速とメモリアクセス律速に陥る原因

`Simple`の最適化器`simple_adamw`が16.881ミリ秒を要する主因は、`kernels/simple/optimizer.cu`が250,287,457個（重み、一次モーメント、二次モーメント、および勾配の4配列で4 GB超）のパラメータを毎ステップ全要素走査していることにある。
特徴変換層は重み減衰`ft.weight_decay = 0.0`（`decay = 1.0f`）で学習しているが、過去一度でも出現した行に加えて一度も出現していない未観測行まで毎ステップ読み書きするため、RTX 3060 TiのGDDR6帯域448 GB/sを1ステップ16.9ミリ秒にわたって消費している。

`Simple`の逆伝播が遅い要因は2つある。
第1に、`simple_sparse_reduce`がGeForce系GPUでFP32の64分の1しかスループットが出ない倍精度浮動小数点数（FP64、0.253 TFLOPS）で出現要素の内側累積を行っている。
第2に、同カーネルは全53,884セグメントから1ブロックずつ起動したうえで先頭セグメント以外のブロックを`starts[row]`判定で即座に終了させる構造であるため、出現回数が中央値2回と短い`HalfKP_vm`で非連続ストライド読み出しと起動効率の低下を招いている。
加えて`Simple`は毎ステップ2.338ミリ秒を要する密勾配ゼロクリア`memset_zeros`を964,224行すべてに対して発行している。

一方、`LayerStack`の`ls_sparse_partials`自体は特徴勾配の累積をすでに`float`で行ったうえで中間`partials`配列へ`double`として書き出しており、Piece-Square Tableの略である駒割位置表PSQT成分のみ内側でFP64を使う一方で、`ls_sparse_reduce`が`double`の中間和を足し上げている。
ヘッド部集約では`ls_head_partials`が1.549ミリ秒、および`simple_head_partials`が4.876ミリ秒を要しており、いずれも1ブロックが32列と128局面を担当してグローバルメモリ上の`forward`と`backward`を縦方向へ読み出すため、ストリーミングマルチプロセッサSMあたりの演算密度が低く主記憶DRAM（Dynamic Random Access Memory）帯域待ちになりやすいと推論される。
さらに`ls_adamw`（0.925ミリ秒）は全要素ループ内で`double`の除算と平方根を実行している。

### 対局用PV-MCTSの推論利用効率が落ちる原因

対局用MCTSが同期的に止まる理由は、`engine.rs`の`run_inner`（638〜669行目）が`select_leaf`のループでバッチを集め終えてから、内部で特徴符号化とONNX Runtimeの`run_binding`を含む`OnnxEvaluator::evaluate`を同期待ちで実行しているためである。
GPU推論中はCPUの探索木走査と盤面再生が止まり、逆に葉収集と特徴符号化中はGPUが遊休状態になる。
また`driver.rs`（346行目）は`LeafKind::New`の葉へ到達するたびに`Game`状態を根から再生して合法手と終局裁定を行っており、木が深くなるほどCPU時間を押し上げていると推論される。

加えて`SearchConfig::default`の既定バッチ幅256はバッチ幅16に比べて実効推論スループットの伸びが小さいにもかかわらず1バッチの所要時間が長いため、`timed_batch_size`の線形見積もりによる切れ端時間での早期終了や`Step::Collision`停止時の実効バッチ割れを増やしている可能性が高い（ただし発生頻度は未検証）。
また`OnnxEvaluator`前段にバッチ内重複排除とNN出力キャッシュがないため、置換局面の再評価を防げていない。

### AlphaZero自己対局と学習器のGPU効率が落ちる原因

自己対局`zero-selfplay`が4スレッドで`utilization.gpu`平均98.0%（最大100%）かつ`utilization.memory`24.0%に達していることは、バッチ幅やCPUスレッドを増やすだけではGPU演算器の活動時間あたりの生成速度が大きく伸びにくい領域に入ったことを示す。
この条件下では、1手ごとの探索木破棄に伴って再訪される序盤や置換局面の評価をNN出力キャッシュで省くアプローチが有効になると推論される。

RLトレーナー（`train.py`と`encoding.py`）では、`encode_planes`が毎バッチ`boards`と`sides`をCPU側へ戻して64枚の制御面を受け取り、再びGPUへ送って`float32`で159面を組んだうえで`bfloat16`へ変換している。
このCPU同期往復と冗長な`float32`テンソル確保が、ミクロバッチ内のGPU待機時間やPCIe（PCI Express）帯域を消費していると推論される。

## 未検証の事項

1. 本番設定（`runner.py`の`MICRO_BATCH_SIZE = 256`およびクラウドGPUを含む環境）でのPyTorch Profilerトレースは未取得であり、`encode_planes`内の`.cpu().numpy()`同期や`torch.any`および`nonzero`の待機比率は実測で確認する必要がある。
2. `zero-selfplay`において1局内の前後手や複数スレッドの対局間でNN評価の重複が何%発生しているかは実測していない。
3. 対局用PV-MCTSで発生する`HardLimit`到達回数、`Step::Collision`中断時の実効バッチ幅分布、および`driver.rs`の`Game`再生が占めるCPU時間割合は未計測であり、改善後の実戦棋力は逐次確率比検定SPRT（Sequential Probability Ratio Test）で検証する必要がある。
4. `Simple`および`LayerStack`で内側ループの加算精度や`ls_adamw`の定数計算を変更した場合に、既存の許容誤差（絶対差と相対差の基準）および学習後の対局棋力が維持されるかは設計書の検証段階で確かめる必要がある。

## 再現資料

本報告に記したすべての測定値、使用コマンド、ハードウェア条件、および再現用スクリプトは以下の文書に保存した。

- [基礎測定記録（gpu-bottlenecks-20261128.md）](../measurements/adhoc/gpu-bottlenecks-20261128.md)
- [数値控え（results.json）](../measurements/adhoc/gpu-bottlenecks-20261128/results.json)
- [NNUEカーネル計測スクリプト（kernel_probe.py）](../measurements/adhoc/gpu-bottlenecks-20261128/kernel_probe.py)
- [PV-MCTSと自己対局の計測スクリプト（mcts_probe.py）](../measurements/adhoc/gpu-bottlenecks-20261128/mcts_probe.py)

この調査結果に基づき、影響クレートと検証ゲートごとに分離した3件の解消計画を起案した。
対象は[NNUE学習器のGPU高速化の設計書](../plans/nnue-trainer-gpu-speedup.md)、[AlphaZero型探索の学習器と自己対局生成のGPU実行の高速化の設計書](../plans/alphazero-rl-gpu-speedup.md)、および[対局用PV-MCTSの葉収集、バッチ推論、およびNNキャッシュの高速化の設計書](../plans/mcts-search-gpu-speedup.md)の3件である。

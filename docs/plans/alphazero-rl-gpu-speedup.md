# AlphaZero型探索の学習器と自己対局生成のGPU実行の高速化の設計書

## 先に読む要約

AlphaZero型強化学習におけるPyTorch製学習器`tools/train/zero`とRust製自己対局生成器`crates/minase/src/bin/minase/zero_selfplay`について、前処理時のGPUおよびホスト間の同期往復と自己対局中の重複ネットワーク評価を取り除き、1サイクルあたりの学習時間と対局生成時間を短縮する。
ここでRLは強化学習、NNはニューラルネットワーク、MNSDは教師データ形式のMinase Neural Self-play/Search Data、DDPは分散データ並列のDistributed Data Parallel、および`ZeroNet`は中将棋向けの残差ネットワーク本体を指す。
[ボトルネック調査報告](../audits/gpu-computation-bottlenecks-2026-11-28.md)では、学習器の`encode_planes`が64枚の制御面を生成する際にテンソルをCPUへ書き戻して`minase_mnsd_reader.control_planes`を呼び出し、その結果をGPUへ再転送したうえで159面テンソル全体を単精度浮動小数点数FP32で組んでからBF16（Brain Floating Point 16-bit）へ縮小している点が確認された。
一方で`zero-selfplay`は、4スレッド並列時にGPU稼働率平均98.0%へ達しながら前後手や他対局間で既評価のNN出力を再利用する仕組みを持っていない。
本計画の学習器側では、まずPyTorch Profilerでミクロバッチ前処理とGPU同期待ちの比率を測ったうえで、ピン留めメモリを用いたホスト側の非同期先読み転送を主案としつつGPU直接制御面生成の副案と比較して前処理方針を確定する。
また自己対局側では、探索木外へ完全キー照合付きの有界なNN出力キャッシュとバッチ内重複排除を組み込み、実測ヒット率とスループットの計測ゲートを通過した場合に限って採用する。
最後に学習器側ではGPU上のInductorコンパイルキャッシュを空にした状態からの`bf16`再開完全一致を、自己対局側では生成教師データの品質と較正値を検証する。

## 状態

本設計書は2026年11月28日に起案された。
実装は未着手であり、レビュー後に利用者の着手の指示を待つ。

## 目的

この設計書の目的は、強化学習第3回サイクル以降の計算時間と費用を圧縮するため、RL学習器と自己対局生成器`zero-selfplay`の双方でGPU演算器の実効利用率および1秒あたりの有効完了局面数を向上させることにある。

具体的には、学習器におけるCPU同期往復と冗長なテンソル変換を廃し、自己対局においては同じGPU計算時間あたりで完了できる独立な探索局面数を増やす。

## 適用範囲

本計画の対象は、`tools/train/zero/minase_zero/`の`encoding.py`、`train.py`、および`runner.py`周辺の前処理供給経路、Rust拡張`tools/train/zero/mnsd_reader/`、ならびに`crates/minase/src/bin/minase/zero_selfplay/`である。

なお、探索の比較対象ベースラインは1手あたり32シミュレーションと20ブロック256チャンネルの`ZeroNet`であり、[AlphaZero型強化学習第3回の設計書](alphazero-rl-3.md)が定める64シミュレーションおよび価値ヘッドと大域層拡張後の本番条件でも同設計が機能するよう構成する。
対局用モンテカルロ木探索PV-MCTS（Policy-Value Monte Carlo Tree Search）の通常パスおよび時間管理の最適化は、[対局用PV-MCTSの葉収集、バッチ推論、およびNNキャッシュの高速化の設計書](mcts-search-gpu-speedup.md)へ分離して扱う。

## 依存関係

設計および検証は、次の既存設計書、ADR（Architecture Decision Record、アーキテクチャ決定記録）、および教訓に依存する。

- 第3回強化学習の学習量および計算予算は[AlphaZero型強化学習第3回の設計書](alphazero-rl-3.md)を前提とする。
- データ読み込みおよび制御面表現の仕様は[MNSD読み込みとRust拡張制御面のADR](../adr/read-lishogi-mnsd-into-the-zero-trainer-and-build-control-planes-in-a-rust-extension.md)に従う。
- `torch.compile`と丸めおよび決定性設定は[DDP内コンパイルとBF16丸めのADR](../adr/compile-the-zero-training-forward-inside-ddp-with-eager-rounding-and-deterministic-inductor-kernels.md)および[コンパイル学習のBF16 GPU検証教訓](../lessons/check-compiled-training-numerics-on-the-gpu-in-bf16.md)に準拠する。
- 自己対局の木生成規則は[手番ごとの新鮮な木でのバッチ自己対局ADR](../adr/batch-zero-self-play-leaves-across-many-games-with-a-fresh-tree-per-move.md)を保ちつつ評価値のみを再利用する。
- 候補「自己対局でのNN出力の再利用」は[Leela Chess Zeroから移せる工夫の提案](../proposals/lc0-transfer-candidates.md)から本設計書へ移管された項目である。

## 設計判断

### 学習器におけるcontrol_planes往復の解消と同期点削減

`encoding.py`の`encode_planes`はコンパイル済み`model`の外側で実行され、盤面入力から159枚の`FP32`平面テンソルを組み立てる途中で`.cpu().numpy()`を経由して`minase_mnsd_reader.control_planes`から64枚の制御面を受け取り、その後GPUへ戻している。
また本番ランナー`runner.py`はミクロバッチ幅256（`MICRO_BATCH_SIZE = 256`）で動作しており、`train.py`には別の検証実験における1秒あたり990局面という過去の再開検証記録が残っている。

この前処理ボトルネックを解消するため、フェーズ1で本番相当のミクロバッチ幅256に対してPyTorch Profilerを走らせ、`control_planes`の往復だけでなく`encode_planes`内の`torch.any`や`nonzero`、さらに`train.py`の`losses`に対する毎ステップ`.item()`検査を含むすべてのCPU同期点の所要時間を実測する。
その実測に基づき、次の2案から採用方式を決める。

1. 主案のピン留めメモリ（ページロックメモリ）を用いたホスト側の非同期先読み転送では、CPU並列ワーカーがMNSDの圧縮記録から159枚相当の入力を直接生成し、サンプル順、`world_size`ごとのミクロバッチ境界、および各ミクロバッチのバッチ正規化統計更新順序を現行と完全に保ったまま非同期転送でGPUへ送り込む。
2. 副案のGPU上制御面計算では、`control.rs`の仕様をGPU演算へ移植してホスト戻り自体をなくすが、獅子、飛び駒、およびブロック駒の厳密仕様を2系統へ二重化する保守コストを比較対象とする。

なお`compile_training`の決定性設定（`deterministic = True`および`emulate_precision_casts = True`）自体は再開再現性を守るために維持し、`bf16`時の冗長な`FP32`確保を抑制する入力型整理がコンパイル済み順伝播の安定性を壊さないかをGPU上のInductorコンパイルキャッシュを空にした再開テストで確認する。

### 自己対局における有界NN出力キャッシュとバッチ内重複排除

`zero-selfplay`における重複排除とキャッシュには、既存の`HashEvaluator`（`crates/minase/src/nn/hash_evaluator.rs`）は使えない。
`HashEvaluator`はネットワークを使わずFowler-Noll-Vo hash function 1aの非暗号ハッシュ関数FNV-1aから擬似値とロジットを生成する`bench`専用の評価器であり、実際の交換型学習済みモデル形式ONNX（Open Neural Network Exchange）の出力を保持するキャッシュ機能を持たないからである。

したがって`crates/minase`内の推論境界へ、実際のネットワーク出力（パック済み半精度浮動小数点数FP16の方策ロジットおよびFP32勝率価値）を保持する新しい有界キャッシュを設計する。
検索鍵には同一盤面と手番に加え、中将棋の先獅子（直前の手での獅子捕獲に対する反撃制限）の判定に使う`lion_capture_square`、ネットワークのSHA-256ハッシュ、および規則コードを含める。
キャッシュ索引時は短縮ハッシュで引き当てたうえで、ハッシュ衝突による誤参照を防ぐために識別鍵全体の完全キー照合を必須とする。

また`worker.rs`の設計上、1バッチ内の要求は同時進行する別々の対局スロットから最大1件ずつ集まるため、バッチ内重複排除は対局間の同一序盤や合流局面を畳み込む形で機能する。
なお、バッチ幅や要素配置が変わるとGPU側の浮動小数点丸め順が微小に変化して探索手や較正値へ影響する可能性があるため、キャッシュとバッチ重複排除の導入後には教師出力と較正統計の変動幅を実測で確かめる。

## 実装フェーズ

### フェーズ1: 本番バッチ幅でのプロファイルと重複率の測定

`runner.py`準拠のミクロバッチ幅256および学習ステップ条件で`train.py`のPyTorch Profilerを取得し、`control_planes`往復、`encode_planes`内の`torch.any`や`nonzero`、および`.item()`同期点の時間割合を確定する。
並行して4スレッド実行の`zero-selfplay`（標準初期配置`initial`とランダム初期配置`shuffled-setup`の両条件）においてバッチ間およびバッチ内の局面重複率を計測し、キャッシュの理論上限効果を見積もる。

### フェーズ2: RL学習器の前処理パイプライン改善と再開検証

フェーズ1の計測結果に基づいて主案（ピン留めメモリを用いたホスト側非同期先読み転送）または副案を採用し、`encode_planes`の同期バブルと過剰な`FP32`作業メモリを取り除く。
実装では元の抽出順、分散ランク別ミクロバッチ割り当て、およびバッチ正規化の移動統計更新順序を維持し、同一GPU台数かつInductorコンパイルキャッシュを空にした条件で中断と再開の完全一致を確認する。

### フェーズ3: 自己対局用有界NNキャッシュの実装と計測ゲート判定

完全キー照合付きの有界NN出力キャッシュとバッチ内重複排除を自己対局用の推論境界へ実装し、`initial`と`shuffled-setup`の双方で1秒あたりの完了局面数を測る。
キャッシュ管理コストが再評価削減の利得を上回る条件やスループットが低下する場合は、設定によるサイレントフォールバックを残さず明示的な計測ゲート判定として対象経路から外し、結果を記録へ残す。

### フェーズ4: 統合検証と第3回強化学習のコスト再見積もり

最適化後の学習器と自己対局生成器を用い、32シミュレーション基準網と[AlphaZero型強化学習第3回の設計書](alphazero-rl-3.md)が想定する64シミュレーションおよび拡張網の双方で1サイクルあたりの所要時間と予算見込みを更新する。

## 検証

検証は学習器側と自己対局側で次の基準を満たしたときに完了と判定する。

1. GPU上で`precision="bf16"`、PyTorch Inductorのコンパイルキャッシュを空にした状態、かつ同一ワールドサイズで強制中断と再開を実行し、変更前後で学習後の重みと損失曲線が基準実装と許容誤差内に収まり、再開前後が完全に一致することを確認する（[コンパイル学習のBF16 GPU検証教訓](../lessons/check-compiled-training-numerics-on-the-gpu-in-bf16.md)）。
2. `runner.py`準拠のミクロバッチ256条件および単独GPU環境で、プロファイル上のCPU待機時間比率と定常状態の1秒あたりの完了局面数の向上幅を測定記録として残す。
3. 自己対局生成について、固定シードかつ同一ネットワークでキャッシュ有効時と無効時の生成局面数、終局割合、較正統計の安定性、およびバッチ形状変更に伴う数値挙動差を検証し、1秒あたりの実効完了局面数の向上率を記録する。

## 完了条件

- `tools/train/zero`における`control_planes`同期往復およびプロファイルで特定したCPU同期点が解消され、学習スループットの向上が実測記録に保存されている。
- BF16精度と`torch.compile`を用いたPyTorch Inductor空キャッシュからのチェックポイント中断および再開完全一致テストが同一ワールドサイズ下のGPU実機で通過している。
- `zero-selfplay`向けの完全キー照合付き有界NNキャッシュとバッチ重複排除について、効果測定に基づく採用または非採用の判定が完了し、手数、終局、価値、および方策からなる教師生成の正しさと較正値の整合が確かめられている。
- すべての実測結果が新しい測定記録に整理され、[AlphaZero型強化学習第3回の設計書](alphazero-rl-3.md)から参照可能な状態になっている。

## 参考資料

- [GPU計算ボトルネック調査報告](../audits/gpu-computation-bottlenecks-2026-11-28.md)
- [基礎測定記録（2026年11月28日）](../measurements/adhoc/gpu-bottlenecks-20261128.md)
- [AlphaZero型強化学習第3回の設計書](alphazero-rl-3.md)
- [Leela Chess Zeroから移せる工夫の提案](../proposals/lc0-transfer-candidates.md)
- [MNSD読み込みとRust拡張制御面のADR](../adr/read-lishogi-mnsd-into-the-zero-trainer-and-build-control-planes-in-a-rust-extension.md)
- [DDP内コンパイルとBF16丸めのADR](../adr/compile-the-zero-training-forward-inside-ddp-with-eager-rounding-and-deterministic-inductor-kernels.md)
- [手番ごとの新鮮な木でのバッチ自己対局ADR](../adr/batch-zero-self-play-leaves-across-many-games-with-a-fresh-tree-per-move.md)
- [コンパイル学習のBF16 GPU検証教訓](../lessons/check-compiled-training-numerics-on-the-gpu-in-bf16.md)

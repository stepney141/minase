# 対局用PV-MCTSの葉収集、バッチ推論、およびNNキャッシュの高速化の設計書

## 先に読む要約

方策と価値を用いるモンテカルロ木探索の対局用PV-MCTS（Policy-Value Monte Carlo Tree Search）について、同期停止型の葉収集と推論ループを見直し、1手あたりの実質探索ノード数と時間管理の安定性を高める。
[ボトルネック調査報告](../audits/gpu-computation-bottlenecks-2026-11-28.md)の実測では、RTX 3060 Ti上のTensorRTによるFP16（半精度浮動小数点数）推論がバッチ幅16ですでにピーク性能7,728.0局面毎秒の93%に達する一方、既定バッチ幅256では`OnnxEvaluator::evaluate`が完了するまで探索駆動部`Driver`が停止している。
本計画ではまず1手の詳細内訳と評価重複率の計測基盤を整備したうえで、バッチ内重複排除と新しい有界NN出力キャッシュを統合し、さらにバッチ幅の再選定と先行チェスエンジンLeela Chess Zeroの`lc0`および将棋エンジン`dlshogi`に倣う衝突予算と平均行動価値Qを変えないQ値中立仮想訪問を比較検証する。
ここでNNはニューラルネットワークを指す。
次に、`LeafKind::New`到達時に`Driver::classify_child`が行う完全な`Game`状態（千日手履歴と裁定を含む）の根からの再生を軽量化し、あわせて特徴符号化を`OnnxEvaluator::evaluate`の外へ切り出す。
そのうえで、`Driver`専有の探索スレッドと`OnnxEvaluator`専有の推論スレッドを二重バッファおよびチャネルで分離する非同期バッチ化（out-of-order batching）を構築し、5種の`StopReason`と評価エラーおよび木証明を含む全終了経路で`Driver::assert_finished`契約を破らない一回限りの完了と取消のプロトコルを定義する。
探索における葉の振り分けや予約規則を変える各変更は、Universal Shogi InterfaceのUSIベンチマーク速度だけでなく、基準エンジンに対する逐次確率比検定SPRT（Sequential Probability Ratio Test）による非劣性以上の対局結果をもって採否を決める。

## 状態

本設計書は2026年11月28日に起案された。
同日に元にした調査報告が訂正されたが、本計画が前提とするコードの構造と数値は訂正の対象に含まれなかった。
ただし報告の数値からは、作業機のRTX 3060 Tiでは推論以外の処理が1シミュレーションの時間の約12〜15%にとどまるので、推論と木の処理の重ね合わせと経路の再生の軽量化で縮められる時間は合わせて最大でもその割合と見積もられる（推論）。
推論の速いGPUでは、CPUの木の処理の占める割合が増え、得られる速さも大きくなる。
実装は未着手であり、レビュー後に利用者の着手の指示を待つ。

## 目的

この設計書の目的は、USIおよび外部対戦パスを含む通常対局探索におけるPV-MCTSの単位時間あたり探索ノード数と持ち時間制御の正確さを高め、同一持ち時間かつ同一GPU環境下での実戦棋力を向上させることにある。

具体的には、バッチ幅と衝突制御をハードウェア飽和特性へ適合させるとともに、重複推論のキャッシュ化、盤面経路再生の軽量化、およびGPU推論とCPU探索のオーバーラップ実行を通じて計算機の遊休時間を削減する。

## 適用範囲

本計画の対象は、`crates/minase/src/search/mcts/`配下の`engine.rs`、`driver.rs`、`tree.rs`、および`puct.rs`を含む探索制御と、`crates/minase/src/nn/`配下の`onnx.rs`、`encoding.rs`、および`evaluator.rs`を含む対局推論境界である。

一方で、`crates/minase/src/bin/minase/zero_selfplay/`とPyTorch製学習器は[AlphaZero型探索の学習器と自己対局生成のGPU実行の高速化の設計書](alphazero-rl-gpu-speedup.md)へ分離する。
さらに`crates/minase`全体の`unsafe_code = "forbid"`制約およびPUCT（Predictor Upper Confidence Trees）定数自体の数値最適化は変更対象から外し、定数調整は[PUCTの係数と方策の温度をSPSAで調整する設計書](mcts-puct-spsa.md)で扱う。

## 依存関係

設計および実装は、以下の既存規約と提案文書を出発点とする。
ここでADRはArchitecture Decision Record（アーキテクチャ決定記録）を指す。

- 木構造と仮想損失の枠組みは[仮想損失バッチ付きPUCTによるMCTS探索のADR](../adr/search-mcts-games-by-puct-with-virtual-loss-batches.md)に従う。
- GPUとCPU間の推論境界では、[ピン留めメモリのuint8入力とパック済みFP16方策によるZero推論テンソル交換のADR](../adr/exchange-zero-inference-tensors-as-uint8-inputs-and-packed-fp16-policies-in-pinned-memory.md)と`unsafe_code = "forbid"`制約（[GPU学習器コードにのみunsafeを許可するADR](../adr/allow-unsafe-only-in-the-trainer-gpu-code.md)）を維持する。
- 本設計書は[Leela Chess Zeroから移せる工夫の提案](../proposals/lc0-transfer-candidates.md)の4項目および[AlphaZero型探索の棋力差の提案](../proposals/alphazero-strength-gap.md)の3項目を統合して実現する。
前者から引き継ぐ項目は第1に実戦のバッチ幅の選び直しと1手の時間の内訳の計測、第2にNN評価の重複の計数と対局用のNNキャッシュおよびバッチ内の重複の除去、第3にQを動かさない仮想訪問と木が大きいときだけの衝突の予算、第4にNNの推論の間に次の葉を集める重ね合わせと推論器が空いたときの早期の送信である。
後者から引き継ぐ項目は、経路の再生を軽くすること、推論と木の処理を重ね合わせること、およびNNキャッシュである。
- 先行エンジン比較の根拠として[lc0探索調査メモ](../research/lc0-transfer/search.md)および[棋力差探索調査メモ](../research/alphazero-strength-gap/search.md)を参照する。

## 設計判断

### バッチ幅、衝突予算（collision budget）、およびQ値中立仮想訪問virtual visitと先行実装比較

[lc0探索調査メモ](../research/lc0-transfer/search.md)に確認されたとおり、`lc0`のコードは小バッチ最適化のプルリクエストPR `#1532`、木規模に応じた衝突予算拡大関数`CalculateCollisionsLeft`を導入したPR `#1540`、推論器空き時の早期送信を導入したPR `#1503`、バッチ内重複排除のPR `#2417`、および探索項分母のみで進行中訪問数を数えて平均行動価値Qを下げない`GetNStarted()`を備える。
これに対して`dlshogi`および現行の`minase`は、仮想損失を訪問回数とQ値の分母双方へ加算しており（`puct.rs` 94〜99行目）、かつ`engine.rs`の`run_inner`が最初の`Step::Collision`で即座にバッチ収集を打ち切るため、既定バッチ幅256が衝突で細切れに割れるか逆に長大バッチで`timed_batch_size`の時間残余見積もり誤差を拡大させる。

そこでフェーズ1では1手の時間を木の持ち越し準備、葉収集と盤面再生、特徴符号化、GPU推論、逆伝播、および最善手構築に分けて実測し、`HardLimit`や衝突発生頻度を定量化する。
そのうえで、バッチ幅再選定、`CalculateCollisionsLeft`型の衝突予算、および`GetNStarted()`型のQ中立予約を独立に設定変更できる構造とし、ノード数固定および時間固定の双方でSPRT比較を行って採否を決める。

### 盤面経路再生の軽量化とGame状態の保持

`driver.rs`の`classify_child`（346行目）は`LeafKind::New`の葉に対してのみ呼び出され、`self.snapshot.game().clone()`に対して経路上の全着手を順番に適用し、`Game::play`の終局裁定、千日手の反復キー履歴`repetition_key`、および3手以内詰み判定を維持したうえで`Position`と合法手を組み立てている。

したがって、単に`Position`の差し替え戻しスタックだけを導入しても、`Game`が保持する反復履歴と裁定状態が脱落してしまい正しく代替できない。
本設計書では、既定256 MiBの探索木メモリ予算を圧迫して大容量木が溢れる弊害をなくすため全節点に盤面を保持する方式は採らず、`Game`状態全体に対する差分チェックポイントによって完全な同一判定を保ったまま`LeafKind::New`1回あたりの再生長さを圧縮する。
具体的には、展開頻度が高い浅い階層や直近探索枝に沿った`Game`の共有参照、増分履歴、部分クローン、あるいは親節点からの1手適用と巻き戻しを比較する。

### 新しい有界NN出力キャッシュ（NN cache）とバッチ内重複除去

`crates/minase/src/nn/hash_evaluator.rs`の`HashEvaluator`は`bench`専用の無状態非暗号ハッシュ関数FNV-1a（Fowler-Noll-Vo hash function 1a）評価器であり、実モデル出力を保持するキャッシュ機能を持たないため、対局用キャッシュの実装母体には使用しない。

代わりに`OnnxEvaluator`の前段へ配置する新しい有界キャッシュ（LRUすなわちLeast Recently Used、または時計置換付き固定長表）を設ける。
キャッシュの一致誤認やルール不整合を防ぐため、検索鍵には盤面配列、手番、先獅子捕獲位置`lion_capture_square`、規則コード、およびモデルのSHA-256ハッシュを含める（中将棋には持ち駒がないので鍵に含めない）。

さらに`engine.rs`の`apply_policy_temperature`は方策温度に応じて事前確率を変調するため、キャッシュには温度適用前かつ合法手抽出前のパック済みFP16方策配列および単精度浮動小数点数FP32価値を保存し、`PolicyTemperature`変更時にも安全な契約を保つ。
読み出し時には要求された合法手だけをキャッシュ済み方策配列から抽出したうえで`apply_policy_temperature`を通すことで、事前確率計算と温度適用の独立性を維持する。
あわせてバッチ収集後かつ`OnnxEvaluator`への送信前に同一局面要求を1件へ畳み込むバッチ内重複除去を行う。

### 特徴符号化の分離と二重バッファ非同期オーバーラップ（out-of-order batching）

現在の`engine.rs`は`select_leaf`の収集後に`OnnxEvaluator::evaluate`を呼び出しており、`OnnxEvaluator::evaluate`の内部でホスト側配列への特徴符号化`encode`（`onnx.rs` 367〜369行目）とGPU推論実行`run_binding`を一体で行っている。
したがって、GPUがバッチ`k`を推論している間に次のバッチ`k+1`の特徴符号化をCPU側で完了させるには、符号化処理を`OnnxEvaluator::evaluate`の同期境界から外側へ分離する必要がある。

そのうえで`unsafe_code = "forbid"`制約の下で、`Driver`を専有する探索スレッドと`OnnxEvaluator`を専有する推論スレッドをチャネルおよび再利用可能な二重入力バッファで接続し、推論スレッドがアイドル状態にあるときは最大バッチ幅未満でも早期送信（`lc0`のPR `#1503`に準じた送信方式）を行える2段パイプラインを組む。
二重入力バッファにはピン留め可能な2面の`Vec<u8>`相当スロットを用いる。

並列パイプライン導入にあたっては、全5種の停止理由`StopReason`（`DepthCompleted`、`NodeLimit`、`SoftLimit`、`HardLimit`、および`ExternalStop`）、`Step::Capacity`の扱い、評価器エラー、ワーカースレッド終了、根の勝敗確定、ならびにGPU推論実行中の外部停止割り込みに対する終了契約を厳密に定める。
`Step::Capacity`に到達した際は、有限探索では`NodeLimit`として探索を停止し、無期限探索では外部停止まで待機する。
具体的には、投入済みキューとインフライト中にあるすべての`EvaluationRequest`について、各葉が1回だけ`driver.evaluate`で反映されるか1回だけ`driver.cancel`で仮想損失を取り消されること（重複反映および取り消し漏れゼロ）を構造上保証し、時間とノード数の予算消費は確定評価済みとインフライト中の合計で管理する。
また、現行`engine.rs`の573行目では`assert_finished`が正常終了時のみ呼ばれているが、エラーや早期停止を含む試験検証ではパイプライン回収後に仮想損失の残存がないことも検査対象に加える。

## 実装フェーズ

### フェーズ1: 1手の時間内訳と重複率および衝突頻度の計測基盤

`engine.rs`に1手内の各区間時間（準備、葉収集と`Game`再生、特徴符号化、GPU推論、逆伝播、最善手選択）、実効バッチ幅分布、衝突停止回数、`HardLimit`停止割合、および重複局面割合を記録する軽量統計機能を加え、実戦に近い持ち時間設定での主要負荷を確定する。

### フェーズ2: バッチ内重複排除、有界NNキャッシュ、およびバッチ幅と衝突予算の再設計

`OnnxEvaluator`から独立した温度適用前のパック済み出力を保持する新しい有界NNキャッシュとバッチ内重複除去を統合し、キャッシュ有無およびバッチ幅（16、32、64、128、および256）の性能差を測定する。
あわせて衝突予算とQ中立仮想訪問を切り替え可能な選択肢として実装し、ノード数固定および時間固定双方のSPRTで基準探索との棋力差を確認する。

### フェーズ3: 特徴符号化の分離とGame状態を保つ経路再生の軽量化

`encode`を`OnnxEvaluator::evaluate`から分離し、同時に`driver.rs`の`LeafKind::New`における`Game`再生を浅い枝のキャッシュや差分前進へ置き換えることで、3,200〜数万ノード探索時のCPU時間削減を測定する。

### フェーズ4: 二重バッファ非同期パイプラインと早期送信の統合

チャネルと二重入力バッファを用いた探索スレッドと推論スレッドの並列オーバーラップおよびアイドル時早期送信を実装する。
全5種の`StopReason`、評価エラー、根確定、および外部停止割り込み時の完全回収と取り消しを実装し、`features invariants`下の全不変条件テストを通過させたうえでSPRT対局ゲートにより最終採用を判定する。

## 検証

検証は次の4水準で実施し、すべての変更は性能向上と棋力安全性の双方を満たした項目のみを採用する。

1. `cargo test -p minase --features invariants`を含む既存のMCTS単体およびエンジン結合テストに加え、GPU推論中の外部停止、評価エラー注入、根の早期証明、`Step::Capacity`の有限および無期限分岐、ならびにハッシュ衝突を含む境界試験ですべての`EvaluationRequest`が一回限り完了または`cancel`されることを検証する。
2. 800ノード、3,200ノード、および固定時間100〜500ミリ秒での1手内訳ベンチマークを実行し、バッチ充填率、有効キャッシュヒット率、CPUとGPUの重複率、および実効探索ノード数NPS（Nodes Per Second）の向上を記録する。
3. 短い秒読みおよび切れ負け近傍の持ち時間設定における高速検証対局で時間超過（Time Forfeit）がゼロであることを確かめる。
4. 探索における葉の振り分け、バッチ幅、衝突予算、仮想訪問、キャッシュ、または非同期順序を変える各段階について、変更前の基準バイナリを相手に`minase`自己対局のShort Time Controlの短時間管理STCおよびLong Time Controlの長時間管理LTCで一般化逐次確率比検定GSPRT（Generalized Sequential Probability Ratio Test）またはSPRTを実施する。
非劣性検証の場合は優越差あるいは許容下限マージン`-5`〜`-10` Eloを明確化して判定し、[棋力測定ガイド](../guides/sprt.md)に沿って統計的に非劣性以上を確認して初めて採用する。

## 完了条件

- 対局用PV-MCTSの1手時間内訳、実効バッチ幅分布、および重複率が実測記録として可視化され、バッチ幅再選定、バッチ内重複排除、および有界NNキャッシュの有効性が実測データとSPRT結果で確定している。
- `Game`の裁定および千日手履歴を保った経路再生の軽量化と、`encode`分離後の二重バッファ非同期オーバーラップが`unsafe_code = "forbid"`制約と全終了経路の`invariants`検証を満たして実装されている。
- 採用された全変更の速度比較およびSTCとLTCのSPRT測定結果が`docs/measurements/`に保存され、棋力劣化がないことが証明されている。

## 参考資料

- [GPU計算ボトルネック調査報告](../audits/gpu-computation-bottlenecks-2026-11-28.md)
- [基礎測定記録（2026年11月28日）](../measurements/adhoc/gpu-bottlenecks-20261128.md)
- [Leela Chess Zeroから移せる工夫の提案](../proposals/lc0-transfer-candidates.md)
- [AlphaZero型探索の棋力差の提案](../proposals/alphazero-strength-gap.md)
- [lc0探索調査メモ](../research/lc0-transfer/search.md)
- [棋力差探索調査メモ](../research/alphazero-strength-gap/search.md)
- [PUCTの係数と方策の温度をSPSAで調整する設計書](mcts-puct-spsa.md)
- [仮想損失バッチ付きPUCTによるMCTS探索のADR](../adr/search-mcts-games-by-puct-with-virtual-loss-batches.md)
- [ピン留めメモリのuint8入力とパック済みFP16方策によるZero推論テンソル交換のADR](../adr/exchange-zero-inference-tensors-as-uint8-inputs-and-packed-fp16-policies-in-pinned-memory.md)

# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの設計の正は plans/ 配下の設計書、測定記録は measurements/、教訓は lessons/ であり（docs/ 全体の配置は [README.md](README.md)）、本書は状態表、現在地、次期候補、および横断的な決定だけを保持する。状態表と現在地は追記せず、マイルストーンの完了時または待機理由の変化時に書き直す。

## マイルストーン状態表

| マイルストーン | 設計書 | 状態 | 完了日 |
|---|---|---|---|
| 合法手生成とmake/unmake | [plans/movegen.md](plans/movegen.md) | 完了 | 2026年7月30日 |
| 指し手正準化 | [plans/move-canonicalization.md](plans/move-canonicalization.md) | 完了 | 2026年7月31日 |
| 対局管理層と審判層 | [plans/game-referee.md](plans/game-referee.md) | 完了 | 2026年8月1日 |
| ローカルルール13コードの実装 | [plans/local-rules.md](plans/local-rules.md) | 完了 | 2026年8月2日 |
| 合法手生成器の総仕上げ | [plans/movegen-hardening.md](plans/movegen-hardening.md) | 完了 | 2026年8月3日 |
| 審判ロジック再編とR0撤去 | [plans/adjudication-refactor.md](plans/adjudication-refactor.md) | 完了 | 2026年8月9日 |
| ランダム対局検証ハーネス | [plans/random-play.md](plans/random-play.md) | 完了 | 2026年8月10日 |
| プロトコル層 | [plans/protocol-layer.md](plans/protocol-layer.md) | 完了 | 2026年8月10日 |
| ブラウザGUI向けUSI照会 | [plans/browser-gui.md](plans/browser-gui.md) | 完了 | 2026年8月11日 |
| 対局ハーネスのバイナリ対戦化 | [plans/match-harness.md](plans/match-harness.md) | 完了 | 2026年8月11日 |
| 直前局面生成器 | [plans/predecessor-generator.md](plans/predecessor-generator.md) | 待機中 | ― |
| 探索部 | [plans/search.md](plans/search.md) | 完了 | 2026年8月22日 |
| 外部対局接続 | [plans/engine-connectivity.md](plans/engine-connectivity.md) | 完了 | 2026年8月14日 |
| Lazy SMP | [plans/lazy-smp.md](plans/lazy-smp.md) | 完了 | 2026年8月23日 |
| テストスイートのspec-first再構築 | [plans/spec-first-tests.md](plans/spec-first-tests.md) | 完了 | 2026年8月15日 |
| 評価関数 | [plans/evaluation.md](plans/evaluation.md) | 完了 | 2026年8月26日 |
| 規則集合の代数的データ型化 | [plans/rules-type.md](plans/rules-type.md) | 完了 | 2026年8月25日 |
| Rust設計監査対応 | [plans/rust-design-audit-remediation.md](plans/rust-design-audit-remediation.md) | 完了 | 2026年8月26日 |
| 評価関数の世代反復 | [plans/evaluation-gen1.md](plans/evaluation-gen1.md) | 完了 | 2026年8月29日 |
| 棋力測定ハーネス基盤の効率化 | [plans/match-harness-efficiency.md](plans/match-harness-efficiency.md) | 完了 | 2026年8月28日 |
| 棋力測定の段階ゲート | [plans/match-staged-gate.md](plans/match-staged-gate.md) | 完了 | 2026年9月1日 |
| 早期投了の導入判定 | [plans/match-early-resignation.md](plans/match-early-resignation.md) | 待機中 | ― |
| 棋力向上の段階計画 | [plans/strength-stages.md](plans/strength-stages.md) | 進行中 | ― |
| 棋力向上段階1 | [plans/strength-stage1.md](plans/strength-stage1.md) | 完了 | 2026年9月2日 |
| 棋力向上段階2 | [plans/strength-stage2.md](plans/strength-stage2.md) | 完了 | 2026年9月4日 |
| 棋力測定の所要時間削減 | [plans/match-cost-reduction.md](plans/match-cost-reduction.md) | 完了 | 2026年9月4日 |
| lishogi Bot接続 | [plans/lishogi-bot.md](plans/lishogi-bot.md) | 進行中 | ― |
| 棋力向上段階3 | [plans/strength-stage3.md](plans/strength-stage3.md) | 完了 | 2026年9月5日 |
| 棋力向上段階4 | [plans/strength-stage4.md](plans/strength-stage4.md) | 完了 | 2026年9月6日 |
| 棋力向上段階5 | [plans/strength-stage5.md](plans/strength-stage5.md) | 完了 | 2026年9月7日 |
| PSTの序中盤と終盤の補間 | [plans/tapered-pst.md](plans/tapered-pst.md) | 完了 | 2026年9月8日 |
| HaChu対minaseの条件格子測定 | [plans/hachu-condition-grid.md](plans/hachu-condition-grid.md) | 完了 | 2026年9月9日 |
| Factorization Machineによる2駒関係評価 | [plans/factorization-machine.md](plans/factorization-machine.md) | 完了 | 2026年9月9日 |
| 棋力向上段階6 | [plans/strength-stage6.md](plans/strength-stage6.md) | 完了 | 2026年9月12日 |
| 棋力向上段階7 | [plans/strength-stage7.md](plans/strength-stage7.md) | 完了 | 2026年9月14日 |
| 合法手生成と利き計算の高速化 | [plans/movegen-speedup.md](plans/movegen-speedup.md) | 完了 | 2026年9月15日 |
| 合法手生成と利き計算の高速化（第2期） | [plans/movegen-speedup-2.md](plans/movegen-speedup-2.md) | 進行中 | |

## 現在地

直近に完了したマイルストーンは、2026年9月14日に完了した[棋力向上段階7](plans/strength-stage7.md)である。
鏡映による重み共有と世代2の再学習重みを、個別の短時間・長時間測定で採用した。
再学習には更新後の射影と[片側絶対値型の追加損失](measurements/strength-stage7-removal-absolute.md)を用い、6代表局面の352件の駒除去診断で新しい符号反転が0件となった。
試行で選んだ手数上限4,000手を生成器へ反映し、[世代2の14,057,872局面](measurements/strength-stage7-gen2-generation.md)を次世代でも使える形で保存した。
[最終重みから再導出した駒価値](measurements/strength-stage7-gen2-values-diag.md)は固定値との差が最大8%で更新基準の20%を超えず、固定駒価値と探索の余裕値を維持する。

最終構成の[段階開始版との固定200ペア](measurements/strength-stage7-gen2-elo200.md)は+113.19 Elo、95%信頼区間[+77.64, +151.19]となり、全400局の異常は0件だった。
[HaChuとの固定200ペア](measurements/strength-stage7-hachu-elo200.md)は+391.57 Elo、95%信頼区間[+339.80, +460.03]となった。
HaChu側のクラッシュ1件は履歴配列によるカウンタ上書きの証拠を得たが、不正着手1件は生の応答が保存されておらず原因未特定であり、どちらも規約どおり反則負けとして集計した。
旧構成で中断した段階開始版との37ペアは保持し、最終構成の集計へは含めない。

直近に完了したマイルストーンは[合法手生成と利き計算の高速化](plans/movegen-speedup.md)であり、利き逆引きの片側化、静止探索の段階的な捕獲生成、およびSEEの早期終了でbenchのNPSを基準比1.500倍にした。
PGOはNPS約1.19倍の効果を確認したが、運用の複雑さを理由に利用者の判断で採用しなかった。
[第2期](plans/movegen-speedup-2.md)は2026年9月15日に起案し、第1期の採用版を基準に10段階で1.3倍を目標とする。利用者の決定により`unsafe`は使わず、探索木を変える変更は段階7から段階9に置き、事前選別を通過した段階を1コミットに固定して1組のSTCとLTCで判定する。2026年9月16日に着手し、段階1（計測基盤の更新と`codegen-units = 1`、親比1.0296倍）と段階2（利き線の事前選別、親比1.0492倍）が完了して累積は基準比1.0781倍である。段階3は測り直しで閾値が1未満になり着手せず、段階4（走り計算の減算方式とSEE逆引きの近傍走査、親比1.1205倍）と段階5（静止探索の候補処理の簡素化、親比1.0443倍）で累積は基準比1.2636倍となり最低受入条件を超えた。次の一手は段階6の主探索の手生成と探索ノードの固定費である。起案時の診断は measurements/movegen-speedup-2-diagnostics-depth5.md、段階1の診断は measurements/movegen-speedup-2-stage1-counts.md にある。
進行中のマイルストーンは、上位計画の[棋力向上の段階計画](plans/strength-stages.md)である。
10段階のうち段階7までが完了し、次の段階は採用構成と世代2のデータを起点に進められる。

待機中のマイルストーンは2件である。
直前局面生成器（plans/predecessor-generator.md）は設計済みだが、2026年8月10日に探索部を先行させると決定してから待機している。`Position`のAPI再編を含むため、着手時期は別途決める。順方向の探索部と評価関数はその完了を前提とせず、いつ再開しても手戻りがない。
早期投了の導入判定（plans/match-early-resignation.md）は、仮想投了が3,000回以上発火する検証群を確保できる記録量に達し、統計契約が確定するまで待機する。

次期候補は、棋力向上段階8（利きマップと評価特徴の拡張）であり、段階7の採用構成と世代2のデータを起点にする。
隣接シードで対局が重複する`rng::derive_seed`の修正は利用者の判断を待つ。
`Threads=4`対2の測定は必要になった時点で plans/lazy-smp.md の手順で実施し、進行中の測定には着手時点のハーネスと測定条件を使って段階ゲートを遡及適用しない。
lishogi Bot接続（plans/lishogi-bot.md）は2026年9月12日に着手し、規則R1へlishogiの反復裁定の前提条件を取り込み、Lishogi-Botとminaseを1つのDockerイメージにまとめる配備手段を整えた。
次の一手は、利用者がBotアカウントを作成してイメージを運用機で起動し、非レート対局の公開運用へ進むことである。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

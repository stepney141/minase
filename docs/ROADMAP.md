# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの設計と実施記録の正は plans/ 配下の設計書であり、本書は状態表、現在地、次期マイルストーンの作業項目、および横断的な決定だけを保持する。マイルストーンの完了時に更新する。

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

直前局面生成器は設計済みだが、2026年8月10日の利用者決定により探索部を先行させ、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地：Lazy SMPの完了と次期マイルストーンの起案

Lazy SMPマイルストーンは2026年8月23日に完了した。
初版（同一深さの補助ワーカーとルート回転）は時間制御GSPRTで効果を示さず、診断により全ノードの約98%が置換表を使わない静止探索と深さ0ノードであることが判明した。
第2版では、静止探索での置換表の照会と記録（ノード固定GSPRTでH1、固定深さ5のノード数−53%）、最深ワーカーの結果採用と周期付き深さスキップ（固定深さbenchで`Threads=2`が深さ5で−26.5%、深さ6で−17.9%）を採用し、`time=30000+300,byoyomi=500`の時間制御GSPRTで`Threads=2`対1が500ペア上限でLLR 2.615の判定保留ながら固定局数Elo +62［+41, +83］となり、利用者の決定で採用した（記録と規定からの逸脱の根拠は plans/lazy-smp.md の実施状況）。
共有カウンタのキャッシュライン分離はNPSゲートに達せず不採用とした。
製品の既定ワーカー数は1のままであり、並列探索はUSIの`Threads`とCECPの`cores`で明示した場合だけ有効になる。

次は、静止探索の効率化を中心とする単一ワーカーの探索改良を別マイルストーンとして起案する。
候補は、捕獲専用の手生成（静止探索で全合法手を生成して捕獲を抽出している時間の削減）、静的交換評価による損な捕獲の保守的な枝刈り、深さ1のfutility pruningであり、各候補は単独コミットでGSPRTにかける。
評価関数の本格化はその後、または並行する別マイルストーンとして起案する。
`Threads=4`対2の測定は、必要になった時点で plans/lazy-smp.md の手順で実施する。
実lishogiサーバへの接続は、未完成のエンジンを公開の場へ出さない方針から対象外のままであり、探索・評価の成熟後に別マイルストーンとして計画する。
直前局面生成器は待機中のままとし、`Position`のAPI再編を含むため、着手時期を別途決める。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

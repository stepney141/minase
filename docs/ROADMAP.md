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
| 評価関数 | [plans/evaluation.md](plans/evaluation.md) | 起案済み | ― |

直前局面生成器は設計済みだが、2026年8月10日に探索部を先行させると決定し、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地：評価関数マイルストーンの起案

Lazy SMPマイルストーンは2026年8月23日に完了した（記録は plans/lazy-smp.md）。
製品の既定ワーカー数は1のままであり、並列探索はUSIの`Threads`とCECPの`cores`で明示した場合だけ有効になる。

2026年8月24日のHaChu戦の要因分析（plans/match-harness.md）は、等時間で−252 Eloの差の主因を、探索量ではなく1ノードあたりの評価知識と位置づけた。
minaseは探索量でHaChuに劣らず、持ち時間を4倍にすると統計的に互角へ達したためである。
この結果を受け、Lazy SMP完了時に第1候補としていた静止探索の効率化より評価関数を優先し、2026年8月25日に評価関数マイルストーン（plans/evaluation.md）を起案した。
方式は、自己対局データから学習する駒状態×升のP型NNUEを主とし、線形の学習PSTでパイプラインを先に検証する。完了基準は、最終採用ネットが着手前の評価関数v0の最終コミットに時間制御GSPRTで有意に勝ち越すことであり、HaChuとの成績は参考値として記録する。世代反復と相対王駒特徴は、P型の実測後に別マイルストーンとして起案する。

静止探索の効率化（捕獲専用の手生成、SEE、futility pruning）は評価関数と独立に採否できるため、評価関数の後に別マイルストーンとして起案する。
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

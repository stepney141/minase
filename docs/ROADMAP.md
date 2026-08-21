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
| Lazy SMP | [plans/lazy-smp.md](plans/lazy-smp.md) | 進行中 | ― |
| テストスイートのspec-first再構築 | [plans/spec-first-tests.md](plans/spec-first-tests.md) | 完了 | 2026年8月15日 |

直前局面生成器は設計済みだが、2026年8月10日の利用者決定により探索部を先行させ、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地：Lazy SMPの着手

探索部マイルストーンは2026年8月22日に完了した。
評価関数v0、反復深化αβ＋PVS、置換表、静止探索（delta pruning付き）、killer・history順序付け、null move pruning、late move reductions、soft/hard二段リミットの時間管理、探索呼び出し境界からなる単一ワーカー探索が揃い、各改良はコミット対コミットのGSPRTで採用された（記録は plans/search.md の実施状況）。
完了基準ゲート（凍結ベースライン0045833のdepth=1に対するdepth=4の対局）は207ペア全勝でH1、時間制御つき自己対局は時間切れなしに完走した。
保留だったmimalloc採否と合法手生成器のVec割当改善も、benchのNPS実測に基づき両方採用して決着した。
aspiration windowsとcheck extensionは、測定条件で発動しないこと、および延長条件の設計コストを理由に見送り、設計だけを残した。

Lazy SMPの着手条件「単一ワーカー探索の完成」と「CECPの対局進行」は両方満たされた。
次は plans/lazy-smp.md の設計に従い、共有置換表、探索チーム、共通の停止とノード予算、USIの`Threads`、CECPの`cores`を5フェーズで実装し、時間制御のコミット対コミットGSPRTで`Threads=2`対1を採否判定する。
評価関数の本格化はLazy SMPの採否完了後に別マイルストーンとして起案する。
実lishogiサーバへの接続は、未完成のエンジンを公開の場へ出さない方針から対象外のままであり、探索・評価の成熟後に別マイルストーンとして計画する。
直前局面生成器は待機中のままとし、`Position`のAPI再編を含むため、Lazy SMPの進み具合による衝突コストを見て着手時期を別途決める。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

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
| 直前局面生成器 | [plans/predecessor-generator.md](plans/predecessor-generator.md) | 完了 | 2026年9月18日 |
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
| 合法手生成と利き計算の高速化（第2期） | [plans/movegen-speedup-2.md](plans/movegen-speedup-2.md) | 完了 | 2026年9月17日 |
| 合法手生成と利き計算の高速化（第3期） | [plans/movegen-speedup-3.md](plans/movegen-speedup-3.md) | 起案 | |
| 棋力向上段階8（前向き枝刈りの第3層） | [plans/strength-stage8.md](plans/strength-stage8.md) | 完了 | 2026年9月21日 |
| 持ち時間の効率的な使用（issue #7） | [plans/time-management-efficiency.md](plans/time-management-efficiency.md) | 完了 | 2026年9月18日 |
| USI先読み（ponder） | [plans/ponder.md](plans/ponder.md) | 完了 | 2026年9月21日 |
| 棋力向上段階9（利きの土台、教師データの改善、王の安全度と利きに基づく評価特徴） | [plans/strength-stage9.md](plans/strength-stage9.md) | 進行中 | ― |
| SPSAによる探索係数と時間管理係数の調整 | [plans/spsa.md](plans/spsa.md) | 進行中 | ― |
| 先読み教師値（将来の探索値の幾何加重平均） | [plans/lookahead-teacher.md](plans/lookahead-teacher.md) | 完了 | 2026年9月22日 |

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

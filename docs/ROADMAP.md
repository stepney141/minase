# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの計画は plans/ 配下の設計書（完了時に凍結する）、仕様の正はコードとそのdocコメント、設計判断は [adr/](adr/README.md)、測定記録は measurements/、教訓は lessons/ であり（docs/ 全体の配置は [README.md](README.md)）、本書は状態表、現在地、および次期候補だけを保持する。状態表と現在地は追記せず、マイルストーンの完了時または待機理由の変化時に書き直す。
「直近の完了」には完了日の新しい順に3件だけを置き、件数の規則と削除する項目の扱いは [plans/README.md](plans/README.md#上書きの規則) が定める。

## マイルストーン状態表

| マイルストーン | 設計書 | 状態 | 完了日 |
|---|---|---|---|
| 合法手生成とmake/unmake | [plans/archived/movegen.md](plans/archived/movegen.md) | 完了（採用） | 2026年7月30日 |
| 指し手正準化 | [plans/archived/move-canonicalization.md](plans/archived/move-canonicalization.md) | 完了（採用） | 2026年7月31日 |
| 対局管理層と審判層 | [plans/archived/game-referee.md](plans/archived/game-referee.md) | 完了（採用） | 2026年8月1日 |
| ローカルルール13コードの実装 | [plans/archived/local-rules.md](plans/archived/local-rules.md) | 完了（採用） | 2026年8月2日 |
| 合法手生成器の総仕上げ | [plans/archived/movegen-hardening.md](plans/archived/movegen-hardening.md) | 完了（採用） | 2026年8月3日 |
| 審判ロジック再編とR0撤去 | [plans/archived/adjudication-refactor.md](plans/archived/adjudication-refactor.md) | 完了（採用） | 2026年8月9日 |
| ランダム対局検証ハーネス | [plans/archived/random-play.md](plans/archived/random-play.md) | 完了（採用） | 2026年8月10日 |
| プロトコル層 | [plans/archived/protocol-layer.md](plans/archived/protocol-layer.md) | 完了（採用） | 2026年8月10日 |
| src構成の整理 | [plans/archived/src-layout.md](plans/archived/src-layout.md) | 完了（採用） | 2026年8月10日 |
| ブラウザGUI向けUSI照会 | [plans/archived/browser-gui.md](plans/archived/browser-gui.md) | 完了（採用） | 2026年8月11日 |
| 対局ハーネスのバイナリ対戦化 | [plans/archived/match-harness.md](plans/archived/match-harness.md) | 完了（採用） | 2026年8月11日 |
| 直前局面生成器 | [plans/archived/predecessor-generator.md](plans/archived/predecessor-generator.md) | 完了（採用） | 2026年9月18日 |
| 探索部 | [plans/archived/search.md](plans/archived/search.md) | 完了（一部不採用） | 2026年8月22日 |
| 外部対局接続 | [plans/archived/engine-connectivity.md](plans/archived/engine-connectivity.md) | 完了（採用） | 2026年8月14日 |
| Lazy SMP | [plans/archived/lazy-smp.md](plans/archived/lazy-smp.md) | 完了（一部不採用） | 2026年8月23日 |
| テストスイートのspec-first再構築 | [plans/archived/spec-first-tests.md](plans/archived/spec-first-tests.md) | 完了（採用） | 2026年8月15日 |
| 評価関数 | [plans/archived/evaluation.md](plans/archived/evaluation.md) | 完了（一部不採用） | 2026年8月26日 |
| 規則集合の代数的データ型化 | [plans/archived/rules-type.md](plans/archived/rules-type.md) | 完了（採用） | 2026年8月25日 |
| Rust設計監査対応 | [plans/archived/rust-design-audit-remediation.md](plans/archived/rust-design-audit-remediation.md) | 完了（採用） | 2026年8月26日 |
| 評価関数の世代反復 | [plans/archived/evaluation-gen1.md](plans/archived/evaluation-gen1.md) | 完了（一部不採用） | 2026年8月29日 |
| 棋力測定ハーネス基盤の効率化 | [plans/archived/match-harness-efficiency.md](plans/archived/match-harness-efficiency.md) | 完了（採用） | 2026年8月28日 |
| 棋力測定の段階ゲート | [plans/archived/match-staged-gate.md](plans/archived/match-staged-gate.md) | 完了（採用） | 2026年9月1日 |
| 早期投了の導入判定 | [plans/match-early-resignation.md](plans/match-early-resignation.md) | 待機中 | ― |
| 棋力向上の段階計画 | [plans/strength-stages.md](plans/strength-stages.md) | 進行中 | ― |
| 棋力向上段階1 | [plans/archived/strength-stage1.md](plans/archived/strength-stage1.md) | 完了（一部不採用） | 2026年9月2日 |
| 棋力向上段階2 | [plans/archived/strength-stage2.md](plans/archived/strength-stage2.md) | 完了（採用） | 2026年9月4日 |
| 棋力測定の所要時間削減 | [plans/archived/match-cost-reduction.md](plans/archived/match-cost-reduction.md) | 完了（採用） | 2026年9月4日 |
| lishogi Bot接続 | [plans/lishogi-bot.md](plans/lishogi-bot.md) | 進行中 | ― |
| 棋力向上段階3 | [plans/archived/strength-stage3.md](plans/archived/strength-stage3.md) | 完了（一部不採用） | 2026年9月5日 |
| 棋力向上段階4 | [plans/archived/strength-stage4.md](plans/archived/strength-stage4.md) | 完了（一部不採用） | 2026年9月6日 |
| 棋力向上段階5 | [plans/archived/strength-stage5.md](plans/archived/strength-stage5.md) | 完了（一部不採用） | 2026年9月7日 |
| PSTの序中盤と終盤の補間 | [plans/archived/tapered-pst.md](plans/archived/tapered-pst.md) | 完了（採用） | 2026年9月8日 |
| HaChu対minaseの条件格子測定 | [plans/archived/hachu-condition-grid.md](plans/archived/hachu-condition-grid.md) | 完了（採用） | 2026年9月9日 |
| Factorization Machineによる2駒関係評価 | [plans/archived/factorization-machine.md](plans/archived/factorization-machine.md) | 完了（不採用） | 2026年9月9日 |
| 棋力向上段階6 | [plans/archived/strength-stage6.md](plans/archived/strength-stage6.md) | 完了（一部不採用） | 2026年9月12日 |
| 棋力向上段階7 | [plans/archived/strength-stage7.md](plans/archived/strength-stage7.md) | 完了（一部不採用） | 2026年9月14日 |
| 合法手生成と利き計算の高速化 | [plans/archived/movegen-speedup.md](plans/archived/movegen-speedup.md) | 完了（一部不採用） | 2026年9月15日 |
| 合法手生成と利き計算の高速化（第2期） | [plans/archived/movegen-speedup-2.md](plans/archived/movegen-speedup-2.md) | 完了（一部不採用） | 2026年9月17日 |
| 合法手生成と利き計算の高速化（第3期） | [plans/movegen-speedup-3.md](plans/movegen-speedup-3.md) | 起案 | |
| 棋力向上段階8（前向き枝刈りの第3層） | [plans/archived/strength-stage8.md](plans/archived/strength-stage8.md) | 完了（一部不採用） | 2026年9月21日 |
| 持ち時間の効率的な使用（issue #7） | [plans/archived/time-management-efficiency.md](plans/archived/time-management-efficiency.md) | 完了（一部不採用） | 2026年9月18日 |
| USI先読み（ponder） | [plans/archived/ponder.md](plans/archived/ponder.md) | 完了（採用） | 2026年9月21日 |
| 棋力向上段階9（利きの土台、教師データの改善、王の安全度と利きに基づく評価特徴） | [plans/archived/strength-stage9.md](plans/archived/strength-stage9.md) | 完了（一部不採用） | 2026年9月23日 |
| SPSAによる探索係数と時間管理係数の調整 | [plans/archived/spsa.md](plans/archived/spsa.md) | 完了（採用） | 2026年9月25日 |
| 先読み教師値（将来の探索値の幾何加重平均） | [plans/archived/lookahead-teacher.md](plans/archived/lookahead-teacher.md) | 完了（一部不採用） | 2026年9月23日 |
| SPSAの調整結果をソースへ反映するコマンド | [plans/archived/spsa-apply.md](plans/archived/spsa-apply.md) | 完了（採用） | 2026年9月22日 |
| 探索部の反復負け回避と反復禁止 | [plans/archived/search-repetition.md](plans/archived/search-repetition.md) | 完了（採用） | 2026年10月15日 |
| 不採用だった探索部の改良のSPSAによる再調整（issue #10） | [plans/archived/search-revival-spsa.md](plans/archived/search-revival-spsa.md) | 完了（採用） | 2026年10月4日 |
| AlphaZero型探索と深層強化学習の統括 | [plans/alphazero.md](plans/alphazero.md) | 進行中 | |
| AlphaZero型探索の実行環境と実現性の実測 | [plans/archived/alphazero-environment.md](plans/archived/alphazero-environment.md) | 完了（採用） | 2026年10月27日 |
| AlphaZero型探索のネットの入出力と推論 | [plans/archived/alphazero-network.md](plans/archived/alphazero-network.md) | 完了（採用） | 2026年10月27日 |
| 探索方式の共存とMCTS | [plans/archived/alphazero-mcts.md](plans/archived/alphazero-mcts.md) | 完了（採用） | 2026年10月27日 |
| AlphaZero型探索の自己対局生成器 | [plans/archived/alphazero-selfplay.md](plans/archived/alphazero-selfplay.md) | 完了（採用） | 2026年10月27日 |
| AlphaZero型探索の学習器 | [plans/archived/alphazero-trainer.md](plans/archived/alphazero-trainer.md) | 完了（採用） | 2026年10月28日 |
| AlphaZero型探索の強化学習の実行 | [plans/alphazero-rl.md](plans/alphazero-rl.md) | 進行中 | |
| AlphaZero型探索の監査対応 | [plans/archived/alphazero-audit-remediation.md](plans/archived/alphazero-audit-remediation.md) | 完了（採用） | 2026年10月28日 |
| AlphaZero型探索の強化学習の再出発に向けた学習技法の導入 | [plans/archived/alphazero-restart-techniques.md](plans/archived/alphazero-restart-techniques.md) | 完了（採用） | 2026年10月31日 |
| 棋力向上段階12（探索の小改良と表の寿命） | [plans/archived/strength-stage12.md](plans/archived/strength-stage12.md) | 完了（不採用） | 2026年9月28日 |
| USI投了（issue #6） | [plans/archived/usi-resignation.md](plans/archived/usi-resignation.md) | 完了（採用） | 2026年9月25日 |
| SPSAの摂動幅と学習率の較正 | [plans/archived/spsa-gain-calibration.md](plans/archived/spsa-gain-calibration.md) | 完了（採用） | 2026年9月26日 |
| 教師の混合比λ=1.0（探索値だけの教師） | [plans/archived/teacher-mixing-ratio.md](plans/archived/teacher-mixing-ratio.md) | 完了（不採用） | 2026年9月26日 |
| 探索局面を用いた評価関数の学習 | [plans/archived/search-aware-evaluation.md](plans/archived/search-aware-evaluation.md) | 完了（不採用） | 2026年9月26日 |
| 相対位置の局所2駒関係による評価の補正 | [plans/archived/relative-pair-eval.md](plans/archived/relative-pair-eval.md) | 完了（一部不採用） | 2026年9月26日 |
| Athénanによる評価関数の強化学習と対局探索 | [plans/athenan.md](plans/athenan.md) | 起案 | |
| 静止探索の出力を学ぶPSTの学習 | [plans/archived/qsearch-output-training.md](plans/archived/qsearch-output-training.md) | 完了（不採用） | 2026年9月30日 |
| 浅い探索の手の順位を学ぶ損失 | [plans/archived/rank-loss-training.md](plans/archived/rank-loss-training.md) | 完了（不採用） | 2026年10月1日 |
| 残存誤りに対応する関係補正項 | [plans/archived/relational-correction.md](plans/archived/relational-correction.md) | 完了（不採用） | 2026年10月2日 |
| デバッグ機能の整備 | [plans/archived/debugging-tools.md](plans/archived/debugging-tools.md) | 完了（採用） | 2026年9月27日 |
| 探索部の不具合修正 | [plans/archived/search-bug-fixes.md](plans/archived/search-bug-fixes.md) | 完了（一部不採用） | 2026年9月29日 |
| 評価関数の候補と探索係数の共適応の検証 | [plans/archived/eval-search-coadaptation.md](plans/archived/eval-search-coadaptation.md) | 完了（一部不採用） | 2026年10月5日 |
| 補正1/4のFMの現行PSTへの再学習 | [plans/archived/fm-quarter-current-pst.md](plans/archived/fm-quarter-current-pst.md) | 完了（一部不採用） | 2026年10月7日 |
| 採用PSTの追加学習のエポック数の延長 | [plans/archived/pst-longer-training.md](plans/archived/pst-longer-training.md) | 完了（採用） | 2026年10月6日 |
| 世代3の教師データによるPSTの再学習とデータ半分割の診断 | [plans/archived/pst-gen3.md](plans/archived/pst-gen3.md) | 完了（採用） | 2026年10月3日 |
| 探索部と評価関数のモジュール再編 | [plans/archived/search-eval-layout.md](plans/archived/search-eval-layout.md) | 完了（採用） | 2026年9月26日 |
| 中核モジュールの再編 | [plans/archived/core-layout.md](plans/archived/core-layout.md) | 完了（採用） | 2026年9月27日 |
| 実行ファイルと対局ハーネスのモジュール再編 | [plans/archived/bin-harness-layout.md](plans/archived/bin-harness-layout.md) | 完了（採用） | 2026年9月27日 |
| 学習ツールの再編 | [plans/archived/train-tools-layout.md](plans/archived/train-tools-layout.md) | 完了（採用） | 2026年10月3日 |
| 勝率尺度の1定数化と教師の尺度補正係数の明示（Kの改名） | [plans/archived/win-rate-scale.md](plans/archived/win-rate-scale.md) | 完了（採用） | 2026年10月16日 |
| ライブラリとエンジンのcrate分割 | [plans/archived/crate-split.md](plans/archived/crate-split.md) | 完了（採用） | 2026年10月4日 |
| 補助ツールのサブコマンド化 | [plans/archived/cli-subcommands.md](plans/archived/cli-subcommands.md) | 完了（採用） | 2026年10月5日 |
| 測定の再開条件の簡素化 | [plans/archived/match-resume-simplification.md](plans/archived/match-resume-simplification.md) | 完了（採用） | 2026年10月5日 |
| PSTとFMの同時学習 | [plans/archived/pst-fm-joint-training.md](plans/archived/pst-fm-joint-training.md) | 完了（不採用） | 2026年10月8日 |
| FMの補正の倍率のSPSAによる調整 | [plans/archived/fm-scale-spsa.md](plans/archived/fm-scale-spsa.md) | 完了（採用） | 2026年10月8日 |
| 秒読みつき時計での持ち時間と秒読みの活用 | [plans/archived/byoyomi-time-usage.md](plans/archived/byoyomi-time-usage.md) | 完了（一部不採用） | 2026年10月23日 |
| NNUEの共通土台と初代のネットによるPST＋FMの完全代替 | [plans/archived/nnue-replacement.md](plans/archived/nnue-replacement.md) | 完了（採用） | 2026年10月12日 |
| 手の順序付けの改善 | [plans/archived/move-ordering.md](plans/archived/move-ordering.md) | 完了（採用） | 2026年10月10日 |
| 詰み探索（df-pn） | [plans/mate-search.md](plans/mate-search.md) | 起案 | |
| Rust製NNUE学習器（GPU） | [plans/archived/nnue-trainer-rust.md](plans/archived/nnue-trainer-rust.md) | 完了（採用） | 2026年10月12日 |
| NNUE学習器の学習率スケジュール | [plans/archived/nnue-lr-schedule.md](plans/archived/nnue-lr-schedule.md) | 完了（採用） | 2026年10月16日 |
| NNUEの隠れ層と位相補間の出力 | [plans/archived/nnue-hidden-layer.md](plans/archived/nnue-hidden-layer.md) | 完了（一部不採用） | 2026年10月16日 |
| Python学習器のRustへの移植と撤去 | [plans/archived/python-trainer-retirement.md](plans/archived/python-trainer-retirement.md) | 完了（採用） | 2026年10月13日 |
| ファイル形式の整理と形式の仕様の整備 | [plans/archived/file-format-consolidation.md](plans/archived/file-format-consolidation.md) | 完了（採用） | 2026年10月13日 |
| 教師データ形式の統一 | [plans/archived/unified-teacher-format.md](plans/archived/unified-teacher-format.md) | 完了（採用） | 2026年10月14日 |
| メモリより大きい教師データの一様シャッフル | [plans/archived/out-of-core-shuffle.md](plans/archived/out-of-core-shuffle.md) | 完了（採用） | 2026年10月17日 |
| 引き分けの評価値の設定（DrawValue） | [plans/archived/draw-value.md](plans/archived/draw-value.md) | 完了（採用） | 2026年10月23日 |
| vast.aiでの教師生成 | [plans/vast-selfplay.md](plans/vast-selfplay.md) | 起案 | |
| 探索の関数形の見直しの統括 | [plans/archived/search-forms.md](plans/archived/search-forms.md) | 完了（採用） | 2026年10月23日 |
| 探索の枝刈りとLMRの関数形の見直し | [plans/archived/search-pruning-forms.md](plans/archived/search-pruning-forms.md) | 完了（採用） | 2026年10月18日 |
| 浅い前向き枝刈りの深さと対象の拡張 | [plans/archived/shallow-pruning-extensions.md](plans/archived/shallow-pruning-extensions.md) | 完了（採用） | 2026年10月19日 |
| historyの更新量の形とcontinuation history | [plans/archived/history-forms.md](plans/archived/history-forms.md) | 完了（採用） | 2026年10月20日 |
| correction historyの表の追加と持ち越し | [plans/archived/correction-history-tables.md](plans/archived/correction-history-tables.md) | 完了（採用） | 2026年10月21日 |
| 静止探索の補正と枝刈りの形の見直し | [plans/archived/qsearch-forms.md](plans/archived/qsearch-forms.md) | 完了（採用） | 2026年10月21日 |
| aspiration windowsの幅と窓の移し方の見直し | [plans/archived/aspiration-window-forms.md](plans/archived/aspiration-window-forms.md) | 完了（採用） | 2026年10月22日 |
| 探索の不安定さに応じた持ち時間の伸縮 | [plans/archived/time-instability-scaling.md](plans/archived/time-instability-scaling.md) | 完了（採用） | 2026年10月23日 |
| NNUEの検証損失が早く頭打ちになる原因の切り分け | [plans/archived/nnue-early-best-epoch.md](plans/archived/nnue-early-best-epoch.md) | 完了（採用） | 2026年10月17日 |
| 先読みなしの教師での対局単位と局面単位の間引きの比較 | [plans/nnue-subsample-no-lookahead.md](plans/nnue-subsample-no-lookahead.md) | 起案 | |
| 教師生成で記録する局面を間引き、記録しない手を少ないノード数で指す | [plans/sparse-teacher-recording.md](plans/sparse-teacher-recording.md) | 起案 | |
| 第1層の全再計算の差分化（Finny tables） | [plans/nnue-finny-tables.md](plans/nnue-finny-tables.md) | 待機中 | |
| 測定記録の再配置 | [plans/archived/measurements-layout.md](plans/archived/measurements-layout.md) | 完了（採用） | 2026年10月18日 |
| AlphaZero型探索のコードの配置と試験専用コードの整理 | [plans/archived/zero-code-layout.md](plans/archived/zero-code-layout.md) | 完了（採用） | 2026年11月2日 |
| 固定時間（movetime）の探索の締切と途中結果 | [plans/archived/movetime-deadline.md](plans/archived/movetime-deadline.md) | 完了（採用） | 2026年10月26日 |
| NNUEの線形経路を升によらない駒価値で初期化する学習 | [plans/archived/nnue-material-linear-init.md](plans/archived/nnue-material-linear-init.md) | 完了（採用） | 2026年10月26日 |
| 審判層の裁定が繰り返す合法手生成の削減 | [plans/archived/referee-adjudication-movegen.md](plans/archived/referee-adjudication-movegen.md) | 完了（採用） | 2026年10月28日 |
| NNUEの線形の経路の対局結果への回帰 | [plans/archived/nnue-outcome-regression.md](plans/archived/nnue-outcome-regression.md) | 完了（採用） | 2026年11月3日 |
| NNUEの教師局面を静止探索の末端へ置き換える比較 | [plans/archived/nnue-qsearch-leaf.md](plans/archived/nnue-qsearch-leaf.md) | 完了（不採用） | 2026年11月4日 |
| NNUEの世代反復をv0からやり直す | [plans/nnue-generation-reset.md](plans/nnue-generation-reset.md) | 進行中 | |
| NNUEに隠れ層を加えて第1層を広げ、整数化の精度を上げる | [plans/archived/nnue-architecture-revision.md](plans/archived/nnue-architecture-revision.md) | 完了（一部不採用） | 2026年11月4日 |
| NNUE教師生成を探索結果を変えずに速くする | [plans/teacher-generation-speedup.md](plans/teacher-generation-speedup.md) | 進行中 | |

## 現在地

### 直近の完了

- [NNUEの構成の見直し](plans/archived/nnue-architecture-revision.md)は2026年11月4日に、一部不採用で完了した。幅256の第1層、要素積、2段の隠れ層、skip接続、および16ビットの累算値を、同じ教師で学んだ幅128のネットに`H1`を取らないまま世代反復で使う構成として確定した（[ADR](adr/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-pairwise-products-with-two-hidden-layers.md)）。[STC](measurements/nnue-architecture-revision/nnue-architecture-revision-init-stc.md)は`H0`（得点率43.6%）、[固定ノードの対局](measurements/nnue-architecture-revision/nnue-architecture-revision-init-fixed-nodes.md)は−2.6 Elo（95%区間−35.8〜+30.5）、探索速度は約0.91倍である。隠れ層の重みをi8からi16へ改めると、量子化誤差は29.80 cpから9.45 cpへ下がった（[記録](measurements/nnue-architecture-revision/nnue-architecture-revision-training.md)）。開発者の決定により世代反復の起点としてmasterへ統合したので、masterのエンジンは統合前より弱いと見込まれる。統合前のコミットはブランチ`nnue-width128-regressed`に残し、世代反復の最終候補の比較の基準にする。最終候補が決まるまで、masterからのリリースとBotの更新ではエンジンが弱くなる。
- [NNUEの教師局面を静止探索の末端へ置き換える比較](plans/archived/nnue-qsearch-leaf.md)は2026年11月4日に、不採用で完了した。やねうら王の`qsearch_psv`と同じ置き換え（qsearch leaf replacement）で学んだネットは、補正前のネットに対するSTCで`H0`だった（406有効ペア、得点率45.7%、[記録](measurements/nnue-qsearch-leaf/nnue-qsearch-leaf-stc.md)、[ADR](adr/do-not-replace-nnue-teacher-positions-with-qsearch-leaves.md)）。結論はP型の幅128での結果であり、世代反復の教師には入れない。
- [NNUEの線形の経路の対局結果への回帰](plans/archived/nnue-outcome-regression.md)は2026年11月3日に、採用で完了した。学習済みのネットの第1層と出力部を固定し、線形の経路の序中盤の列を教師の対局結果へのロジスティック回帰（Texel tuningと同種）で補正した（[ADR](adr/correct-the-nnue-linear-path-by-regressing-on-game-outcomes.md)）。判定の対局の予測の損失は0.0071下がり（[記録](measurements/nnue-outcome-regression/nnue-outcome-regression-fit.md)）、配置の距離は0.4602から0.4332へ下がった（[記録](measurements/nnue-outcome-regression/nnue-outcome-regression-placement-stc.md)）。20〜99手に奔王が1〜2段にいる割合は86.0%から14.6%へ、麒麟が右端2筋にいる割合は39.9%から1.2%へ減ったが、仲人、銀将、王将、および金将の配置の距離は増えた。[STC](measurements/nnue-outcome-regression/nnue-outcome-regression-stc.md)は117ペア、[LTC](measurements/nnue-outcome-regression/nnue-outcome-regression-ltc.md)は122ペアでともに`H1`となった。LTCでは機械の一時的な停止による時間切れが1件あり、該当ペアを除いたLLRも判定境界を超える。

### 次の候補

以下の計画に着手するかどうかと実施の順序は、いずれも利用者の指示で決める。
方式と測定の条件は各設計書にあるので、ここには位置づけと次の判断だけを書く。

#### 評価関数（NNUE）

- [NNUEの世代反復をv0からやり直す計画](plans/nnue-generation-reset.md)は、2026年11月4日に着手した進行中の計画であり、構成の見直しで確定した構成を使う。作業はブランチ`nnue-generation-reset`で行う。手書きの評価関数v0の教師からλ=0.75で世代反復をやり直し、後付けの補正なしで偏りの少ないネットを学べるかを確かめる。[線形の経路の結果回帰](plans/archived/nnue-outcome-regression.md)が学習の後に補正を足す2段階の手順であることを受けた計画であり、2026年11月3日の利用者の決定により、AlphaZero型の強化学習の再開より先に行う。教師はvast.aiで借りた1台の計算機で生成する。2026年11月4日の利用者の決定により、反復は第4世代までとし、第1世代の本がそろった時点で計算機を返して、第1世代の判定の後に第2世代以降の進め方を決める。2026年11月5日に計算機が障害で使えなくなったので、同じ日の利用者の決定により、第1世代はそれまでに届いた約6,566万局面で学習した。第1世代のネットはλ=0.75で第0世代に`H0`、同じ教師をλ=1.0で学習した比較のネットは`H1`であり（[記録](measurements/nnue-generation-reset/nnue-generation-reset-gen1-stc.md)）、λ=1.0のネットに`fit-linear`を足すとさらに`H1`を取ったので（[記録](measurements/nnue-generation-reset/nnue-generation-reset-gen1-review.md)）、反復を止めて手順を見直している。
- [tataraの調査](research/tatara-nnue-transfer.md)の残りの候補（出力bucket、損失の形など）は、設計書がまだなく、世代反復の結果を見て起案する。採用条件は、棋力が向上し、配置の偏りが悪化しないことである（2026年11月3日の開発者の決定）。
- [第1層の全再計算の差分化（Finny tables）](plans/nnue-finny-tables.md)は待機中である。現行のP型では探索中に全再計算が起きないので、自玉の位置で条件付けるネットを対局で測るか、累算器の更新方式を変えた時点で再開する。

#### 教師生成

強くする手段に人間の棋譜は使わない（[ADR](adr/do-not-use-human-games-to-strengthen-the-engine.md)）。

- [NNUE教師生成を探索結果を変えずに速くする計画](plans/teacher-generation-speedup.md)は、2026年11月4日に着手した進行中の計画であり、作業はブランチ`teacher-speedup`で行う。教師ファイルがバイト単位で一致する変更だけで、生成機の代理環境の探索速度が基準版の1.580倍になった（[測定](measurements/teacher-generation-speedup/teacher-generation-speedup-engine.md)）。次の判断は、生成機での比較と段階ゲートの時期である。
- [先読みなしの教師での間引きの比較](plans/nnue-subsample-no-lookahead.md)と[記録する局面の間引き](plans/sparse-teacher-recording.md)は、同じ計算時間でより多くの対局から教師を作る一連の計画であり、前者の判定が後者の着手の前提になる。どちらも現行の探索値だけの教師を前提に書かれているので、世代反復の結果を見て続けるかを決める。
- [vast.aiでの教師生成](plans/vast-selfplay.md)は、教師の生成をvast.aiで借りる計算機へ移す計画である。試走では30ファイルを約5.5時間、約5.6米ドルと見込み（[記録](measurements/vast-selfplay/vast-selfplay-trial.md)）、作業機で同じ条件を測るフェーズ0の作業が残っている。世代反復は同書の選定条件と検査の方式を使うが、世代をまたいで同じ計算機を使うので専用のスクリプトで生成し、同書のフェーズ0の残りとフェーズ1を待たない。
- AlphaZero型のネットの評価値で教師の値を付け直す案は、[強豪エンジンの教師生成の調査](research/teacher-generation-strong-engines.md)を受けて、AlphaZero型の計画の後に起案する（2026年10月25日の開発者の決定）。tanuki-はこの方法でレーティング差R+104.3を得ている。付け直しに使う前に、[線形経路の計画](plans/archived/nnue-material-linear-init.md)の麒麟の診断で、そのネットに偏りがないことを確かめる。
- 設計書のない着想は調査メモにある。対局途中のランダム化と評価が食い違う局面の採取は[教師生成の調査](research/teacher-generation-strong-engines.md)に、教師の予算の重点配分は[評価関数を改善する3つの観点](research/evaluation-improvement-strategy.md)に、別系統の局面を開始局面に加える案は[偏りの先行事例](research/self-play-bias-prior-art.md)にある。

#### AlphaZero型探索

- [統括計画](plans/alphazero.md)は、ゼロから強化学習で育てたネットのMCTSを、αβ探索とNNUEの組合せと同等以上に強くすることを目標とする。6本のサブタスクのうち5本を完了し、残る[強化学習の実行](plans/alphazero-rl.md)はサイクル32で止めている（再出発の前に[学習技法の導入](plans/archived/alphazero-restart-techniques.md)を済ませた）。[コードの配置の整理](plans/archived/zero-code-layout.md)の後なので、次のセッションを起動する前に、起動する作業ツリーで`cargo build --release --bin minase`を実行する。c0032はSTCでαβ探索に2勝192敗であり、再開の前に探索回数とネットの拡大を検討する（[調査](research/alphazero-strength-gap.md)）。再開は、2026年11月3日の利用者の決定により、[NNUEの世代反復のやり直し](plans/nnue-generation-reset.md)の後とする。

#### 探索

- [棋力向上の段階計画](plans/strength-stages.md)は、段階11（並列度、終盤処理）が未着手である。
- [詰み探索（df-pn）](plans/mate-search.md)は、攻め方の手を王手に限る標準df-pnのソルバを、`go mate`と対局探索の両方から呼べるライブラリとして作る。αβ探索とPV-MCTSへの組み込みは、発動率を測ってから別の設計書で起案する。

#### 合法手生成と対局ハーネス

- [合法手生成と利き計算の高速化（第3期）](plans/movegen-speedup-3.md)は、起案のまま着手していない。
- [早期投了の導入判定](plans/match-early-resignation.md)は待機中である。校正群とは別に、仮想投了が3,000回以上発火する検証群を確保できる記録量に達しておらず、統計契約も確定していない。

#### 配備

- [lishogi Bot接続](plans/lishogi-bot.md)は公開運用中であり、完了条件の達成状況の確認と配備方式の記述の見直しが残っている。Botはcrates.ioのminase 2.5.0を固定しているので、結果回帰の補正ネットを配備するには[リリース](guides/release.md)が要る。

#### 前提の見直しが要る計画

- [Athénanの計画](plans/athenan.md)は、PSTを既定の評価関数とする前提で書かれており、2026年10月12日にNNUEへ置き換わった後の扱いが決まっていない。対局用の探索UBFMsは評価関数によらない試みなので計画は残し（2026年11月3日の利用者の判断）、着手の前に前提を見直す。

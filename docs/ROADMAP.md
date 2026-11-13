# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの計画は plans/ 配下の設計書（完了時に凍結する）、仕様の正はコードとそのdocコメント、設計判断は [adr/](adr/README.md)、測定記録は measurements/、教訓は lessons/ であり（docs/ 全体の配置は [README.md](README.md)）、本書は状態表、現在地、および次期候補だけを保持する。状態表と現在地は追記せず、マイルストーンの完了時または待機理由の変化時に書き直す。
完了したマイルストーンの結論を現在地に書かない理由と、その結論に依存する項目の書き方は、[plans/README.md](plans/README.md#上書きの規則) が定める。

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
| AlphaZero型探索の強化学習の実行 | [plans/archived/alphazero-rl.md](plans/archived/alphazero-rl.md) | 中断 | 2026年11月9日 |
| AlphaZero型探索の強化学習の2回目の実行の準備 | [plans/archived/alphazero-rl-2-prep.md](plans/archived/alphazero-rl-2-prep.md) | 完了（採用） | 2026年11月10日 |
| AlphaZero型探索の強化学習の2回目の実行 | [plans/alphazero-rl-2.md](plans/alphazero-rl-2.md) | 起案 | |
| 対局用MCTSを全部の規則コードで探索できるようにする | [plans/archived/mcts-rule-codes.md](plans/archived/mcts-rule-codes.md) | 完了（採用） | 2026年11月13日 |
| AlphaZero型探索の強さを測る固定の相手 | [plans/archived/mcts-fixed-opponents.md](plans/archived/mcts-fixed-opponents.md) | 完了（採用） | 2026年11月14日 |
| AlphaZero型探索のネットを診断する固定の検証用データ | [plans/zero-validation-games.md](plans/zero-validation-games.md) | 起案 | |
| 対局用MCTSのPUCTの係数と方策の温度のSPSAによる調整 | [plans/mcts-puct-spsa.md](plans/mcts-puct-spsa.md) | 起案 | |
| AlphaZero型探索の学習と自己対局生成の複数GPU化 | [plans/archived/alphazero-multi-gpu.md](plans/archived/alphazero-multi-gpu.md) | 完了（採用） | 2026年11月11日 |
| AlphaZero型探索の推論で、ホストとGPUの間の転送を減らす | [plans/archived/alphazero-inference-transfer.md](plans/archived/alphazero-inference-transfer.md) | 完了（採用） | 2026年11月11日 |
| 開発環境とvast.aiの機械の実行環境を、1つのDockerイメージで用意する | [plans/archived/docker-environment.md](plans/archived/docker-environment.md) | 完了（一部不採用） | 2026年11月13日 |
| AlphaZero型探索の推論で、終了処理での異常終了をなくす | [plans/archived/alphazero-teardown-abort.md](plans/archived/alphazero-teardown-abort.md) | 完了（採用） | 2026年11月11日 |
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
| NNUEの世代反復をv0からやり直す | [plans/archived/nnue-generation-reset.md](plans/archived/nnue-generation-reset.md) | 中断 | 2026年11月6日 |
| NNUEに隠れ層を加えて第1層を広げ、整数化の精度を上げる | [plans/archived/nnue-architecture-revision.md](plans/archived/nnue-architecture-revision.md) | 完了（一部不採用） | 2026年11月4日 |
| NNUE教師生成を探索結果を変えずに速くする | [plans/archived/teacher-generation-speedup.md](plans/archived/teacher-generation-speedup.md) | 完了 | 2026年11月6日 |
| NNUEの標準レシピv1と、v0からの世代反復のやり直し | [plans/nnue-standard-recipe.md](plans/nnue-standard-recipe.md) | 待機中 | |
| MCTSの葉での王駒の捕獲の判定と方策の温度 | [plans/archived/mcts-royal-capture-temperature.md](plans/archived/mcts-royal-capture-temperature.md) | 完了（採用） | 2026年11月9日 |
| 起動引数と診断ツールをUSIへ移す | [plans/archived/cli-to-usi.md](plans/archived/cli-to-usi.md) | 完了（採用） | 2026年11月12日 |
| NNUEの教師生成を中断の後に再開できるようにする | [plans/resumable-nnue-teacher-generation.md](plans/resumable-nnue-teacher-generation.md) | 起案 | |

## 現在地

以下の計画に着手するかどうかと実施の順序は、いずれも利用者の指示で決める。
方式と測定の条件は各設計書にあるので、ここには位置づけと次の判断だけを書く。

- [NNUEの標準レシピv1と、v0からの世代反復のやり直し](plans/nnue-standard-recipe.md)は、2026年11月7日にフェーズ1と2を終え、フェーズ3（第1世代の生成）は利用者の着手の指示を待っている。2026年11月6日に中断した[世代反復のやり直し](plans/archived/nnue-generation-reset.md)の系列の教師での対局の結果、標準レシピv1は線形の経路の学習率0.03を保ち、学習後の補正を行わず、その世代の教師だけで学ぶと決まった（[測定1](measurements/nnue-standard-recipe/nnue-standard-recipe-lr-linear-stc.md)、[測定2](measurements/nnue-standard-recipe/nnue-standard-recipe-fit-stc.md)、[測定3](measurements/nnue-standard-recipe/nnue-standard-recipe-window-stc.md)）。補正前のネットは補正後のネットに得点率57.1%で勝ち、第1世代の教師を併用したネットは第2世代の教師だけのネットに得点率36.4%で負けた。次は1世代を教師37本と定めてv0から世代反復をやり直す。2026年11月13日の利用者の決定で、新しい系列の自己対局と世代の判定の対局は規則セット`lishogi`で行うので、教師の生成器などの規則を切り替えてから、第1世代の37本もすべて新しく生成する。世代反復の最終候補は、[NNUEの構成の見直し](plans/archived/nnue-architecture-revision.md)の統合前のコミットを残したブランチ`nnue-width128-regressed`と比べる。最終候補が決まるまで、masterのエンジンは統合前より弱いと見込まれる。
- 教師生成の運用の残りの候補として、教師ファイルの末尾で次のファイルを重ねて始める案と、NUMAノードごとに2つのプロセスへ分ける案がある。前者は第2世代の生成機の実測で約1.12〜1.14倍の見込みだが、節約に対してスクリプトの変更が大きいので2026年11月5日の利用者の決定で見送った（[記録](measurements/nnue-generation-reset/nnue-generation-reset-gen2-training.md)）。後者は第1世代の測定で1.06〜1.09倍の見込みである（[記録](measurements/nnue-generation-reset/nnue-generation-reset-generator-speed.md)）。世代反復を延長するときなどに検討する。

### 評価関数（NNUE）

- [tataraの調査](research/tatara-nnue-transfer.md)の残りの候補（出力bucket、損失の形など）は、設計書がまだなく、世代反復の結果を見て起案する。採用条件は、棋力が向上し、配置の偏りが悪化しないことである（2026年11月3日の開発者の決定）。
- [第1層の全再計算の差分化（Finny tables）](plans/nnue-finny-tables.md)は待機中である。現行のP型では探索中に全再計算が起きないので、自玉の位置で条件付けるネットを対局で測るか、累算器の更新方式を変えた時点で再開する。

### 教師生成

強くする手段に人間の棋譜は使わない（[ADR](adr/do-not-use-human-games-to-strengthen-the-engine.md)）。

- [先読みなしの教師での間引きの比較](plans/nnue-subsample-no-lookahead.md)と[記録する局面の間引き](plans/sparse-teacher-recording.md)は、同じ計算時間でより多くの対局から教師を作る一連の計画であり、前者の判定が後者の着手の前提になる。どちらも現行の探索値だけの教師を前提に書かれているので、世代反復の結果を見て続けるかを決める。後者は`JCSA`で生成した基準の教師を使う前提なので、着手の前に`lishogi`の規則に合わせて改める。
- [教師生成の中断と再開](plans/resumable-nnue-teacher-generation.md)は起案の段階で、利用者の承認を待っている。生成器が教師ファイルを250局ずつの塊に分けて確定させ、止まった後は続きから生成できるようにする。いつ止められてもよい代わりに安い計算機（GCPのSpot VMやvast.aiの中断可能インスタンス）で教師を生成するための土台である。そうした計算機での料金は止まる頻度と失う量で決まり、中断に強い生成器がないと測れないので、借り先の比較と決定、および借りた機械を回す仕組みは、本書の完了後に起案する設計書で扱う。vast.aiより確実に安ければGCPに固定する（2026年11月14日の利用者の方針）。教師の内容は変えないので、[NNUEの標準レシピv1](plans/nnue-standard-recipe.md)のフェーズ3とは順序を問わない。
- [vast.aiでの教師生成](plans/vast-selfplay.md)は、教師の生成をvast.aiで借りる計算機へ移す計画である。試走では30ファイルを約5.5時間、約5.6米ドルと見込み（[記録](measurements/vast-selfplay/vast-selfplay-trial.md)）、作業機で同じ条件を測るフェーズ0の作業が残っている。世代反復は同書の選定条件と検査の方式を使うが、教師の取得の後に学習と補正まで続けて行うので専用のスクリプトで生成し、同書のフェーズ0の残りとフェーズ1を待たない。
- AlphaZero型のネットの評価値で教師の値を付け直す案は、[強豪エンジンの教師生成の調査](research/teacher-generation-strong-engines.md)を受けて、AlphaZero型の計画の後に起案する（2026年10月25日の開発者の決定）。tanuki-はこの方法でレーティング差R+104.3を得ている。付け直しに使う前に、[線形経路の計画](plans/archived/nnue-material-linear-init.md)の麒麟の診断で、そのネットに偏りがないことを確かめる。
- 設計書のない着想は調査メモにある。対局途中のランダム化と評価が食い違う局面の採取は[教師生成の調査](research/teacher-generation-strong-engines.md)に、教師の予算の重点配分は[評価関数を改善する3つの観点](proposals/archived/evaluation-improvement-strategy.md)に、別系統の局面を開始局面に加える案は[偏りの先行事例](research/self-play-bias-prior-art.md)にある。

### AlphaZero型探索

[統括計画](plans/alphazero.md)は、ゼロから強化学習で育てたネットのMCTSを、αβ探索とNNUEの組合せと同等以上に強くすることを目標とする。
統括計画のサブタスクは、実行環境、ネットの入出力と推論、探索方式の共存とMCTS、自己対局生成器、学習器、および強化学習の実行の6本であり、状態表のそのほかのAlphaZero型探索の行は、サブタスクを補う設計書である。
サブタスク1〜5は完了し、強化学習の[1回目の実行](plans/archived/alphazero-rl.md)はサイクル32で中断したので、次は2回目の実行で、20ブロック256チャネルのネットをランダムに初期化してやり直す。
2回目の実行のための初期ネット、複数GPU化、推論の転送の削減、および導入スクリプトは用意が済んでいる（状態表の完了の行）。

2回目の実行は、次の2本の設計書で進める。
1本目は、2026年11月13日に、2回目の実行の設計書から、学習の実行と独立に進められる準備を分けて起案したものである。
2本とも起案の段階で、利用者の承認を待っている。
同じ日に分けたほかの2本、[対局用MCTSの規則コードへの対応](plans/archived/mcts-rule-codes.md)と[強さを測る固定の相手](plans/archived/mcts-fixed-opponents.md)は完了している。

| 順 | 設計書 | 内容 | 前提 |
|---|---|---|---|
| 1 | [固定の検証用データ](plans/zero-validation-games.md) | lishogiの人間同士の棋譜から作る検証用データと、診断の道具の対応 | なし |
| 2 | [2回目の実行](plans/alphazero-rl-2.md) | 自己対局の規則の`lishogi`への切り替え、定例の測定のスクリプトの用意、3サイクルの試走、機械の選択、およびサイクルの実行 | 規則の切り替えと試走は前提なし。サイクル1は順1の完了の後 |

次の2件は、2回目の実行と独立に進められる。

- [対局用MCTSのPUCTの係数と方策の温度のSPSAによる調整](plans/mcts-puct-spsa.md)は起案の段階で、利用者の承認を待っている。道具の変更はすぐに始められるが、本番の調整は、2回目の実行のネットのうち利用者が指定するものを使う。自己対局の探索はPUCTの係数を使わず、定例の測定はサイクル1を始めたときのバイナリに固定するので、調整しても2回目の学習の途中で条件が変わることはない。
- 2回目の実行と[NNUEの標準レシピv1](plans/nnue-standard-recipe.md)は、借りる計算機が競合しないので並行して進め、作業機での対局の測定だけを重ならないように順番を調整する。どちらも、自己対局とネットの強さを測る対局を規則セット`lishogi`で行う（2026年11月13日の利用者の決定、[統括計画](plans/alphazero.md)の「自己対局と測定の規則」の節）。2回目の実行のHaChuとの対局だけは、HaChuの規則で行う。

統括計画の完了の後には、AlphaZero型のネットで教師の値を付け直す案（「教師生成」の節）を起案する。到達の判定でMCTSがαβ探索と同等以上とされた場合は、既定の探索方式の変更とlishogi Botへの配備も別の設計書で扱う。

### 探索

- [棋力向上の段階計画](plans/strength-stages.md)は、段階11（並列度、終盤処理）が未着手である。
- [詰み探索（df-pn）](plans/mate-search.md)は、攻め方の手を王手に限る標準df-pnのソルバを、`go mate`と対局探索の両方から呼べるライブラリとして作る。αβ探索とPV-MCTSへの組み込みは、発動率を測ってから別の設計書で起案する。

### 合法手生成と対局ハーネス

- [合法手生成と利き計算の高速化（第3期）](plans/movegen-speedup-3.md)は、起案のまま着手していない。
- [早期投了の導入判定](plans/match-early-resignation.md)は待機中である。校正群とは別に、仮想投了が3,000回以上発火する検証群を確保できる記録量に達しておらず、統計契約も確定していない。

### プロトコルと起動引数

- [起動引数と診断ツールをUSIへ移す計画](plans/archived/cli-to-usi.md)は、エンジンの起動引数`--rules`と`--nn-model`を削除し、規則セットとネットの指定をUSIのオプションで必須にした。この変更は、エンジンを起動する側の追従を必要とする。開発用GUIとlishogi Botの配備一式には、それぞれのリポジトリの`docs/plans/minase-ruleset-option.md`に追従の計画を起案してあり、どちらも利用者の着手の指示を待っている。開発用GUIは、変更を含むminaseの実行ファイルへ切り替えるのと同じ時点で改める。Botは、変更を含む版のminaseをリリースした後に、固定する版、起動用のスクリプト、および設定を同じコミットで切り替える。

### 配備

- [lishogi Bot接続](plans/lishogi-bot.md)は公開運用中であり、完了条件の達成状況の確認と配備方式の記述の見直しが残っている。Botはcrates.ioのminase 2.6.0を固定しているので、結果回帰の補正ネットを配備するには[リリース](guides/release.md)が要る。

### 前提の見直しが要る計画

- [Athénanの計画](plans/athenan.md)は、PSTを既定の評価関数とする前提で書かれており、2026年10月12日にNNUEへ置き換わった後の扱いが決まっていない。対局用の探索UBFMsは評価関数によらない試みなので計画は残し（2026年11月3日の利用者の判断）、着手の前に前提を見直す。

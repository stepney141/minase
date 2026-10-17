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
| AlphaZero型探索と深層強化学習 | [plans/alphazero.md](plans/alphazero.md) | 起案 | |
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
| 王の安全度と利きに基づく評価特徴の再学習 | [plans/evaluation-terms-relearning.md](plans/evaluation-terms-relearning.md) | 起案 | |
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
| 秒読みつき時計での持ち時間と秒読みの活用 | [plans/byoyomi-time-usage.md](plans/byoyomi-time-usage.md) | 進行中 | |
| NNUEの共通土台と初代のネットによるPST＋FMの完全代替 | [plans/archived/nnue-replacement.md](plans/archived/nnue-replacement.md) | 完了（採用） | 2026年10月12日 |
| 手の順序付けの改善 | [plans/archived/move-ordering.md](plans/archived/move-ordering.md) | 完了（採用） | 2026年10月10日 |
| 詰み探索（df-pn） | [plans/mate-search.md](plans/mate-search.md) | 起案 | |
| Rust製NNUE学習器（GPU） | [plans/archived/nnue-trainer-rust.md](plans/archived/nnue-trainer-rust.md) | 完了（採用） | 2026年10月12日 |
| NNUE学習器の学習率スケジュール | [plans/archived/nnue-lr-schedule.md](plans/archived/nnue-lr-schedule.md) | 完了（採用） | 2026年10月16日 |
| NNUEの隠れ層と位相補間の出力 | [plans/archived/nnue-hidden-layer.md](plans/archived/nnue-hidden-layer.md) | 完了（一部不採用） | 2026年10月16日 |
| NNUEの出力部の層積みへの統一 | [plans/nnue-layer-stack.md](plans/nnue-layer-stack.md) | 起案 | |
| Python学習器のRustへの移植と撤去 | [plans/archived/python-trainer-retirement.md](plans/archived/python-trainer-retirement.md) | 完了（採用） | 2026年10月13日 |
| ファイル形式の整理と形式の仕様の整備 | [plans/archived/file-format-consolidation.md](plans/archived/file-format-consolidation.md) | 完了（採用） | 2026年10月13日 |
| 教師データ形式の統一 | [plans/archived/unified-teacher-format.md](plans/archived/unified-teacher-format.md) | 完了（採用） | 2026年10月14日 |
| メモリより大きい教師データの一様シャッフル | [plans/archived/out-of-core-shuffle.md](plans/archived/out-of-core-shuffle.md) | 完了（採用） | 2026年10月17日 |
| 引き分けの評価値の設定（DrawValue） | [plans/draw-value.md](plans/draw-value.md) | 進行中 | |
| vast.aiでの教師生成 | [plans/vast-selfplay.md](plans/vast-selfplay.md) | 起案 | |
| 探索の枝刈りとLMRの関数形の見直し | [plans/search-pruning-forms.md](plans/search-pruning-forms.md) | 起案 | |
| 浅い前向き枝刈りの深さと対象の拡張 | [plans/shallow-pruning-extensions.md](plans/shallow-pruning-extensions.md) | 起案 | |
| historyの更新量の形とcontinuation history | [plans/history-forms.md](plans/history-forms.md) | 起案 | |
| correction historyの表の追加と持ち越し | [plans/correction-history-tables.md](plans/correction-history-tables.md) | 起案 | |
| 静止探索の補正と枝刈りの形の見直し | [plans/qsearch-forms.md](plans/qsearch-forms.md) | 起案 | |
| aspiration windowsの幅と窓の移し方の見直し | [plans/aspiration-window-forms.md](plans/aspiration-window-forms.md) | 起案 | |
| 探索の不安定さに応じた持ち時間の伸縮 | [plans/time-instability-scaling.md](plans/time-instability-scaling.md) | 起案 | |
| NNUEの検証損失が早く頭打ちになる原因の切り分け | [plans/nnue-early-best-epoch.md](plans/nnue-early-best-epoch.md) | 起案 | |
| 第1層の全再計算の差分化（Finny tables） | [plans/nnue-finny-tables.md](plans/nnue-finny-tables.md) | 待機中 | |

## 現在地

### 直近の完了

- [メモリより大きい教師データの一様シャッフル](plans/archived/out-of-core-shuffle.md)は2026年10月17日に、採用で完了した。NNUEの学習器に、MNSDを1局ずつ再生して局面を実行専用の一時ファイルへ書き、エポックごとに各訓練局面を一様にB個のbucketへ振り分けてbucketごとに並べ替える経路（`--buckets`）を加えた。`--buckets 1`は変更前とビット単位で同じ結果を出し、B=16は10ファイル（訓練約2,650万局面）の学習をメモリの上限2 GBの中で最大常駐0.91 GBで完走し、所要時間はB=1を上回らなかった（[照合記録](measurements/out-of-core-shuffle-equivalence.md)、[時間とメモリの記録](measurements/out-of-core-shuffle-time-memory.md)）。
- [NNUEの隠れ層と位相補間の出力](plans/archived/nnue-hidden-layer.md)は2026年10月16日に、一部不採用で完了した。学習器から駒除去差分の補助損失を外し、代わりに書き出し時に駒除去差分の符号反転の割合を記録するようにした。これによりGPUの1更新はP型で27.3 msから4.7 msに縮んだが、補助損失なしで学んだネットは検証1万局面の除去の1.7%で符号が反転し（補助損失ありでは5件）、罰則の発火率の低さは補助損失が不要であることを意味しなかった（[教訓](lessons/low-penalty-activation-does-not-mean-the-penalty-is-unneeded.md)）。推論と学習器に加えた3つの出力部（位相補間、幅16の隠れ層、その両方）は、同じ条件で学んでも検証損失が補助損失なしの直接内積を下回らず（隠れ層つきは訓練損失だけが下がる過学習の側）、段階ゲートに出す候補はなかった（[学習の記録](measurements/nnue-hidden-layer-training.md)、[速度の記録](measurements/nnue-hidden-layer-bench.md)）。埋め込みネットと`ActiveHead`は変えず、出力部のコードは汎用の層積みを設計する次の設計書へ引き継ぐ。
- [勝率尺度の1定数化と教師の尺度補正係数の明示（Kの改名）](plans/archived/win-rate-scale.md)は2026年10月16日に、採用で完了した。学習器が「K」と呼んでいた2つの勝率尺度を、重みファイルのヘッダが持つ勝率尺度`cp_per_logit`と教師の分類ごとの教師の勝率尺度`teacher_cp_per_logit`に改名し、両者の比を尺度補正係数`teacher_score_multiplier`として記録するようにした。教師の勝率尺度は、学習の必須の引数`--teacher-cp-per-logit`で、訓練集合から推定する`calibrate`か、新設の`minase-trainer data calibrate`が書き出す較正ファイルのパスを指定する。計算する値は変えず、NNUE、PST、FM、同時学習、および診断の学習結果と、較正コマンドの出力が変更前の学習器と文字列として一致した（[照合の記録](measurements/win-rate-scale-equivalence.md)、[ADR](adr/hold-cp-per-logit-in-the-weight-header-and-pass-the-teacher-scale-explicitly.md)）。

### 次の候補

- [NNUEの検証損失が早く頭打ちになる原因の切り分け](plans/nnue-early-best-epoch.md)は、現行のNNUEの学習で検証損失が学習率の与え方によらず約0.4489で頭打ちになる原因を、教師の側の要因に切り分ける。同じ対局の後続40手を混ぜる先読み教師については先読みを外した学習との汎化差で、対局数の少なさについては局数と局面数を入れ替えた間引きの学習で、事前に定めた基準により判定し、決着局面の多さは集計で記述する。先読みを外したネットは段階ゲートで採否を決める。[分割の検査](measurements/nnue-validation-split-check.md)では分割と局面の重複に不具合はなかった。着手は利用者の指示を待つ。
- [NNUEの出力部の層積みへの統一](plans/nnue-layer-stack.md)は、前計画が残した4つの固定の出力部を、隠れ層の段数と各幅、端点数を名前で与える1つの出力部（`stack-h16-e2`のような構成名）に置き換え、現行の直接内積を`stack-e1`として同じ型で表す。構造の整理であり段階ゲートは持たず、ノード数の一致と前計画の学習結果のビット単位の再現で挙動が変わっていないことを確かめる。以後のアーキテクチャの実験は、構成名を変えて学び段階ゲートで測るだけになる。着手は利用者の指示を待つ。
- [第1層の全再計算の差分化（Finny tables）](plans/nnue-finny-tables.md)は、Stockfishとやねうら王がNNUEの高速化に使う表をminaseに当てはめられるかを調べ、待機中とした。この表は、自玉の位置で条件付ける入力特徴集合や、差分の連鎖が途切れる更新方式が生む第1層の全再計算を速くするものであり、現行のP型と着手ごとの差分更新では探索中に全再計算が起きない（深さ9のbenchで累算器の更新6,400,021回のうち15回で、いずれも探索の開始時）。自玉の位置で条件付けるネットを時間制御の対局で測り直すか、更新方式を変えて全再計算が起きるようになった時点で再開する。
- [引き分けの評価値の設定](plans/draw-value.md)は、千日手の引き分けに返す評価値をやねうら王式のUSIオプション`DrawValueBlack`と`DrawValueWhite`で設定できるようにする。オプションと探索の実装は完了し、既定値では探索が変わらないことを`bench`で確かめた。残りは、この版のリリース後にlishogiのBotの設定で両オプションを−1にするフェーズであり、リリースは利用者の判断を待つ。[手の順序付けの改善](plans/archived/move-ordering.md)の完了を受けて、counter move historyとcontinuation historyの持ち越し（再診断で発動率44.6〜71.8%）を扱う設計書の起案も候補になる。
- 探索が使う既定の評価関数は、2026年10月12日に[NNUE](plans/archived/nnue-replacement.md)へ置き換わった。PSTとFMを既定としていた時期に挙げたほかの候補（PSTのさらに長い学習、補正なしのPSTを軽い評価として使うlazy evaluation）を、NNUEの採用後も続けるかは決まっていない。
- [Athénanの計画](plans/athenan.md)は、学習用の探索Descentで採用PSTから追加学習し、学習したPSTを現行のαβ探索に載せた候補と、対局用の探索UBFMsに載せた候補の採否を別々に測る。PSTを既定の評価関数とする前提で書かれているので、着手の前に前提を見直すかどうかを決める必要がある。
- 評価関数の後続3計画（[静止探索の出力](plans/archived/qsearch-output-training.md)、[順位の損失](plans/archived/rank-loss-training.md)、[関係補正項](plans/archived/relational-correction.md)）は、いずれも採用候補なしで完了した。順位の損失を関係を表すモデルと組み合わせる方式は、関係補正項の計画が補正項を選ばなかったので、起案するかは利用者の判断による。[評価関数を改善する3つの観点](research/evaluation-improvement-strategy.md)が挙げた、候補の誤りへの教師予算の重点配分、教師の判定が不安定な根の追加診断、および教師が必要な差を識別できない場合の代替教師の検討は、3計画には含めず、担当計画は未定である。
- [王の安全度と利きに基づく評価特徴の再学習](plans/evaluation-terms-relearning.md)は、[原因調査](audits/heuristic-learning-causes-2026-09-27.md)の結果を受けて、段階9で見送った7項目を採用中の教師で学び直し、新しい自己対局での予測損失で項目を選別してから、土台の費用ごとに段を分けて採否を測る。この計画もPSTに評価項を加える前提で書かれているので、着手の前に前提を見直すかどうかを決める必要がある。
- [秒読みつき時計での持ち時間と秒読みの活用](plans/byoyomi-time-usage.md)は、段階A（秒読み期の締切と途中結果の採用）を秒読みつきの2条件のGSPRTでともに`H1`として採用し、段階B（難しさで重み付けした持ち時間の配分）は不採用とした。2026年10月7日にmasterへ統合し、2026年10月8日にminase 2.2.0として公開した。lishogiのBotは2.4.0で動いており、`ByoyomiMargin`を3,000 msにした後の秒読みつきの対局20局の記録が残っている。
- [詰み探索（df-pn）](plans/mate-search.md)は、受け方の王駒が1枚の局面を対象に、攻め方の手を王手に限る標準df-pnの単独ソルバを`go mate`と`minase dev mate`から使えるようにし、証明手順を審判層で検証してから出力する。通常探索への組み込みは、採否の測定条件での発動率を測ってから別の設計書で起案する。着手は利用者の指示を待つ。
- 探索の関数形を両エンジンの形へ寄せる7本の設計書を、連鎖として起案した。[係数の関数形の比較](research/search-coefficient-forms-prior-art.md)が、minaseの探索と時間管理の係数の多くはStockfishとYaneuraOuと違う形で使われ、その差は係数の値の調整では埋まらないことを示したためである。提案する順は、[枝刈りとLMRの関数形](plans/search-pruning-forms.md)、[浅い枝刈りの拡張](plans/shallow-pruning-extensions.md)、[history](plans/history-forms.md)、[correction historyの表](plans/correction-history-tables.md)、[静止探索](plans/qsearch-forms.md)、[aspiration windows](plans/aspiration-window-forms.md)、[持ち時間の伸縮](plans/time-instability-scaling.md)である。各設計書は1つ前の設計書の完了後のmasterを起点とし、全項目を1つの候補にまとめて全係数をSPSAで調整してから段階ゲートで判定する。SPSAは利用者の決定で1本あたり30,000ペア（約3日）とし、測定機の占有は1本あたり約3.2〜4.8日、7本すべてで約1か月になる。係数の数は全部を採用すると約95になる。順序と取捨、および各設計書が挙げる利用者の判断事項は利用者が決め、着手は利用者の指示を待つ。
- [vast.aiでの教師生成](plans/vast-selfplay.md)は、自己対局の教師生成をvast.aiで借りる計算機へ移し、作業機より短い時間で終える計画である。既存の生成単位であるシード範囲1つにつきファイル1本をそのままバッチとし、生成機のディスクに置いたファイルを作業機が10分ごとに取得して検査し、検査済みのファイルが作業機にあることを完了の記録とする。機種はZen 4以降のEPYCを2基積んだ384スレッドの機械を条件で選ぶ。フェーズ0の生成機での試走では、EPYC 9654の2基構成が同時384局で1ファイルを653秒で生成し、30ファイルは約5.5時間、約5.6米ドルと見込む（[試走の記録](measurements/vast-selfplay-trial.md)）。作業機での同じ条件の測定は、作業機で動いている別の学習が終わるのを待っている。
- 評価関数の計画と探索部の計画は測定機を共有するので、着手の順序は利用者が決める。
- [AlphaZero型探索と深層強化学習](plans/alphazero.md)は、GPUを使う学習と強化学習の自己対局を、利用者が契約したGoogle Colabの有料プランの仮想機械（公式CLI `colab` で作業機から操作し、費用は月次付与の計算単位で払う。強化学習は1か月が上限）で行い、教師生成と時間制御の測定を作業機で行う形で設計を確定した。2026年10月14日に、NNUEの採用、Python学習器の撤去、教師データ形式の版2への統一、およびレンタルGPUサーバからColabへの変更を反映して改訂した。実装はまだなく、追加購入の上限は1万円、教師データの保管先は作業機にマウントしたNASと決まり、着手は利用者の許可を待ち。

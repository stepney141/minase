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
| 探索部の反復負け回避 | [plans/search-repetition.md](plans/search-repetition.md) | 起案 | |
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
| NNUE学習器の学習率スケジュール | [plans/nnue-lr-schedule.md](plans/nnue-lr-schedule.md) | 起案 | |
| NNUEの隠れ層と位相補間の出力 | [plans/nnue-hidden-layer.md](plans/nnue-hidden-layer.md) | 起案 | |
| Python学習器のRustへの移植と撤去 | [plans/archived/python-trainer-retirement.md](plans/archived/python-trainer-retirement.md) | 完了（採用） | 2026年10月13日 |
| ファイル形式の整理と形式の仕様の整備 | [plans/archived/file-format-consolidation.md](plans/archived/file-format-consolidation.md) | 完了（採用） | 2026年10月13日 |
| NNUEに合わせた探索係数と時間管理係数のSPSAによる再調整 | [plans/nnue-search-spsa.md](plans/nnue-search-spsa.md) | 起案 | |
| 教師データ形式の統一 | [plans/archived/unified-teacher-format.md](plans/archived/unified-teacher-format.md) | 完了（採用） | 2026年10月14日 |

## 現在地

### 直近の完了

- [教師データ形式の統一](plans/archived/unified-teacher-format.md)は2026年10月14日に、採用で完了した。教師データの形式を、1局ごとに開始局面を1回だけ書き、以後は1手ごとに指した手、価値、印、および方策を並べる対局単位のMNSDの版2の1つにまとめ、AlphaZeroの計画が予定していた別の形式MNZTは作らなかった。生成器はこれまで捨てていた局面にも除外の理由を付けて書き、学習器は手順を再生して局面を復元する。同じ条件で生成した版1と版2のデータで、学習器が読む局面の列と、NNUEとPSTの学習結果がビット単位で一致することを確かめ（[照合の記録](measurements/unified-teacher-format-equivalence.md)）、生のバイナリは版1の約11分の1から17分の1になった。版1のデータは利用者の判断で生成し直さず、`data/`の44ファイル（約18.3 GB）を削除した。
- [ファイル形式の整理と形式の仕様の整備](plans/archived/file-format-consolidation.md)は2026年10月13日に、採用で完了した。各形式の版を1つにし、過去のファイルを読み続けない原則で形式を整理した。教師データの局面の由来をMNSDのレコードに置いて来歴JSONと付け直し（MNRS）を撤去し、MNPTを版3だけにし、学習器のチェックポイントをNPZに置き換えて、`MN`の独自形式を7から3へ減らした。残った全形式の仕様はそれを書き出すモジュールのdocコメントに、一覧は[ファイル形式の一覧](guides/file-formats.md)にある。変更の前後で学習の結果がビット単位で一致することを確かめ（[照合の記録](measurements/file-format-consolidation-equivalence.md)）、使えなくなった`data/`のファイル594件（約30 GB）を削除した。学習器の引数`--lambda-override`は必須の`--lambda`に変わった。
- [Python学習器のRustへの移植と撤去](plans/archived/python-trainer-retirement.md)は2026年10月13日に、採用で完了した。PST、FM、PSTとFMの同時学習の学習器と、PSTとNNUEの診断を、学習器のクレート`crates/minase-trainer`（`nnue-trainer`から改名）へ移植し、評価関数ごとに独立したパイプラインとして持たせた。Pythonの学習順と初期値を一時的に与えた照合で、検証損失と最良エポックが一致し、書き出した整数の重みはPSTでバイト単位、FMで±1単位以内で一致し、現行の`crates/minase/nets/pst.bin`も`fm scale`でバイト単位で再生成できた（[照合の記録](measurements/python-trainer-retirement-checks.md)）。CPUの学習器はPythonのGPUの学習器より速いのでGPUのカーネルは加えず、Pythonの学習ツールと`minase dev pst-probe`、`minase dev nnue-probe`を削除した。同日にmasterへマージした。
- [NNUEの共通土台と初代のネットによるPST＋FMの完全代替](plans/archived/nnue-replacement.md)は2026年10月12日に、採用で完了した。現行PSTを線形経路として内包する幅128のP型NNUEを、既存の約2,920万局面と新規の約5,100万局面（1局面2万ノード）から[Rust製NNUE学習器](plans/archived/nnue-trainer-rust.md)で学び、基準M（PST＋FM）に対する段階ゲートでSTC `H1`（[0, 0, 24, 0, 80]）、LTC `H1`（[0, 0, 17, 0, 87]、異常0件）となった（[STC](measurements/nnue-replacement-stc.md)、[LTC](measurements/nnue-replacement-ltc.md)）。同じデータのHalfKP-hm-b8はP型に対するSTCで`H0`だった。探索が使う既定の評価関数はNNUEになり、同日にmasterへマージした。
- [Rust製NNUE学習器（GPU）](plans/archived/nnue-trainer-rust.md)は2026年10月12日に、採用で完了した。将棋のNNUE学習器tataraの速さがRustではなく疎なgather、逆索引の後退、融合した最適化器、非同期の供給というGPUカーネルとデータ経路の設計に由来することを一次情報で確かめ（[調査](research/nnue-trainer-speed.md)）、ホストをRust、カーネルをCUDA C++（NVRTC）で書いた学習器`crates/nnue-trainer`（2026年10月13日に`crates/minase-trainer`へ改名）を作った。GPUカーネルはCPU参照と一致し、1更新の勾配と重みはPythonの学習器と一致する。全30ファイルの1エポックは145〜155秒で、Pythonの学習器の約20分の1である（[記録](measurements/nnue-trainer-rust-checks.md)）。

### 次の候補

- 探索部の次の対象は[探索部の反復負け回避](plans/search-repetition.md)である。[手の順序付けの改善](plans/archived/move-ordering.md)の完了を受けて、counter move historyとcontinuation historyの持ち越し（再診断で発動率44.6〜71.8%）を扱う設計書の起案も候補になる。
- 探索が使う既定の評価関数は、2026年10月12日に[NNUE](plans/archived/nnue-replacement.md)へ置き換わった。[NNUEに合わせた探索係数と時間管理係数のSPSAによる再調整](plans/nnue-search-spsa.md)は、`FmScale`を除く38係数（手の順序付けで加わった未調整の3係数と、時間管理の6係数を含む）を現行のネットで調整し直し、起点のmasterとの段階ゲートで採否を決める。測定機の占有は、過去の同規模の記録からの概算で約半日から2日であり、着手は利用者の指示を待つ。PSTとFMを既定としていた時期に挙げたほかの候補（PSTのさらに長い学習、補正なしのPSTを軽い評価として使うlazy evaluation）を、NNUEの採用後も続けるかは決まっていない。
- [Athénanの計画](plans/athenan.md)は、学習用の探索Descentで採用PSTから追加学習し、学習したPSTを現行のαβ探索に載せた候補と、対局用の探索UBFMsに載せた候補の採否を別々に測る。PSTを既定の評価関数とする前提で書かれているので、着手の前に前提を見直すかどうかを決める必要がある。
- 評価関数の後続3計画（[静止探索の出力](plans/archived/qsearch-output-training.md)、[順位の損失](plans/archived/rank-loss-training.md)、[関係補正項](plans/archived/relational-correction.md)）は、いずれも採用候補なしで完了した。順位の損失を関係を表すモデルと組み合わせる方式は、関係補正項の計画が補正項を選ばなかったので、起案するかは利用者の判断による。[評価関数を改善する3つの観点](research/evaluation-improvement-strategy.md)が挙げた、候補の誤りへの教師予算の重点配分、教師の判定が不安定な根の追加診断、および教師が必要な差を識別できない場合の代替教師の検討は、3計画には含めず、担当計画は未定である。
- [王の安全度と利きに基づく評価特徴の再学習](plans/evaluation-terms-relearning.md)は、[原因調査](audits/heuristic-learning-causes-2026-09-27.md)の結果を受けて、段階9で見送った7項目を採用中の教師で学び直し、新しい自己対局での予測損失で項目を選別してから、土台の費用ごとに段を分けて採否を測る。この計画もPSTに評価項を加える前提で書かれているので、着手の前に前提を見直すかどうかを決める必要がある。
- [秒読みつき時計での持ち時間と秒読みの活用](plans/byoyomi-time-usage.md)は、段階A（秒読み期の締切と途中結果の採用）を秒読みつきの2条件のGSPRTでともに`H1`として採用し、段階B（難しさで重み付けした持ち時間の配分）は不採用とした。2026年10月7日にmasterへ統合し、2026年10月8日にminase 2.2.0として公開した。lishogiのBotは2.4.0で動いており、`ByoyomiMargin`を3,000 msにした後の秒読みつきの対局20局の記録が残っている。
- [詰み探索（df-pn）](plans/mate-search.md)は、受け方の王駒が1枚の局面を対象に、攻め方の手を王手に限る標準df-pnの単独ソルバを`go mate`と`minase dev mate`から使えるようにし、証明手順を審判層で検証してから出力する。通常探索への組み込みは、採否の測定条件での発動率を測ってから別の設計書で起案する。着手は利用者の指示を待つ。
- [NNUE学習器の学習率スケジュール](plans/nnue-lr-schedule.md)と[NNUEの隠れ層と位相補間の出力](plans/nnue-hidden-layer.md)は、tataraにあってminaseにない機能のうち利用者が取り込むと決めたものを扱う。前者は、NNUEの検証損失が最初の訓練エポックの直後で頭打ちになる症状を受けて、更新ごとの学習率の倍率を加え、5つの設定のどれかで改善するかを確かめる。後者は、学習器から駒除去差分の補助損失を外したうえで、出力部に幅16の隠れ層と、盤上総駒数で序盤用と終盤用の出力を補間する位相補間を加えた候補を比べる。後者の学習は前者の結論を待つ。両計画が学習に使う約30ファイルの教師データは[教師データ形式の統一](plans/archived/unified-teacher-format.md)で削除したので、前者の学習の前に版2で生成し直す（約35時間）。
- 評価関数の計画と探索部の計画は測定機を共有するので、着手の順序は利用者が決める。
- [AlphaZero型探索と深層強化学習](plans/alphazero.md)は、GPUを使う学習を外部のレンタルGPUサーバ（総額2万円、強化学習は1か月が上限）で行い、教師生成と時間制御の測定を作業機で行う形で設計を確定した。着手は利用者の許可を待ち、SPSAの調整セッションが完了しても許可があるまでは始めない。

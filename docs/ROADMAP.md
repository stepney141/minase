# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの設計の正は plans/ 配下の設計書、測定記録は measurements/、教訓は lessons/ であり（docs/ 全体の配置は [README.md](README.md)）、本書は状態表、現在地、次期候補、および横断的な決定だけを保持する。状態表と現在地は追記せず、マイルストーンの完了時または待機理由の変化時に書き直す。
「直近の完了」には完了日の新しい順に3件だけを置き、件数の規則と削除する項目の扱いは [plans/README.md](plans/README.md#上書きの規則) が定める。

## マイルストーン状態表

| マイルストーン | 設計書 | 状態 | 完了日 |
|---|---|---|---|
| 合法手生成とmake/unmake | [plans/movegen.md](plans/movegen.md) | 完了（採用） | 2026年7月30日 |
| 指し手正準化 | [plans/move-canonicalization.md](plans/move-canonicalization.md) | 完了（採用） | 2026年7月31日 |
| 対局管理層と審判層 | [plans/game-referee.md](plans/game-referee.md) | 完了（採用） | 2026年8月1日 |
| ローカルルール13コードの実装 | [plans/local-rules.md](plans/local-rules.md) | 完了（採用） | 2026年8月2日 |
| 合法手生成器の総仕上げ | [plans/movegen-hardening.md](plans/movegen-hardening.md) | 完了（採用） | 2026年8月3日 |
| 審判ロジック再編とR0撤去 | [plans/adjudication-refactor.md](plans/adjudication-refactor.md) | 完了（採用） | 2026年8月9日 |
| ランダム対局検証ハーネス | [plans/random-play.md](plans/random-play.md) | 完了（採用） | 2026年8月10日 |
| プロトコル層 | [plans/protocol-layer.md](plans/protocol-layer.md) | 完了（採用） | 2026年8月10日 |
| ブラウザGUI向けUSI照会 | [plans/browser-gui.md](plans/browser-gui.md) | 完了（採用） | 2026年8月11日 |
| 対局ハーネスのバイナリ対戦化 | [plans/match-harness.md](plans/match-harness.md) | 完了（採用） | 2026年8月11日 |
| 直前局面生成器 | [plans/predecessor-generator.md](plans/predecessor-generator.md) | 完了（採用） | 2026年9月18日 |
| 探索部 | [plans/search.md](plans/search.md) | 完了（一部不採用） | 2026年8月22日 |
| 外部対局接続 | [plans/engine-connectivity.md](plans/engine-connectivity.md) | 完了（採用） | 2026年8月14日 |
| Lazy SMP | [plans/lazy-smp.md](plans/lazy-smp.md) | 完了（一部不採用） | 2026年8月23日 |
| テストスイートのspec-first再構築 | [plans/spec-first-tests.md](plans/spec-first-tests.md) | 完了（採用） | 2026年8月15日 |
| 評価関数 | [plans/evaluation.md](plans/evaluation.md) | 完了（一部不採用） | 2026年8月26日 |
| 規則集合の代数的データ型化 | [plans/rules-type.md](plans/rules-type.md) | 完了（採用） | 2026年8月25日 |
| Rust設計監査対応 | [plans/rust-design-audit-remediation.md](plans/rust-design-audit-remediation.md) | 完了（採用） | 2026年8月26日 |
| 評価関数の世代反復 | [plans/evaluation-gen1.md](plans/evaluation-gen1.md) | 完了（一部不採用） | 2026年8月29日 |
| 棋力測定ハーネス基盤の効率化 | [plans/match-harness-efficiency.md](plans/match-harness-efficiency.md) | 完了（採用） | 2026年8月28日 |
| 棋力測定の段階ゲート | [plans/match-staged-gate.md](plans/match-staged-gate.md) | 完了（採用） | 2026年9月1日 |
| 早期投了の導入判定 | [plans/match-early-resignation.md](plans/match-early-resignation.md) | 待機中 | ― |
| 棋力向上の段階計画 | [plans/strength-stages.md](plans/strength-stages.md) | 進行中 | ― |
| 棋力向上段階1 | [plans/strength-stage1.md](plans/strength-stage1.md) | 完了（一部不採用） | 2026年9月2日 |
| 棋力向上段階2 | [plans/strength-stage2.md](plans/strength-stage2.md) | 完了（採用） | 2026年9月4日 |
| 棋力測定の所要時間削減 | [plans/match-cost-reduction.md](plans/match-cost-reduction.md) | 完了（採用） | 2026年9月4日 |
| lishogi Bot接続 | [plans/lishogi-bot.md](plans/lishogi-bot.md) | 進行中 | ― |
| 棋力向上段階3 | [plans/strength-stage3.md](plans/strength-stage3.md) | 完了（一部不採用） | 2026年9月5日 |
| 棋力向上段階4 | [plans/strength-stage4.md](plans/strength-stage4.md) | 完了（一部不採用） | 2026年9月6日 |
| 棋力向上段階5 | [plans/strength-stage5.md](plans/strength-stage5.md) | 完了（一部不採用） | 2026年9月7日 |
| PSTの序中盤と終盤の補間 | [plans/tapered-pst.md](plans/tapered-pst.md) | 完了（採用） | 2026年9月8日 |
| HaChu対minaseの条件格子測定 | [plans/hachu-condition-grid.md](plans/hachu-condition-grid.md) | 完了（採用） | 2026年9月9日 |
| Factorization Machineによる2駒関係評価 | [plans/factorization-machine.md](plans/factorization-machine.md) | 完了（不採用） | 2026年9月9日 |
| 棋力向上段階6 | [plans/strength-stage6.md](plans/strength-stage6.md) | 完了（一部不採用） | 2026年9月12日 |
| 棋力向上段階7 | [plans/strength-stage7.md](plans/strength-stage7.md) | 完了（一部不採用） | 2026年9月14日 |
| 合法手生成と利き計算の高速化 | [plans/movegen-speedup.md](plans/movegen-speedup.md) | 完了（一部不採用） | 2026年9月15日 |
| 合法手生成と利き計算の高速化（第2期） | [plans/movegen-speedup-2.md](plans/movegen-speedup-2.md) | 完了（一部不採用） | 2026年9月17日 |
| 合法手生成と利き計算の高速化（第3期） | [plans/movegen-speedup-3.md](plans/movegen-speedup-3.md) | 起案 | |
| 棋力向上段階8（前向き枝刈りの第3層） | [plans/strength-stage8.md](plans/strength-stage8.md) | 完了（一部不採用） | 2026年9月21日 |
| 持ち時間の効率的な使用（issue #7） | [plans/time-management-efficiency.md](plans/time-management-efficiency.md) | 完了（一部不採用） | 2026年9月18日 |
| USI先読み（ponder） | [plans/ponder.md](plans/ponder.md) | 完了（採用） | 2026年9月21日 |
| 棋力向上段階9（利きの土台、教師データの改善、王の安全度と利きに基づく評価特徴） | [plans/strength-stage9.md](plans/strength-stage9.md) | 完了（一部不採用） | 2026年9月23日 |
| SPSAによる探索係数と時間管理係数の調整 | [plans/spsa.md](plans/spsa.md) | 完了（採用） | 2026年9月25日 |
| 先読み教師値（将来の探索値の幾何加重平均） | [plans/lookahead-teacher.md](plans/lookahead-teacher.md) | 完了（一部不採用） | 2026年9月23日 |
| SPSAの調整結果をソースへ反映するコマンド | [plans/spsa-apply.md](plans/spsa-apply.md) | 完了（採用） | 2026年9月22日 |
| 探索部の反復負け回避 | [plans/search-repetition.md](plans/search-repetition.md) | 起案 | |
| 不採用だった探索部の改良のSPSAによる再調整（issue #10） | [plans/search-revival-spsa.md](plans/search-revival-spsa.md) | 完了（採用） | 2026年10月4日 |
| AlphaZero型探索と深層強化学習 | [plans/alphazero.md](plans/alphazero.md) | 起案 | |
| 棋力向上段階12（探索の小改良と表の寿命） | [plans/strength-stage12.md](plans/strength-stage12.md) | 完了（不採用） | 2026年9月28日 |
| USI投了（issue #6） | [plans/usi-resignation.md](plans/usi-resignation.md) | 完了（採用） | 2026年9月25日 |
| SPSAの摂動幅と学習率の較正 | [plans/spsa-gain-calibration.md](plans/spsa-gain-calibration.md) | 完了（採用） | 2026年9月26日 |
| 教師の混合比λ=1.0（探索値だけの教師） | [plans/teacher-mixing-ratio.md](plans/teacher-mixing-ratio.md) | 完了（不採用） | 2026年9月26日 |
| 探索局面を用いた評価関数の学習 | [plans/search-aware-evaluation.md](plans/search-aware-evaluation.md) | 完了（不採用） | 2026年9月26日 |
| 相対位置の局所2駒関係による評価の補正 | [plans/relative-pair-eval.md](plans/relative-pair-eval.md) | 完了（一部不採用） | 2026年9月26日 |
| Athénanによる評価関数の強化学習と対局探索 | [plans/athenan.md](plans/athenan.md) | 起案 | |
| 静止探索の出力を学ぶPSTの学習 | [plans/qsearch-output-training.md](plans/qsearch-output-training.md) | 完了（不採用） | 2026年9月30日 |
| 浅い探索の手の順位を学ぶ損失 | [plans/rank-loss-training.md](plans/rank-loss-training.md) | 完了（不採用） | 2026年10月1日 |
| 残存誤りに対応する関係補正項 | [plans/relational-correction.md](plans/relational-correction.md) | 完了（不採用） | 2026年10月2日 |
| 王の安全度と利きに基づく評価特徴の再学習 | [plans/evaluation-terms-relearning.md](plans/evaluation-terms-relearning.md) | 起案 | |
| デバッグ機能の整備 | [plans/debugging-tools.md](plans/debugging-tools.md) | 完了（採用） | 2026年9月27日 |
| 探索部の不具合修正 | [plans/search-bug-fixes.md](plans/search-bug-fixes.md) | 完了（一部不採用） | 2026年9月29日 |
| 評価関数の候補と探索係数の共適応の検証 | [plans/eval-search-coadaptation.md](plans/eval-search-coadaptation.md) | 完了（一部不採用） | 2026年10月5日 |
| 補正1/4のFMの現行PSTへの再学習 | [plans/fm-quarter-current-pst.md](plans/fm-quarter-current-pst.md) | 起案 | |
| 採用PSTの追加学習のエポック数の延長 | [plans/pst-longer-training.md](plans/pst-longer-training.md) | 起案 | |
| 世代3の教師データによるPSTの再学習とデータ半分割の診断 | [plans/pst-gen3.md](plans/pst-gen3.md) | 完了（採用） | 2026年10月3日 |
| ライブラリとエンジンのcrate分割 | [plans/crate-split.md](plans/crate-split.md) | 完了（採用） | 2026年10月4日 |
| 補助ツールのサブコマンド化 | [plans/cli-subcommands.md](plans/cli-subcommands.md) | 完了（採用） | 2026年10月5日 |
| 測定の再開条件の簡素化 | [plans/match-resume-simplification.md](plans/match-resume-simplification.md) | 起案 | |

## 現在地

### 直近の完了

- [補助ツールのサブコマンド化](plans/cli-subcommands.md)は2026年10月5日に、採用で完了した。対局測定、係数調整、学習データの生成、および検証と診断の10個の補助ツールを実行ファイル`minase`のサブコマンド（`minase match run`、`minase spsa`、`minase data selfplay`、`minase dev bench`など）へ移し、`cargo install minase`が導入する実行ファイルを1つにした。エンジンの起動`minase --protocol … --rules …`は変えていない。全ツールの固定シードの出力が移動前と一致し、benchのNPS中央値は移動前の0.9955倍だった（[測定記録](measurements/cli-subcommands-bench-depth5.md)）。runnerがエンジンと同じ実行ファイルになったので、測定の途中で`minase`を再ビルドすると再開できなくなる。
- [評価関数の候補と探索係数の共適応の検証](plans/eval-search-coadaptation.md)は2026年10月5日に、一部不採用で完了した。採用PSTのG23から混合比λ=1.0で学び直したPSTはSTCで−151.1 Elo（95%信頼区間 −177.5〜−125.1）と負け、探索係数29個をSPSAで調整し直しても回復量は−10.8 Elo（95%信頼区間 −45.9〜+25.4）で、負けを探索係数との不整合で説明する仮説は支持されなかった（[調整後](measurements/eval-coadaptation-after-stc400.md)）。追加診断では、同じ10エポックの追加学習をλ=0.75で行った対照Ccが段階ゲートの[STC](measurements/eval-coadaptation-lambda075-stc.md)と[LTC](measurements/eval-coadaptation-lambda075-ltc.md)でともに`H1`（得点率57.4%と57.3%）となり、新しい採用PSTになった。λ=1.0の負けは追加学習ではなく混合比に結び付き、G23は10エポックでは学習しきれていなかった。
- [ライブラリとエンジンのcrate分割](plans/crate-split.md)は2026年10月4日に、採用で完了した。リポジトリを仮想マニフェストのワークスペースとし、規則と合法手生成のライブラリ`minase-core`（MIT）と、探索、評価、プロトコル、および実行ファイルのエンジン`minase`（GPL-3.0-or-later）を`crates/`の下に分けた。分割前と比べてbench、perft、および試験の名前の集合が一致し、両crateの`cargo package`が検証ビルドまで通る。RULES.mdは[docs/rules/](rules/RULES.md)へ移した。エンジンの次の版は2.0.0、ライブラリは0.1.0から始め、リリースとcrates.ioへの公開は利用者が行う。

### 次の候補

- 探索部の次の対象は[探索部の反復負け回避](plans/search-repetition.md)である。[棋力向上段階12](plans/strength-stage12.md)で他エンジンの定番の改良が7件とも効果を示さなかったので、探索の改良を続ける前に、βカットのうち最初の手によるものが59%にとどまる手の順序付けの[監査](audits/beta-cutoff-first-move-2026-09-27.md)の結果と合わせて、原因を切り分けることも候補になる。
- 評価関数の次の計画は[採用PSTの追加学習のエポック数の延長](plans/pst-longer-training.md)である。採用PSTのPcを初期値に、同じデータと条件で最大40エポックまで学習し、検証損失が最良のエポックの重みを段階ゲートで測る。採用PSTを変え得るので、FMの計画より先に完了させる。
- その次は[補正1/4のFMの現行PSTへの再学習](plans/fm-quarter-current-pst.md)である（2026年10月4日の利用者決定）。基点の採用PSTは、共適応の検証で採用したPcに変わった。ブランチ`fm-eval`でSTCとLTCをともに通過したままmasterへ統合されなかった補正1/4のFMを、現行の採用PSTの上で学び直し、標準の段階ゲートで採否を測る。測定機の時間は約2〜39時間であり、補正の教師の選択が利用者の判断を待っている。
- 評価関数でほかに着手できるのは[Athénanの計画](plans/athenan.md)である。学習用の探索Descentで採用PSTから追加学習し、学習したPSTを現行のαβ探索に載せた候補と、対局用の探索UBFMsに載せた候補の採否を別々に測る。対照を置かず、学習は同時16対局で48時間を予算とする。着手の条件は満たしている。
- 評価関数の後続3計画（[静止探索の出力](plans/qsearch-output-training.md)、[順位の損失](plans/rank-loss-training.md)、[関係補正項](plans/relational-correction.md)）は、いずれも採用候補なしで完了した。順位の損失を関係を表すモデルと組み合わせる方式は、関係補正項の計画が補正項を選ばなかったので、起案するかは利用者の判断による。[評価関数を改善する3つの観点](research/evaluation-improvement-strategy.md)が挙げた、候補の誤りへの教師予算の重点配分、教師の判定が不安定な根の追加診断、および教師が必要な差を識別できない場合の代替教師の検討は、3計画には含めず、担当計画は未定である。
- [王の安全度と利きに基づく評価特徴の再学習](plans/evaluation-terms-relearning.md)は、[原因調査](audits/heuristic-learning-causes-2026-09-27.md)の結果を受けて、段階9で見送った7項目を採用中の教師で学び直し、新しい自己対局での予測損失で項目を選別してから、土台の費用ごとに段を分けて採否を測る。
- 評価関数の計画と探索部の計画は測定機を共有するので、着手の順序は利用者が決める。
- [AlphaZero型探索と深層強化学習](plans/alphazero.md)は、GPUを使う学習を外部のレンタルGPUサーバ（総額2万円、強化学習は1か月が上限）で行い、教師生成と時間制御の測定を作業機で行う形で設計を確定した。着手は利用者の許可を待ち、SPSAの調整セッションが完了しても許可があるまでは始めない。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

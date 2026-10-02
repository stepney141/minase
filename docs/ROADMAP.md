# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの設計の正は plans/ 配下の設計書、測定記録は measurements/、教訓は lessons/ であり（docs/ 全体の配置は [README.md](README.md)）、本書は状態表、現在地、次期候補、および横断的な決定だけを保持する。状態表と現在地は追記せず、マイルストーンの完了時または待機理由の変化時に書き直す。

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
| 不採用だった探索部の改良のSPSAによる再調整（issue #10） | [plans/search-revival-spsa.md](plans/search-revival-spsa.md) | 起案 | |
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

## 現在地

### 直近の完了

- [残存誤りに対応する関係補正項](plans/relational-correction.md)は2026年10月2日に、不採用で完了した。フェーズ0では、新しい自己対局の1,974根で探索の変更で解消を確認できなかった大きな誤り62根を独立した2者が分類し、一致した根は近接2駒15、多駒12、不明11、王駒射線3で、最多の近接2駒も基準の21根に届かず見送りとなった（[準備記録](measurements/relational-correction-prep.md)）。利用者の指示で基準を外して半径1の局所2駒表を実装して測ったところ、零の表でも探索速度がS0の0.775倍に落ち（[速度](measurements/relational-correction-speed.md)）、学習した表は検証損失を0.000178下げたものの、S0との[STC](measurements/relational-correction-stc.md)は`H0`（得点率39.3%、約−75 Elo）だった。[固定ノード数の200ペア](measurements/relational-correction-nodes200.md)はElo +9.4（95%信頼区間 −26.4〜+45.4）であり、損失は主に速度の低下による。
- [浅い探索の手の順位を学ぶ損失](plans/rank-loss-training.md)は2026年10月1日に、不採用で完了した。探索が変わったので15,000局を生成し直し、判定の差mを117.7センチポーンに測り直したうえで、[小標本](measurements/rank-loss-pilot.md)は2つの判定を満たした。S0の深さ1の値は、教師が安定して区別する手の組の約3分の2で順位を逆にしていた。[学習](measurements/rank-loss-training.md)では、順位の損失が検証の組の正答率を対照Cの0.33から0.48〜0.51へ上げた一方、静的評価と教師値の平均絶対誤差をCの1.18〜1.28倍に悪化させ、3通りのβがすべて事前に定めた上限の1.05倍を超えた。利用者の判断で、誤差の上限だけを外して選ばれるβ=1.0を対照Cと対局させた[STC](measurements/rank-loss-vs-control-stc.md)は、得点率38.5%（約−81 Elo）で`H0`となり、S0との対局へは進まなかった。
- [静止探索の出力を学ぶPSTの学習](plans/qsearch-output-training.md)は2026年9月30日に、採用候補なしで完了した。静かな局面で学習目標を静止探索の出力へ変えた候補Qは、同じ条件で静的値を学んだ対照Rに対して[STC](measurements/qsearch-output-q-vs-r-stc.md)で`H0`（得点率47.2%）となり、捕獲局面を加えた候補QcはQに対して[STC](measurements/qsearch-output-qc-vs-q-stc.md)の上限3,000ペアでLLRが負（得点率50.4%）だった。採否前の診断ではどちらも除外されておらず、比較用のSTCで除外された。フェーズ0の[探索の追跡](measurements/qsearch-output-search-trace.md)では、段階12の7項目と同じく、6根の誤りを特定の枝刈りに帰属できなかった。
- [探索部の不具合修正](plans/search-bug-fixes.md)は2026年9月29日に完了した。[探索部の不具合監査](audits/search-bugs-2026-09-28.md)の指摘のうち、反復の検出がnull moveをまたぎ、null move直後のじっとを実在しない引き分けとしていた不具合を直し、非劣性のSTCとLTCをともに通過して採用した（masterの得点率はSTC 45.7%、LTC 48.0%）。静止探索のdelta pruningが成り益を無視する不具合は、修正の診断で修正前の版がわずかに強い方向となったので取り込まず、不具合は残る。王駒がすべて狙われた静止探索の局面で全合法手を読む修正は約120 Elo弱く、不具合ではなく静止探索の近似と分類し直して、探索の不具合は手法の契約違反に限って分類するという[教訓](lessons/classify-bugs-by-method-contract.md)にした。
- [棋力向上段階12](plans/strength-stage12.md)は2026年9月28日に、採用項目なしで完了した。発動率の診断で基準を満たした7項目（aspiration windowsのfail-high時の減深、負の捕獲手の後回し、捕獲履歴、静止探索の手数制限、butterfly historyの持ち越し、null moveの前提条件、PVノードでの置換表の打ち切りの停止）は、いずれもSTCで`H0`または上限到達時にLLRが負となり、補正表の鍵の追加は診断で見送った。静止探索の手数制限は約−173 Eloと大きく負け、失う良手の割合で閾値を選ぶ規則を深さの上限のない再帰に使えないことを[教訓](lessons/recall-loss-does-not-bound-recursive-pruning.md)にした。探索コードは段階開始版から変わっていない。
- [デバッグ機能の整備](plans/debugging-tools.md)は2026年9月27日に完了した。USIの独自コマンド`d`、`eval`、`tt`、入出力のログ`--io-log`、`cargo test`で常に有効な整合検査（フィーチャ`invariants`）、および`bench`が出力する探索統計（フィーチャ`search-stats`）を追加し、使い方を[デバッグの手引き](guides/debugging.md)にまとめた。探索統計の初回の実行では、深さ5の`bench`でβカットのうち最初に探索した手によるものが59%であり、手の順序付けを見直すときの出発点になる。
- USI投了（issue #6）は、2026年9月25日にlishogiの公開対局で投了の成立を確認して完了した。
- SPSAによる調整は2回とも採用した。
  - 1回目の20係数は[STC](measurements/spsa-stage9-20260923-stc.md)と[LTC](measurements/spsa-stage9-20260923-ltc.md)でともに`H1`となり、2026年9月25日に採用して、探索部と関係する設計書の現行値へ反映した。
  - 2026年9月26日の較正で、`spsa_runner`の既定値を終了時の摂動幅が範囲の1/6となる減衰する利得へ改め、標準の規模を3,000ペアとした。
  - この設定による2回目の22係数も[STC](measurements/spsa-stage9-20260925-c5-stc.md)と[LTC](measurements/spsa-stage9-20260925-c5-ltc.md)でともに`H1`となり、採用した。
- 評価関数の3計画は2026年9月26日に完了した。2計画は不採用、相対2駒評価は未実装の見送りであり、採用PSTは従来の重みのままである。
  - [教師の混合比](plans/teacher-mixing-ratio.md)では、λ=1.0の候補が[STC](measurements/teacher-mixing-ratio-100-stc.md)で`H0`となった。
  - [探索局面を用いた学習](plans/search-aware-evaluation.md)では、探索局面を半数混ぜた候補Bは対照Aに対して、通常局面だけを深い教師で学び直した対照Aは基点に対して、ともにSTCで`H0`となった（[B対A](measurements/search-aware-pst-b-vs-a-stc.md)、[A対基点](measurements/search-aware-pst-a-vs-base-stc.md)）。
  - [相対2駒評価](plans/relative-pair-eval.md)は、事前登録した判定で配置に依存する残存誤りの根拠が得られず、未実装で見送った。大きな着手の誤りを示した6根は、いずれも100,000ノードの探索の名目深さ内に駒の損得の差が現れる型だったが、各枝の到達深さと原因は追跡で確かめる必要がある（[準備記録](measurements/relative-pair-prep.md)）。

### 次の候補

- 探索部の次の対象は[探索部の反復負け回避](plans/search-repetition.md)である。段階12で他エンジンの定番の改良が7件とも効果を示さなかったので、探索の改良を続ける前に、βカットのうち最初の手によるものが59%にとどまる手の順序付けの[監査](audits/beta-cutoff-first-move-2026-09-27.md)の結果と合わせて、原因を切り分けることも候補になる。
- [不採用だった探索部の改良のSPSAによる再調整](plans/search-revival-spsa.md)は、過去にSTCで不採用となった探索部の改良のうち数値係数を持つ8項目を戻し、既存の探索係数とまとめてSPSAで調整し直して採否を測る。調整した版から8項目だけを取り除いた版とも対局させ、向上が8項目によるのか既存の係数の調整し直しによるのかを切り分ける。測定機の時間は段階2で終われば約10〜17時間、採用まで進めば約16〜55時間であり、着手は利用者の判断による。
- 評価関数で次に着手できるのは[Athénanの計画](plans/athenan.md)である。学習用の探索Descentで採用PSTから追加学習し、学習したPSTを現行のαβ探索に載せた候補と、対局用の探索UBFMsに載せた候補の採否を別々に測る。対照を置かず、学習は同時16対局で48時間を予算とする。着手の条件は満たしている。
  - 評価関数の後続3計画（静止探索の出力、順位の損失、関係補正項）は、いずれも採用候補なしで完了した。順位の損失を関係を表すモデルと組み合わせる方式は、関係補正項の計画が補正項を選ばなかったので、起案するかは利用者の判断による。
  - 総論が挙げた、候補の誤りへの教師予算の重点配分、教師の判定が不安定な根の追加診断、および教師が必要な差を識別できない場合の代替教師の検討は、3計画には含めず、担当計画は未定である。
- [王の安全度と利きに基づく評価特徴の再学習](plans/evaluation-terms-relearning.md)は、[原因調査](audits/heuristic-learning-causes-2026-09-27.md)の結果を受けて、段階9で見送った7項目を採用中の教師で学び直し、新しい自己対局での予測損失で項目を選別してから、土台の費用ごとに段を分けて採否を測る。
- 評価関数の計画と探索部の計画は測定機を共有するので、着手の順序は利用者が決める。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

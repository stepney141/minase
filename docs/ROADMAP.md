# 開発ロードマップ

本書は、minaseの各マイルストーンの状態と次の計画を一覧するダッシュボードである。
各マイルストーンの設計の正は plans/ 配下の設計書、測定記録は measurements/、教訓は lessons/ であり、本書は状態表、現在地、次期候補、および横断的な決定だけを保持する。状態表と現在地は追記せず、マイルストーンの完了時または待機理由の変化時に書き直す。

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
| lishogi Bot接続 | [plans/lishogi-bot.md](plans/lishogi-bot.md) | 起案 | ― |
| 棋力向上段階3 | [plans/strength-stage3.md](plans/strength-stage3.md) | 完了 | 2026年9月5日 |
| 棋力向上段階4 | [plans/strength-stage4.md](plans/strength-stage4.md) | 完了 | 2026年9月6日 |
| 棋力向上段階5 | [plans/strength-stage5.md](plans/strength-stage5.md) | 完了 | 2026年9月7日 |
| PSTの序中盤と終盤の補間 | [plans/tapered-pst.md](plans/tapered-pst.md) | 完了 | 2026年9月8日 |
| HaChu対minaseの条件格子測定 | [plans/hachu-condition-grid.md](plans/hachu-condition-grid.md) | 完了 | 2026年9月9日 |
| Factorization Machineによる2駒関係評価 | [plans/factorization-machine.md](plans/factorization-machine.md) | 完了 | 2026年9月9日 |
| 棋力向上段階6 | [plans/strength-stage6.md](plans/strength-stage6.md) | 完了 | 2026年9月12日 |
| 棋力向上段階7 | [plans/strength-stage7.md](plans/strength-stage7.md) | 進行中 | ― |

## 現在地

直近に完了したマイルストーンは、棋力向上段階6（plans/strength-stage6.md、2026年9月12日）である。
aspiration windows、internal iterative reduction、および最善手安定時の早期終了をSTCとLTCの`H1`で採用し、fail-lowによる延長、最善手交替時の延長、および係数候補`MIN_MOVES = 130`はSTCの`H0`で不採用、置換表のクラスタ化は実装後の固定深さ再生で効果が基準に届かず外し、静的評価の置換表保存とmate distance pruningは診断で見送った。
最終構成は段階開始版に対してSTCで+129.2 Elo、HaChu戦で+334.1 Elo（段階5完了時の+188.5 Eloと信頼区間が重ならない）であった。
internal iterative reductionのLTCは外部のOOMによる時間切れ11件を伴い、該当ペアを除いても`H1`であることから、利用者が規則からの逸脱を記録したうえで採用すると決定した。
その前に完了したFactorization Machineによる2駒関係評価（plans/factorization-machine.md、2026年9月9日）では、現行PSTを固定して2駒関係の補正項を学習したFMが、教師値から対局結果の成分を外すと検証損失と教師誤差を全局面帯で改善し探索速度の費用も1.2%に収まったが、STCで得点率19.6%の`H0`となり不採用とした。候補はブランチ`fm-eval`に保持する。
同日に完了したHaChu対minaseの条件格子測定（plans/hachu-condition-grid.md）では、HaChuを60秒＋1秒加算・256 MBに固定してminaseの持ち時間比と両者の置換表比を変えた8条件と、1手固定時間の2条件を固定200ペアEloで測った。
対等条件で+207.5 Elo、持ち時間1/4で−26.1 Eloの互角、1/8で−205.0 Eloとなり、置換表を16 MBまたは1,024 MBへ変えた4条件はいずれも対等条件と信頼区間が重なり、1手固定時間のminase 1秒対HaChu 2秒でもminaseが+166.2 Eloで強く、HaChuが強いという仮説は支持されなかった。
対局ハーネスには候補と基準ごとの置換表容量オプションと、CECPエンジンへの1手固定時間の対応を追加した。
その前に完了したPSTの序中盤と終盤の補間（plans/tapered-pst.md、2026年9月8日）では、序中盤用と終盤用の2組のPSTを盤上総駒数で線形補間する評価を実装し、既存の世代0と世代1のデータで学習した2端点PSTが開始版に対してSTCとLTCの両方で`H1`となり採用した。
単一PSTの同条件の再学習は開始版と一致したため、構造比較は省いた。
進行中のマイルストーンは、上位計画の棋力向上の段階計画（plans/strength-stages.md）と段階7（plans/strength-stage7.md）である。
10段階のうち段階6までが完了した。
段階7は2026年9月12日に着手し、鏡映共有モデルを[STC](measurements/strength-stage7-mirror-stc.md)と[LTC](measurements/strength-stage7-mirror-ltc.md)のH1で採用した。
試行生成で選んだ手数上限4,000を既定値へ反映し、[世代2の生成](measurements/strength-stage7-gen2-generation.md)では5シードの14,057,872局面を保存した。
全11入力の[識別性診断](measurements/strength-stage7-gen2-identifiability.md)は合格し、保存範囲への射影を加えた[再学習](measurements/strength-stage7-gen2-training.md)も候補の保存まで成功したが、駒除去差分の符号反転が残った。
追加損失の[二乗型の予備比較](measurements/strength-stage7-removal-pilot.md)では適格な正係数がなく、[片側絶対値型の5係数比較](measurements/strength-stage7-removal-absolute.md)ではη1000だけが保存可能性と標本の符号反転0件を満たし、独立監査も合格した。
全11入力の元訓練集合をη1000と学習率0.03で再学習し、元の検証集合で最良エポック10を選び、既存診断を完了した。
同じ6代表局面の352件の駒除去で新しい符号反転は0件となり、量子化の平均絶対誤差とRustの評価および成り差分の一致条件も満たした。
候補の既存検査と深さ5のbenchも完了し、探索速度の低下がないことを確認した。
採用済みの鏡映共有モデルとの[世代2のSTC](measurements/strength-stage7-gen2-stc.md)と[LTC](measurements/strength-stage7-gen2-ltc.md)はともにH1となり、全保存結果の異常0件を独立監査で確認して世代2の重みを採用した。
最終採用重みからの[駒価値の再導出](measurements/strength-stage7-gen2-values-diag.md)は最大相対差8%で更新基準20%以下となり、固定駒価値と探索の余裕値を維持する。
フェーズ5までの採否は確定し、[段階開始版との固定200ペア](measurements/strength-stage7-gen2-elo200.md)は得点率65.74%、Elo +113.19、95%信頼区間[+77.64, +151.19]となり、全400局の異常0件を確認した。
残るHaChuとの固定200ペア測定を実行中であり、段階7は未完了である。
旧構成で中断した段階開始版との37ペアは保持し、最終構成の新しい測定には混ぜない。

待機中のマイルストーンは2件である。
直前局面生成器（plans/predecessor-generator.md）は設計済みだが、2026年8月10日に探索部を先行させると決定してから待機している。`Position`のAPI再編を含むため、着手時期は別途決める。順方向の探索部と評価関数はその完了を前提とせず、いつ再開しても手戻りがない。
早期投了の導入判定（plans/match-early-resignation.md）は、仮想投了が3,000回以上発火する検証群を確保できる記録量に達し、統計契約が確定するまで待機する。

次期候補は、棋力向上段階8（利きマップと評価特徴の拡張）であり、段階7の採用構成と世代2のデータを起点にする。
隣接シードで対局が重複する`rng::derive_seed`の修正は利用者の判断を待つ。
`Threads=4`対2の測定は必要になった時点で plans/lazy-smp.md の手順で実施し、進行中の測定には着手時点のハーネスと測定条件を使って段階ゲートを遡及適用しない。
実lishogiサーバへの接続は、段階2完了時点の棋力（HaChuに対して+149 Elo）を受けて2026年9月4日に公開へ進むと決め、lishogi Bot接続（plans/lishogi-bot.md）として起案した。
着手前に、lishogiの反復裁定の前提条件を規則R1の定義へ取り込む仕様変更と、その変更をSPRTの対象外とする扱いについて裁定を要する。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

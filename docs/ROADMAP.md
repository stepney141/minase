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

## 現在地

直近に完了したマイルストーンは、棋力測定の所要時間削減（plans/match-cost-reduction.md、2026年9月4日）である。
GSPRTの仮説H1を5 Eloから10 Eloへ広げ、LTCの時間制御を`time=60000+600`から`time=60000+200`へ改めた。
新LTCの煙試験は時間切れ0件、1局の平均時間224秒、全手数帯でSTCを上回る到達深さであり、LTCの処理能力は約82ペア/時から約150ペア/時へ改善する。
新条件は以後に開始する段階から適用し、旧条件で開始した測定は`manifest.json`の`h1_elo`と時間制御で識別する。

その直前に完了したマイルストーンは、棋力向上段階2（plans/strength-stage2.md、2026年9月4日）である。
手数に基づく残り手数見積りの予算式、次反復の完了予測による反復継続の判断、および`position`再生の差分適用を採用し、段階開始版との固定200ペアはSTCで+113.9 Elo、HaChu戦は+149.3 Eloであった。

進行中のマイルストーンは、上位計画の棋力向上の段階計画（plans/strength-stages.md）である。
10段階のうち段階2までが完了し、時間管理の適応的な延長と係数の再調整は段階6へ移した。
次の一手は、段階3の個別設計書を起案し、静止探索の質へ進むことである。

待機中のマイルストーンは2件である。
直前局面生成器（plans/predecessor-generator.md）は設計済みだが、2026年8月10日に探索部を先行させると決定してから待機している。`Position`のAPI再編を含むため、着手時期は別途決める。順方向の探索部と評価関数はその完了を前提とせず、いつ再開しても手戻りがない。
早期投了の導入判定（plans/match-early-resignation.md）は、仮想投了が3,000回以上発火する検証群を確保できる記録量に達し、統計契約が確定するまで待機する。

次期候補は、棋力向上段階3の静止探索の質である。
棋力向上の段階計画と並行して進めてよい候補は、採用PSTによる世代2の生成と再学習である。
隣接シードで対局が重複する`rng::derive_seed`の修正は利用者の判断を待つ。
`Threads=4`対2の測定は必要になった時点で plans/lazy-smp.md の手順で実施し、進行中の測定には着手時点のハーネスと測定条件を使って段階ゲートを遡及適用しない。
実lishogiサーバへの接続は、未完成のエンジンを公開の場へ出さない方針から対象外のままとし、探索と評価の成熟後に別マイルストーンとして計画する。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

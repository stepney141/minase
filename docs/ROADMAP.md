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
| Lazy SMP | [plans/lazy-smp.md](plans/lazy-smp.md) | 完了 | 2026年8月23日 |
| テストスイートのspec-first再構築 | [plans/spec-first-tests.md](plans/spec-first-tests.md) | 完了 | 2026年8月15日 |
| 評価関数 | [plans/evaluation.md](plans/evaluation.md) | 完了 | 2026年8月26日 |
| 規則集合の代数的データ型化 | [plans/rules-type.md](plans/rules-type.md) | 完了 | 2026年8月25日 |
| Rust設計監査対応 | [plans/rust-design-audit-remediation.md](plans/rust-design-audit-remediation.md) | 完了 | 2026年8月26日 |
| 評価関数の世代反復 | [plans/evaluation-gen1.md](plans/evaluation-gen1.md) | 完了 | 2026年8月29日 |
| 棋力測定ハーネス基盤の効率化 | [plans/match-harness-efficiency.md](plans/match-harness-efficiency.md) | 完了 | 2026年8月28日 |
| 棋力測定条件の校正と段階ゲート | [plans/match-measurement-calibration.md](plans/match-measurement-calibration.md) | 待機中 | ― |
| 早期投了の導入判定 | [plans/match-early-resignation.md](plans/match-early-resignation.md) | 待機中 | ― |

直前局面生成器は設計済みだが、2026年8月10日に探索部を先行させると決定し、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地：評価関数の世代反復の完了

評価関数の世代反復マイルストーン（plans/evaluation-gen1.md）は2026年8月29日に完了した。
学習PST同士の自己対局で世代1データ10,138,396局面を生成し、世代0と合わせた1,106万局面で再学習した学習PST（コミットaf200b4）を採用した。
直前コミットに対する時間制御GSPRTは得点率90.9%の`H1`、HaChu比較の固定局数Eloは−111（前回−282）である。
同じデータで学習したP型NNUEは固定ノードと時間制御の両方で`H0`となり不採用とした（分岐`nnue-gen1`に保持）。
前マイルストーンの「教師不足が主因」という判定は成り立たず、NNUEが学習しなかった主因は学習器の第1層初期化がclipped ReLUを飽和させていたことにあった。
初期化の修正後も対局では学習PSTに劣り、次の課題はデータ量より教師値と評価の質にある。
次期候補は、採用PSTによる世代2の生成と再学習（Kを固定した尺度効果の切り分けを含む）、NNUEの教師値の質と過学習対策、静止探索の効率化であり、隣接シードで対局が重複する`rng::derive_seed`の修正は利用者の判断を待つ。

Rust設計監査対応は2026年8月26日に完了した。
公開入力のパニック、不正状態を作れる公開型、探索スレッドの未回収および文字列へ失われていた規則エラーを修正し、Rust 1.88.0による全ターゲット検査を含む検証結果をplans/rust-design-audit-remediation.mdへ記録した。

評価関数マイルストーンも2026年8月26日に完了した。
学習PSTを最終採用し、世代0のP型NNUEは固定ノードGSPRTで`H0`となったためツリーから外した。
教師局面の増加と世代反復は上記のマイルストーンとして起案し、静止探索の効率化は別途起案する。

Lazy SMPマイルストーンは2026年8月23日に完了した（記録は plans/lazy-smp.md）。
製品の既定ワーカー数は1のままであり、並列探索はUSIの`Threads`とCECPの`cores`で明示した場合だけ有効になる。

2026年8月24日のHaChu戦の要因分析（plans/match-harness.md）は、等時間で−252 Eloの差の主因を、探索量ではなく1ノードあたりの評価知識と位置づけた。
minaseは探索量でHaChuに劣らず、持ち時間を4倍にすると統計的に互角へ達したためである。
この結果を受け、Lazy SMP完了時に第1候補としていた静止探索の効率化より評価関数を優先し、2026年8月25日に評価関数マイルストーン（plans/evaluation.md）を起案した。
2026年8月26日にフェーズ1（データ生成器と形式）とフェーズ2（学習PST）を完了した。コミット6bc33e1の`selfplay_gen`で世代0の学習データ1,002,722局面（`data/`、gitignore対象）を生成し、そこから学習した線形PST（コミット588363a、レビュー修正7e13888）が固定ノードと時間制御の両GSPRTでv0基準に`H1`（時間制御で得点率62.3%）となり採用した。評価関数のtrait抽象化は採らず、v0は`src/eval/handcrafted.rs`に残す。既存の駒得評価関数v0は利用者の決定により比較用に残す。
続くフェーズ3と4でP型NNUEを実装して測定したが、学習PSTに対する固定ノードGSPRTが`H0`となったため不採用とした。最終ゲートは学習PSTがv0基準に対して既に満たしており、実装と測定の詳細はplans/evaluation.mdに残した。

静止探索の効率化（捕獲専用の手生成、SEE、futility pruning）は評価関数と独立に採否できるため、評価関数の後に別マイルストーンとして起案する。
`Threads=4`対2の測定は、必要になった時点で plans/lazy-smp.md の手順で実施する。
棋力測定ハーネス基盤の効率化は2026年8月28日に完了した。
並列枠の補充、原子的な保存と再開、監査可能な資源記録、および保存記録の再集計を実装し、利用手順を`docs/sprt.md`へ反映した。
実装と検証の詳細はplans/match-harness-efficiency.mdに残した。

時間制御と同時対局数の校正、および段階ゲートはplans/match-measurement-calibration.mdへ分離した。
正例の現行条件200ペアは異常0件で完走したが、負例のP型NNUEは学習PSTに400局全敗してペア得点分散が0となり、校正指標を算出できなかった。
結果を見た後に比較カードを差し替えず、新しいカードと判定契約を事前固定するまで同マイルストーンを待機中とする。

早期投了はplans/match-early-resignation.mdへ分離した。
必要な記録量と検証契約が確定するまで、同マイルストーンを待機中とする。

進行中の評価測定には着手時点のハーネスと測定条件を使い、段階ゲートを遡及適用しない。
実lishogiサーバへの接続は、未完成のエンジンを公開の場へ出さない方針から対象外のままであり、探索・評価の成熟後に別マイルストーンとして計画する。
直前局面生成器は待機中のままとし、`Position`のAPI再編を含むため、着手時期を別途決める。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

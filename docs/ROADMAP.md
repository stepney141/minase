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
| 評価関数の世代反復 | [plans/evaluation-gen1.md](plans/evaluation-gen1.md) | 進行中 | ― |

直前局面生成器は設計済みだが、2026年8月10日に探索部を先行させると決定し、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地：評価関数の世代反復

2026年8月27日に評価関数の世代反復マイルストーン（plans/evaluation-gen1.md）を起案した。
評価関数マイルストーンが残した「教師不足が主因」という判定を、世代1データ1,000万局面の生成と、学習PSTの再学習およびP型NNUEの再学習で検証する。
採否は時間制御GSPRTで決め、固定ノードGSPRTとbench NPSなどの速度の診断指標を併記して、評価の重さが総合の強さに与える影響を記録する。

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
実lishogiサーバへの接続は、未完成のエンジンを公開の場へ出さない方針から対象外のままであり、探索・評価の成熟後に別マイルストーンとして計画する。
直前局面生成器は待機中のままとし、`Position`のAPI再編を含むため、着手時期を別途決める。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合は把握済みだが、性能改善は実測してから判断する。
mimalloc採否とVec割当改善は、2026年8月22日に探索部のbench実測で両方採用して決着した（記録は plans/search.md）。

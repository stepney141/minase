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
| 探索部 | [plans/search.md](plans/search.md) | 進行中 | ― |
| 外部対局接続 | [plans/engine-connectivity.md](plans/engine-connectivity.md) | 完了 | 2026年8月14日 |
| Lazy SMP | [plans/lazy-smp.md](plans/lazy-smp.md) | 未着手 | ― |

直前局面生成器は設計済みだが、2026年8月10日の利用者決定により探索部を先行させ、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地：探索部の残フェーズとLazy SMP準備

対局ハーネスのバイナリ対戦化は2026年8月11日に完了した。
`match_runner`は2つのコミットハッシュの指定だけでビルド・キャッシュからGSPRT判定までを実行でき、機能採否の測定はエンジンの差分内容をハーネスに知らせずに行える。
凍結ベースラインはコミット0045833のdepth=1である。

2026年8月13日に、残タスクの進行順をLazy SMP着手の最速化を基準とする並行運用（探索トラックと接続トラック）として確定した。
接続トラックは2026年8月14日に外部対局接続マイルストーンの完了として決着し、CECP対局進行とXBoard・HaChuの端到端検証まで終えた（記録は plans/engine-connectivity.md）。
実lishogiサーバへの接続は、未完成のエンジンを公開の場へ出さない方針から対象外のままであり、探索・評価の成熟後に別マイルストーンとして計画する。

残る本線は探索トラックである。
plans/search.md に従い、静止探索の実装とGSPRT採否、続いて第2層の逐次採否を進める。
評価関数v0と探索骨格、置換表（2026年8月12日、GSPRTでH1採用）、時間管理と探索呼び出し境界は完了済みである。
各機能の採否は、機能実装コミット対直前コミットの対等条件GSPRTで判定する。

Lazy SMPの着手条件は「単一ワーカー探索の完成」と「CECPの対局進行」であり、後者は達成済みである。
単一ワーカー探索が完成し次第、plans/lazy-smp.md の設計に従い、共有置換表、探索チーム、共通の停止とノード予算、USIの`Threads`、CECPの`cores`を5フェーズで実装し、時間制御のコミット対コミットGSPRTで`Threads=2`対1を採否判定する。
評価関数の本格化はLazy SMPの採否完了後に別マイルストーンとして起案する。
直前局面生成器は待機中のままとし、`Position`のAPI再編を含むため、探索部の進み具合による衝突コストを見て着手時期を別途決める。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合や合法手生成器内の駒ごとのVec割当は把握済みだが、性能改善は探索部の実装時に実測してから判断する。
未コミットのmimalloc実験も判断保留のまま作業ツリーに残し、探索部の実測時に採否を決着させる。

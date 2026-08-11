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
| 探索部 | [plans/search.md](plans/search.md) | 未着手 | ― |
| 外部対局接続 | [plans/engine-connectivity.md](plans/engine-connectivity.md) | 未着手 | ― |

直前局面生成器は設計済みだが、2026年8月10日の利用者決定により探索部を先行させ、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 次期マイルストーン：探索部の残フェーズ

対局ハーネスのバイナリ対戦化は2026年8月11日に完了した。
`match_runner`は2つのコミットハッシュの指定だけでビルド・キャッシュからGSPRT判定までを実行でき、機能採否の測定はエンジンの差分内容をハーネスに知らせずに行える。
凍結ベースラインはコミット0045833のdepth=1である。

探索部の設計書は plans/search.md にあり、自己対局ハーネスとSPRT、評価関数v0と探索骨格、置換表（2026年8月12日、GSPRTでH1採用）までを完了済みである。残りは静止探索、時間管理と探索呼び出し境界、第2層の逐次採否の順で進める。
各機能の採否は、機能実装コミット対直前コミットの対等条件GSPRTで判定する。
外部対局接続は、探索部の時間管理と探索呼び出し境界のフェーズ完了後に着手でき、第2層の逐次採否とは並行できる。
USIのLishogi-Bot経路、CECPの対局状態機械、外部環境による端到端検証の順で進める。
完了時には、Lishogi-Bot経由の非レート対局1局とXBoard仲介のMinase対Minase対局1局を完走し、HaChuとは接続と指し手授受を確認する（詳細は plans/engine-connectivity.md）。
この分離により、探索の棋力検証と外部セッションの正しさを別々の完了条件で判定する。
評価関数の本格化は探索部の完了後に別マイルストーンとして起案する。

## 横断的な記録済みの決定

perftの外部照合は再挑戦しない。
指し手の正準形がエンジン間で異なり、変換層を書くコストが照合の利益に見合わないためであり、外部オラクルの役割は指し手単位のリプレイ照合が担う。

独立したリファクタリングは単独のマイルストーンとしない。
審判ロジック再編は、R0撤去と確認済みの規則不具合修正に必要な構造変更であるため、この決定の対象外である。
`try_make_move`の全手生成による照合や合法手生成器内の駒ごとのVec割当は把握済みだが、性能改善は探索部の実装時に実測してから判断する。
未コミットのmimalloc実験も判断保留のまま作業ツリーに残し、探索部の実測時に採否を決着させる。

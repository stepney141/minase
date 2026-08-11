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
| 直前局面生成器 | [plans/predecessor-generator.md](plans/predecessor-generator.md) | 待機中 | ― |
| 探索部 | [plans/search.md](plans/search.md) | 未着手 | ― |
| 外部対局接続 | [plans/engine-connectivity.md](plans/engine-connectivity.md) | 未着手 | ― |

直前局面生成器は設計済みだが、2026年8月10日の利用者決定により探索部を先行させ、待機中のままとする。
順方向の探索部と評価関数は直前局面生成器の完了を前提とせず、いつ再開しても手戻りがない。

## 現在地

ランダム対局検証ハーネスは2026年8月10日に完了した。
乱数モジュールの分離、`to_sfen`の2欄基本形、`Game`の複製、`random_play`バイナリの4フェーズを実装し、代表9規則セット×300局の計2,700局が異常なし・打ち切り0局で完走した。
テストは200件全緑であり、実施記録は設計書 plans/random-play.md にある。
プロトコル層は、フェーズ1（仕様調査）からフェーズ4（lishogi棋譜リプレイ照合）までを同2026年8月10日に完了した。
リプレイ照合の過程でUSI解析の3升連結正規化の漏れ1件を修正し、lishogiの裸玉裁定と第22条の駒枯れの規則差1件を特定した。
規則差はRULES.mdの第10版改定で解消し、第32条へ新設したローカルルールE3（Lishogi式裸玉即時裁定）の実装により、リプレイ照合はL1+L2+P3+R1+E1+E3で10局・全2,462手が終局裁定まで一致した。
また、規則セットを名前で指定できるプリセット`engine-default`と`lishogi`を導入し（RULES.md第12版・第33条5項および6項）、`--rules`とRuleSetオプションの双方で受理する。前者は実行可能なエンジン既定構成R1、後者は検証済みのLishogi互換構成を表す。HaChu互換セットのプリセット化はフェーズ5の実測検証後に判断する。
プロトコル層は同2026年8月10日に完了した。
フェーズ5では、XBoard内蔵VariantChu駒文字表とHaChu表の照合（21文字完全一致）を経て、CECP複数レグ指し手表記`src/notation/cecp.rs`とCECPプロトコルモジュール`src/protocol/cecp.rs`を実装し、エンジン本体の変更なしで2つ目のプロトコルを追加できることを確認した。
HaChu互換規則セットの実対局照合は、HaChu 0.23が基本移動規則に反する手を自発出力するため不成立と判定し、プリセットは導入しなかった（詳細は docs/protocols/hachu.md 第11章）。
テストは271件全緑であり、経緯は設計書 plans/protocol-layer.md にある。
探索部は2026年8月10日に設計書 plans/search.md を起案し、設計確定済みである。
同設計のフェーズ1（自己対局ハーネスとGSPRT）は2026年8月11日に完了した。
統計モジュール`src/stats.rs`と自己対局バイナリ`src/bin/selfplay.rs`を追加し、ペンタノミアルGSPRTのLLRはfishtest参照値5件と1e-6以内で一致し、モンテカルロによる検定健全性の確認とランダム対ランダムの煙試験を経て、テストは290件全緑である。
フェーズ2（評価関数v0と探索骨格）も同2026年8月11日に完了した。
`src/eval/`（HaChu由来の駒割と最小PST）と`src/search/`（反復深化＋PVS＋MVV-LVA）を新設し、selfplayのプレイヤー指定`--candidate`/`--baseline`とbenchバイナリを追加した。
深さ1エンジンはランダム着手に100ペア200局で全勝し、完了基準の凍結ベースライン構成（深さ1・評価v0・静止探索なし・置換表なし）を確定した。テストは303件全緑である。
次はフェーズ3（置換表）に着手する。
2026年8月11日に、探索部からUSI・CECPのセッション制御と実環境接続を分離し、外部対局接続の設計書 plans/engine-connectivity.md を起案した。
外部対局接続は探索部の探索呼び出し境界（`SearchSnapshot`、`SearchLimits`、停止機構）を前提とし、USI・CECPの対局進行と外部環境との端到端検証を所有する。
直前局面生成器は2026年8月10日に設計書を確定済みだが、利用者決定により実装は待機のままとする。
ブラウザGUI向けUSI照会は、同2026年8月11日に起案から実装完了まで完結した。
第1段階で`WinReason`と`DrawReason`へ`BareKing`を分割新設してE3の裸玉裁定を第22条の駒枯れと区別し、第2段階でUSI拡張`moves`と`state`を`src/protocol/usi.rs`へ追加した。
テストは276件全緑であり、2コマンドの契約は docs/protocols/usi-lishogi.md の「minase固有のUSI拡張」の節に、実施記録は設計書 plans/browser-gui.md にある。

## 次期マイルストーン：探索部

設計書は plans/search.md にあり、自己対局ハーネスとSPRTを最初に作り、評価関数v0、探索骨格、置換表、静止探索、時間管理と探索呼び出し境界、第2層の逐次採否の順で進める。
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

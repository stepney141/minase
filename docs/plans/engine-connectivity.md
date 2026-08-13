# 外部対局接続の設計書

## 実施状況

2026年8月11日に本設計書を起案した。
起案に先立ち、Game・Rules・探索を共有して対局進行をプロトコル別に分ける構成案を批判的にレビューし、時間制御のwire解析と予算計算の所有境界、USIで探索手を永続`Engine.game`へ適用しない非対称の根拠、CECP状態機械、およびHaChuを接続試験に限って使う完了条件を確定した。
同日、Claude Fable 5の設計レビューを実施した。
レビューで指摘された、探索進捗チャネルの欠落、Lishogi-Bot接続時の規則前提と局面同期失敗、終局後のCECP `go`、秒読みを含む時間予算の検証、および実lishogi接続の完了条件を本版へ反映した。
本設計書の起案に伴い、探索部設計書（search.md）の後半にあったUSI・CECPの対局進行、`go`系コマンドのwire処理、実GUI接続の端到端検証の責務を本書へ移した。
実装は未着手である。

2026年8月11日、対局ハーネスのバイナリ対戦化マイルストーン（[match-harness.md](match-harness.md)）が、USIの同期版`go depth|nodes`と`bestmove`応答を前倒しで実装することになった。
本書の適用範囲のうち`go`一式のwire処理は、この同期版を非同期の探索呼び出し境界（`SearchSnapshot`・停止フラグ・チャネル）へ置き換え、時間引数・`stop`・`info`を追加する作業として残る。
wire上の`go depth|nodes`→`bestmove`の契約は前倒し実装から変更しない。

2026年8月12日、実lishogiサーバへの接続をフェーズ3と完了条件から除外した。
未完成のエンジンを公開サーバへ出さないという運用判断によるものであり、Lishogi-Bot経路の検証は実送信系列の台本テストまでとする（「適用範囲」の対象外の項を参照）。

2026年8月12日にフェーズ1（USI対局進行）を完了した。
着手条件である探索部フェーズ5（時間管理と探索呼び出し境界）は、同日の順序入替（search.mdの実施状況を参照）により先行実装済みである。
実装はcodexへ委任し、Claudeが設計整合のレビューと検証を行った。
`go`は`btime`・`wtime`・`binc`・`winc`・`byoyomi`・`movetime`・`depth`・`nodes`・`infinite`を受理してミリ秒の`SearchLimits`へ正規化し、残り時間と加算は現局面の手番側から選ぶ。
裸の`go`と未知引数は従来どおりエラーとし、`go depth|nodes`→`bestmove`のwire契約は不変でmatch_runnerは無改修のまま動作する。
探索は`SearchSnapshot`の複製上で行い、`bestmove`は`Engine.game`へ適用しない。
実バイナリはstdin専用readerスレッドとチャネル駆動の`run_channel`で入力と探索イベントを並行処理し、台本テスト用の逐次`run`も維持した。
`stop`・`gameover`・`quit`・重複`go`・`go ponder`・`ponderhit`の探索中契約、`go infinite`の`bestmove`保留、`position`検証失敗後の同期失敗状態と正当な`position`による回復、`USI_Hash`（既定256MB、探索中でないときのリサイズ）、`info depth <d> score <cp|mate> <v> nodes <n> nps <n> pv ...`行を実装した。
テストは342件全緑（Lishogi-Bot実送信系列、時間引数の先手後手別正規化、同期回復系列、USI_Hash等を追加）、clippy警告なし、fmt通過で、CECP台本13件とlishogiリプレイ照合10局は変化しなかった。
実バイナリの煙試験で、時計引数つき`go`のinfo行と`stop`によるbestmove送出、`go infinite`→`stop`の単一bestmove、探索中`quit`の結果破棄を確認した。

2026年8月13日にフェーズ2（CECP対局進行）を完了した。
実装はcodexへ委任し、Claudeが設計整合のレビューと検証を行った。
CECPアダプターへ担当手番（`Option<Color>`）とforce状態を追加し、`new`（force解除・担当手番は後手）、`force`、`go`（`InGame`以外はエラー）、`usermove`後の自動応手、`result`（探索停止・結果破棄つき確定通知）、`?`（move now）の状態機械を実装した。
エンジン着手は`ApplyMove`→`move`行（既存のレグ分割、じっとは`@@@@`）→`newly_finished`時のみ`RESULT`1回の順で送出し、拒否時は`tellusererror`を出してforce状態へ退避する。
feature宣言を`time=1`・`memory=1`化し、`time`・`otim`（1/100秒）、`level`（分・`分:秒`・加算秒）、`st`（秒）、`sd`（深さ）をミリ秒へ正規化して`SearchLimits`へ写す。`memory <MB>`は探索中でないときの置換表リサイズとして受理する（既定256MB、探索間で再利用、`new`とRuleSet変更でクリア）。
USIと同型のstdin専用readerスレッド＋チャネル駆動`run_channel`を実装して実バイナリのCECP分岐を切り替え、台本テスト用の逐次`run`も維持した。探索中のコマンドは停止を指示するもの（`?`・`force`・`result`・`new`・`quit`）を除きpendingキューへ積んで探索join後に適用し、探索中の`ping`への`pong`はmove行の後に返す順序契約を台本テストで固定した。停止済み探索の遅延結果は探索IDの不一致で破棄し、CECPでは探索`Progress`を出力しない。
実装判断として、`level`第1引数（区切り手数）は構文検証のみ行い予算へは渡さない（`SearchLimits`に残り手数欄がないため）。探索制限が一つも設定されていない`go`は、暗黙の既定値でフォールバックせず`tellusererror`を返す。
テストは357件全緑（`new`→`go`、自動応手、force、move→RESULT順序、`RESULT`後の`go`拒否、move now、探索中`result`/`new`の破棄、`ping`順序、時間正規化、`memory`の11本を追加）、clippy警告なし、fmt通過、既存のCECP台本13件・lishogiリプレイ照合は不変である。

## 目的

本マイルストーンは、探索部が実装する着手決定器を外部の対局環境へ接続し、minaseをUSIとCECPの両経路で自律対局できるエンジンにする。
対象経路は、Lishogi-Botを介したlishogi接続（USI）と、XBoardを介した対局（CECP）の2つである。
通信プロトコルは混ぜず、Game・Rules・探索呼び出し境界を共有したうえで、対局進行だけをプロトコル別のモジュールに実装する。

## 適用範囲

本マイルストーンでは、次の作業を行う。

- USIの通常`go`一式（`btime`・`wtime`・`binc`・`winc`・`byoyomi`・`movetime`・`depth`・`nodes`・`infinite`）と`bestmove`、`stop`、`info`のwire処理を実装する。
- Lishogi-Bot経路の対局進行（毎手の`position`再構築、複製上の探索、`bestmove`返却）を実装する。
- CECPの対局進行状態機械（`force`、`go`、`usermove`、自動応手、`move`、`RESULT`、`result`、`?`、時間制御コマンド）を実装する。
- wire時間指定の解析とミリ秒への単位正規化を実装し、探索部の`SearchLimits`へ接続する。
- `USI_Hash`オプションと`feature memory=1`・`memory`コマンドのwire受理を実装する。
- 台本テストと、外部環境との端到端検証を実施する。

次の項目は対象外とする。

- 探索アルゴリズム、評価関数、`SearchLimits`の予算式、停止機構と探索スレッドの内部実装。探索部（search.md）が所有する。
- ponder。`USI_Ponder`は宣言せず、接続前提としてLishogi-Bot設定でponderを無効にする。
- 評価値による任意投了と引き分け提案。CECPの`feature draw=0`は維持する。
- USI拡張`moves`と`state`。自作ブラウザGUI専用であり、browser-gui.mdが所有する。
- WinBoard/XBoardのhighlight機構による人間の対話入力支援。人間との対局は自作ブラウザGUIが担うため実装しない。
- 実lishogiサーバへの接続。lishogiのBot対局は非レートでも公開サーバ上で閲覧可能であり、未完成のエンジンを公開の場へ出さない方針から、本マイルストーンではLishogi-Botの実送信系列を再現した台本テストまでを検証範囲とする。実接続は探索・評価の成熟後に別マイルストーンとして計画する。
- CUI対局管理マネージャ。
- HaChu互換規則セットの検証とプリセット化（プロトコル層で不成立と判定済み。「HaChuの位置づけ」の章参照）。

互換コマンド、別名、自動プロトコル判別、既定規則、暗黙のフォールバックは追加しない。
単一バイナリと`--protocol usi|cecp`・`--rules`の明示指定を維持する。

## 依存関係

- 探索部（search.md）のフェーズ5「時間管理と探索呼び出し境界」の完了を着手条件とする。本マイルストーンは、`SearchSnapshot`、`SearchLimits`、停止フラグ、探索ID付き進捗・完了チャネルを利用する。
- 探索部フェーズ6（第2層の逐次採否）とは独立であり、並行して進められる。
- ブラウザGUI向けUSI照会（browser-gui.md）とは設計上独立だが、同じ`src/protocol/usi.rs`を編集するため、ROADMAPの順序（ブラウザGUI照会が先行）に従う。
- プロトコル層（protocol-layer.md）は完了済みであり、本マイルストーンはその握手、局面設定、表記変換、規則オプションを変更せずに使う。feature宣言の変更は`time=1`と`memory=1`の2点だけである。

参照の向きは一方向である。
本設計書はsearch.mdの探索呼び出し境界とbrowser-gui.mdの2コマンド仕様を参照するが、search.mdとbrowser-gui.mdは所有境界の注記を除いて本設計書の内容に依存しない。

## 責務分担

| 責務 | 所有する設計書 |
|---|---|
| 探索アルゴリズム、評価関数、置換表とそのサイズ変更・クリアの意味論 | search.md |
| `SearchSnapshot`（内容と構築API） | search.md |
| `SearchLimits`型、soft/hard予算式、ミリ秒単位の契約 | search.md |
| 探索スレッド、停止フラグ、探索ID付き進捗・完了チャネル | search.md |
| wireコマンドの解析と時間単位の正規化（`go`引数、`time`・`otim`・`level`・`st`・`sd`） | 本設計書 |
| プロトコル別の対局進行と、探索の起動・停止のタイミング | 本設計書 |
| `bestmove`・`info`行・`move`行・`RESULT`行の生成と送出順序 | 本設計書 |
| `USI_Hash`・`memory`のwire受理 | 本設計書 |
| 外部環境との端到端検証 | 本設計書 |
| USI拡張`moves`・`state`（自作GUI専用照会） | browser-gui.md |

時間制御の境界は次のとおりとする。
wire形式の解析と単位正規化（CECPの`time`・`otim`は1/100秒、`st`は秒、`level`は分、USIはミリ秒）までが本設計書の責務であり、正規化済みの`SearchLimits`から先（soft/hard予算式、ノード周期の時計チェック、停止）はsearch.mdの責務である。
探索停止も同様に分ける。
停止フラグとjoinの機構はsearch.mdが所有し、どのwireコマンド（USIの`stop`・`gameover`・`quit`、CECPの`?`・`force`・`result`・`new`・`quit`）がいつ停止を指示するかは本設計書が定める。

## 設計判断

| 項目 | 決定 |
|---|---|
| 永続`Engine.game`への適用 | USIは探索手を適用せず、CECPは適用する。根拠は各経路の章に記す。 |
| 探索呼び出しの構造 | `EngineCommand`と`EngineReply`は状態変更の境界のまま変種を追加しない。探索の起動・停止・結果受領はプロトコルモジュールが探索呼び出し境界を直接使う（browser-gui.mdが読み取り専用コマンドで採った整理と同じ）。CECPの着手適用だけが既存の`ApplyMove`を通る。 |
| `go`のライフサイクル契約 | `InGame`以外で届いた`go`は`info string error: ...`を出力し`bestmove`を返さない。詳細と根拠はLishogi-Bot経路の章に記す。 |
| 台本テストの決定性 | 探索を含む台本テストは`depth`または`nodes`固定（CECPは`sd`）で書き、時間ベースの対局は台本にしない。時間制御の実挙動は端到端検証と探索部の自己対局で確認する。 |
| HaChuの用途 | 接続試験（握手と指し手授受の相互運用確認）に限る。規則互換性の基準と完走相手には使わない。 |

## Lishogi-Bot経路（USI）

### 局面同期

lishogiサーバを対局結果の正とする。
Lishogi-Botは毎手`position <初期局面> moves <全着手列>`を送り、minaseは既存の`SetPosition`（`InGame`では現局の原子的再構成）で局面を再構築する。
反復履歴と先獅子状態は着手列の再適用で復元される（プロトコル層の確定事項）。
`usinewgame`には依存しない（Lishogi-Botは送信しないことを調査で確認済み）。

### 探索とbestmove

`go`を受けたら、`Engine.game`から`SearchSnapshot`を写し、`go`引数から正規化した`SearchLimits`とともに探索を開始する。
探索は局面の複製上で行い、返った着手を`bestmove`として送出するだけで、`Engine.game`へは適用しない。
適用しない根拠は2つある。
第1に、次の`position`が全着手列で現局を再構成するため、適用は冗長である。
第2に、`bestmove`が対局に反映されるかどうか（サーバ側の裁定、切断、終局）はlishogiが決めるため、エンジン側で適用すると、サーバの認識と食い違った状態を保持する分岐だけが増える。
`bestmove`送出後も`Engine`は`InGame`のままであり、次の`position`受信で同期する。

`go`が`InGame`以外（`AwaitingStart`・`Finished`）で届いた場合は、`info string error: ...`を出力し`bestmove`を返さない。
USI原典は`go`に`bestmove`を要求するが、この系列は契約外の入力として扱う。
Lishogi-Bot経路は`--rules lishogi`を必須とし、他の規則セットとの裁定互換性を保証しない。
この規則セットでは、minaseの終局裁定はlishogiの裁定とリプレイ照合で一致済みであり、サーバが終局させた対局へ`go`が届くことはない。

`position`の検証に失敗した場合、原子的再構成により直前の`Engine.game`は保持されるが、USIセッションは局面同期失敗状態へ入る。
次に正当な`position`を受理するまで`go`はエラー情報行だけを返し、保持している旧局面から`bestmove`を生成しない。
これにより、規則差または壊れた棋譜を受けた後に、サーバから見て不正な着手を黙って返す経路を閉じる。

`InGame`で`go`を受けた局面には、常に1手以上のルート合法手がある。
審判層が着手適用時に第23条（合法手がない場合の敗北）を裁定するため、合法手のない局面は`SetPosition`または`ApplyMove`の時点で`Finished`になっているからである。
したがって`bestmove resign`を送出する経路は本マイルストーンには存在せず、評価値による任意投了も対象外である。

`go ponder`と`ponderhit`は対象外であり、受信した場合は`info string error: ...`を出力し`bestmove`を返さない。
`go mate`への`checkmate notimplemented`応答は現行のまま維持する。

### 思考情報

反復深化の各イテレーション完了時に、探索がチャネルで返す`Progress`を次の1行へ変換して出力する。

```text
info depth <depth> score <cp|mate> <value> nodes <nodes> nps <nps> pv <move1> <move2> ...
```

PVの指し手は既存のlishogi系USI表記を使う。
深さ1の完了前に探索を打ち切った場合は`info`を省略し、探索開始前に確保した合法手を`bestmove`として返す。
`go infinite`では停止指示（`stop`）まで`bestmove`を出さない。

### 探索中に届くコマンド

探索中の`stop`は探索を停止し、現時点の最善手を`bestmove`として返す。
探索中の`gameover`と`quit`は探索を停止して結果を破棄し、`bestmove`を返さない。
探索中の重複した`go`はエラー情報行とする。
その他のコマンドは探索のjoin後（`bestmove`送出後）に適用する。
停止済み探索の遅延結果は、探索IDの不一致により破棄する。

### 終局責任

終局の裁定と通知はlishogiサーバが行い、minaseは`gameover`の受理（`AwaitingStart`への復帰）だけを行う。
USIにはエンジン発の裁定通知手段がないという既存の整理は変えない。
`state`と`moves`はこの経路では使わない。

## CECP経路（XBoard）

### 状態機械

CECPアダプターは、既存の`Engine`に加えて担当手番（`Option<Color>`）とforce状態を持つ。
コマンドごとの遷移は次のとおりとする。

- `new`：pending規則をcommitし、初期局面で`InGame`へ入る（現行どおり）。force状態を解除し、担当手番を後手（CECPのBlack）とする。
- `force`：探索中なら停止して結果を破棄する。以後は着手を出さず、`usermove`を両陣営分適用するだけの状態になる。担当手番は解除する。
- `go`：対局が継続中の場合に限り、force状態を解除し、現在の手番を担当手番として直ちに探索を開始する。`Finished`または`AwaitingStart`ではエラーを返し、探索を開始しない。
- `usermove`：`Game`へ適用する。適用後、force状態でなく、対局が継続中で、手番が担当手番と一致するなら探索を開始する。
- `result`：時間切れや切断など外部要因を含むGUIからの確定通知として受理する。探索中なら停止して結果を破棄し、`EndGame`で`AwaitingStart`へ戻る。エンジンは自らの裁定と食い違っても異議を唱えない。
- `?`（move now）：探索中なら停止を指示し、その時点の最善手で通常の着手処理を行う。探索中でなければ無視する。

`setboard`で設定した局面からの対局進行も同じ状態機械で動くが、CECPのsetboardは先獅子状態・成り権保留・反復履歴を運べない（プロトコル層の確定事項）ため、反復判定が空の履歴から始まる制約つきの対局になる。
本マイルストーンの端到端検証は、この制約の影響を受けない通常初期局面（`new`）からの対局で行う。

### 着手と結果の順序

探索が返した手は、次の順序で処理する。

1. `ApplyMove`で`Game`へ適用する。審判層で確定済みのルート合法手から選んだ手なので、拒否は探索とプロトコルの間の契約違反である。拒否された場合は`tellusererror`を出してforce状態へ退避する。
2. `move`行として送信する。複数レグは既存のレグ分割（非最終レグの末尾コンマ）を使い、じっとは`@@@@`とする。
3. この着手で`newly_finished`が値を持つ場合に限り、`RESULT {comment}`行を1回だけ送る。既存の再生成禁止の契約を保つ。

`usermove`の適用で終局した場合も、現行どおり`newly_finished`から`RESULT`を1回送る。
エンジンが`RESULT`を送った後にGUIから届く`result`は、前節の確定通知として処理する。

### 時間制御と探索中のコマンド

feature宣言を`time=0`から`time=1`へ変更し、`time`・`otim`（1/100秒）を受理してミリ秒へ正規化する。
`level`（分）・`st`（秒）・`sd`（深さ上限）も受理し、正規化した値を`SearchLimits`へ写す。
`draw=0`は維持し、引き分け提案は扱わない。

探索中に届いたコマンドは、停止を指示するもの（`?`・`force`・`result`・`new`・`quit`）を除き、探索のjoin後に適用する。
探索中の`ping`への`pong`の順序は、先行コマンドの処理完了後に返すという仕様の規定に従い、フェーズ2の実装時に台本テストで固定する。

### 現状維持の範囲

feature交渉、`variant chu`の受理、`undo`・`remove`・`analyze`へのエラー応答、未知コマンドの扱いは現行実装を変更しない。

## 終局状態の通知経路の対比

同じ終局状態でも、通知の経路は接続先ごとに異なる。

- USI（Lishogi-Bot）：エンジンから自動出力しない。裁定の正はlishogiサーバであり、エンジンは内部状態としてのみ保持する。
- CECP（XBoard）：エンジンが`RESULT`行として1回だけpushする。
- 自作ブラウザGUI：`state`要求への応答として返す（browser-gui.md）。

この対比が示すとおり3経路の通知は独立であり、経路をまたぐ通知の共通化は行わない。
共通なのはエンジン内部の裁定（`GameStatus`への一元化）だけである。

## HaChuの位置づけ

HaChu 0.23は、基本移動規則に反する手を自発出力する欠陥がプロトコル層の検証で確定している（docs/protocols/hachu.md第11章）。
したがってHaChuは、規則互換性の基準にも、対局完走の相手にも使えない。
本マイルストーンでは、XBoardがMinaseとHaChuを2つのCECPエンジンとして仲介する接続試験（握手の成立と序盤数手の指し手授受の相互運用確認）に限って使う。
HaChuの不正手をminaseが`Illegal move`で拒否する挙動は、この接続試験では正常系である。
HaChuとの規則互換性の照合と互換プリセットの再検討は、完了条件に含めない（プロトコル層の決定を維持する）。

## 実装フェーズ

分業は従来どおり、実装をcodexへフェーズ単位で委任し、Claudeが設計指示・レビュー・コミットを担当する。

### フェーズ1　USI対局進行

`go`引数の解析と単位正規化、探索呼び出し境界への接続、`bestmove`・`info`・`stop`・`go infinite`、`go`のライフサイクル契約（`InGame`以外と`go ponder`のエラー）、`USI_Hash`を実装する。
台本テストは`depth`または`nodes`固定の`go`で決定的に書き、Lishogi-Botの実送信系列（`setoption`→`position`→`go`の毎手反復、`usinewgame`なし）を含める。
`--rules lishogi`以外の構成、`position`失敗後の`go`、次の正当な`position`による同期回復も検証する。

### フェーズ2　CECP対局進行

状態機械（担当手番・force）、`usermove`後の自動応手、`move`行と`RESULT`の順序、`result`・`?`・時間制御コマンド、feature宣言の`time=1`・`memory=1`化、`memory`コマンドを実装する。
台本テストは`sd`またはノード数上限で決定的に書き、探索中コマンドと`ping`の順序契約、および`RESULT`送出後の`go`拒否を含める。

### フェーズ3　端到端検証

XBoardの自動対局機構でMinase対Minaseの時間制御つき1局を完走させ、時間切れ、反則、および両者の裁定不一致がないことを確認する。
HaChuとの接続試験（握手と序盤の指し手授受）を実施する。
Lishogi-Bot経路の検証は、実送信系列を再現した台本テスト（フェーズ1）までとし、実lishogiサーバへの接続は行わない（「適用範囲」の対象外の項を参照）。
検証の手順と結果は、本設計書の実施状況の章へ記録する。

## 検証

- `cargo test`、`cargo clippy --all-targets`、`cargo fmt --all -- --check`、`git diff --check`を各フェーズの完了条件とする。
- USIとCECPの台本テストが、探索を含む対局進行（`position`→`go`→`bestmove`、`usermove`→`move`→`RESULT`）を決定的な探索条件で再現する。
- 既存の台本テスト、lishogi棋譜リプレイ照合、およびブラウザGUI照会の`moves`・`state`の試験が変化しないことを確認する。

## 完了条件

- USIで、Lishogi-Botの実送信系列を台本テストで完走し、探索手を`Engine.game`へ適用しないことが検証されている。
- `--rules lishogi`の前提、`position`失敗後の探索禁止、および次の正当な`position`による同期回復が台本テストで検証されている。
- CECPで、担当手番とforce状態の状態機械、自動応手、着手適用→`move`行→`RESULT`1回の順序が台本テストで検証されている。
- `InGame`以外の`go`、`go ponder`、探索中の各コマンドの契約が台本テストで固定されている。
- 時間指定のwire解析と単位正規化はプロトコルモジュール、予算計算と停止は探索側という所有境界がコード配置に反映されている。
- XBoard仲介のMinase対Minase時間制御つき1局が、時間切れおよび反則なしに完走し、手順と結果が記録されている。
- HaChuとの接続試験が実施され、結果が記録されている。HaChuとの規則互換性および対局完走は完了条件に含めない。
- `moves`と`state`がLishogi-Bot接続とXBoard接続で使われないことが、文書と台本テストの範囲で保たれている。
- 本節の検証コマンドがすべて成功する。

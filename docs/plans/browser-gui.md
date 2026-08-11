# ブラウザGUI向けUSI照会の設計書

## 実施状況

2026年8月11日に本設計書を起案した。
同日のレビューで裁定理由enumの分割を第1段階として編入し、終局語彙とライフサイクル契約を確定した。
利用者決定により、本マイルストーンは探索部の実装より先に完結させる。

同2026年8月11日に両段階の実装を完了した。
第1段階では`WinReason`と`DrawReason`へ`BareKing`を新設し、`src/core/adjudication.rs`の`bare_king_result`だけがこれを返すよう分割した。
CECPの終局文字列はenumとの1対1写像へ単純化し、`PieceExhaustion`へ"piece exhaustion"、`BareKing`へ"bare king"/"bare kings"を当てた。
E3経路の既存テスト5件とlishogi棋譜リプレイの期待値、`random_play`の理由集計を`BareKing`へ追従させた。
第2段階では`src/protocol/usi.rs`へ`moves`と`state`を追加し、テスト方針の全ケース（集合比較、単一行完全一致、開始前エラー、`gameover`後の規則切替）をインラインテストとして実装した。
`resignation`と`agreement`は設計どおりstatusを出力せず`info string error: ...`で通知する。
検証は`cargo test`276件全緑、`cargo clippy --all-targets`警告なし、`cargo fmt --all -- --check`および`git diff --check`の通過を確認した。
`docs/protocols/usi-lishogi.md`へ「minase固有のUSI拡張」の節を追加し、2コマンドの契約を記録した。

本設計書はminase側のマイルストーンだけを扱う。
別リポジトリへ置くGUIとランチャーの計画は、本設計書の規範に含めない。
したがって、本マイルストーンはGUIの完成を待たず、本設計書の完了条件だけで完了する。

## 目的

開発用GUIが規則を再実装せずに対局を進められるよう、現局面の合法手と表示用状態をUSIで取得できるようにする。

minase側の変更は、合法手を返す`moves`と、規則、盤面、終局状態を返す`state`の2コマンドに限る。
探索と時間管理は[探索部](search.md)、`go`と`bestmove`を外部対局へ接続するセッション制御は[外部対局接続](engine-connectivity.md)で実装する。

`state`が必要なのは、USIにエンジンからGUIへ終局裁定を通知する標準経路がなく、自作GUIがminaseを審判として使うためである。
Lishogi-Botではlishogiサーバーの終局statusを正とし、CECPでは`RESULT`をプッシュ通知するため、どちらも`state`を使わない。
同様に、`moves`は自作GUIが入力候補を表示し、`bestmove`の合法性を検査するための照会であり、外部対局接続の前提ではない。

## 適用範囲

本マイルストーンでは、次の作業を行う。

- `WinReason`と`DrawReason`へ`BareKing`を分割新設する。
- USI拡張`moves`を実装する。
- USI拡張`state`を実装する。
- 2コマンドの試験を既存のUSIテストへ追加する。
- `docs/protocols/usi-lishogi.md`に2コマンドをminase固有拡張として記録する。

次の作業は本マイルストーンに含めない。

- 探索、評価関数、時間管理、`info`、`bestmove`の実装。
- CECPへの同等コマンドの追加。
- Lishogi-BotおよびXBoardとの実対局接続。
- GUI、WebSocket中継、テスト用USIプロセスの実装。
- 完全な対局履歴を表す新しいSFEN形式の設計。
- 拡張コマンドの版番号、機能交渉、互換コマンド、別名の追加。

## 裁定理由enumの分割

E3の裸玉裁定と第22条の駒枯れは、現行実装では同じ`WinReason::PieceExhaustion`と`DrawReason::PieceExhaustion`に写像されている。
両者は規則上排他であり情報欠落はないが、裸玉裁定に`piece-exhaustion`という語を当てるのは意味的に不正確である。
そこで本マイルストーンの第1段階として、`WinReason`と`DrawReason`へ`BareKing`を新設し、E3の裁定経路だけがこれを返すよう分割する。
第22条の駒枯れは、第8項の引き分けを含めて従来どおり`PieceExhaustion`を返す。

CECPの終局文字列は、規則文脈による訳し分けを廃してenumとの1対1写像へ単純化する。
E3経路で`PieceExhaustion`を期待している既存テストとlishogi棋譜リプレイの期待値は、`BareKing`へ更新する。

## `moves`コマンド

GUIは`position`の適用後で対局が進行中のとき、次の1行を送る。

```text
moves
```

minaseは`Game::legal_moves()`の全要素を既存のlishogi系USI表記へ変換し、次の1行を返す。

```text
moves <move1> <move2> ...
```

指し手の順序は契約に含めず、受信側は集合として扱う。
出力を並べ替える処理は追加しない。

`AwaitingStart`または`Finished`では、minaseは次のエラーを返す。

```text
info string error: moves requires an active game
```

空の合法手集合を進行中の正常状態として表す形式は設けない。
審判層が終局を確定した後は`moves`ではなく`state`を問い合わせる。

`moves`は`src/notation/usi.rs`の変換だけを使い、GUI専用の指し手表記を追加しない。
2段移動、居喰い、じっと、成りは既存のUSI表記と同じ文字列になる。

## `state`コマンド

GUIは`position`が成功した後、次の1行を送る。

```text
state
```

minaseは次の1行を返す。

```text
state rules <rules> board <board-sfen> <side> status <status>
```

`<rules>`は現局へ適用中の規則コードを正準順のコンマ区切りで表す。
`<board-sfen> <side>`は`to_sfen`が返す2欄SFENであり、盤面表示と手番確認だけに使う。

`<status>`は次のいずれかとする。

```text
ongoing
win black royal-capture|repetition|piece-exhaustion|bare-king|stalemate|mate
win white royal-capture|repetition|piece-exhaustion|bare-king|stalemate|mate
draw repetition|piece-exhaustion|bare-king
```

`piece-exhaustion`は第22条の駒枯れ、`bare-king`はE3の裸玉裁定を表し、採用規則により排他である。

現行USIから`Game`へ投了または引き分け合意を適用する経路はないため、`resignation`と`agreement`は文法に含めない。
実装がこれらの裁定に遭遇した場合は、statusを出力せず既存形式の`info string error: ...`で通知する。
GUIは`bestmove resign`を自身の対局進行で処理する。

`state`は`InGame`と`Finished`で応答する。
起動直後と`gameover`受信後の`AwaitingStart`では、minaseは次のエラーを返す。

```text
info string error: state requires an active or finished game
```

`gameover`受信後は、エンジンが終局済み局面を内部に保持していても`state`は応答しない。
GUIは終局状態を`state`で確認してから`gameover`を送る。
`moves`が`InGame`だけで応答するのに対し`state`は`Finished`でも応答するという非対称は、この節と`moves`の節の定義による意図的な契約である。

2欄SFENは、先獅子状態、成り権保留、反復履歴を表さない。
したがって、GUIは`board`欄から対局状態を復元せず、対局開始からのUSI着手列を保持して`position startpos moves ...`を再送する。
2欄SFENは盤面表示、手番確認、過去局面の表示に限って使う。

## 実装方針

本マイルストーンは、裁定理由enumの分割（第1段階）と2コマンドの追加（第2段階）の2段で進める。

第2段階の2コマンドは`src/protocol/usi.rs`へ追加する。
既存の`Engine::game()`、`Engine::active_rule_codes()`、`Game::legal_moves()`、`to_sfen`、USI指し手表記をそのまま使う。

`EngineCommand`と`EngineReply`は状態変更の境界なので、読み取り専用の2コマンドのために変種を追加しない。
`src/core/`とCECPの変更は第1段階のenum分割とその追従に限り、第2段階では変更しない。

未知コマンドを無視する既存のUSI方針は変えない。
`moves`と`state`の意味的な不正だけを、既存と同じ`info string error: ...`で通知する。

## テスト方針

既存のUSIテスト（`src/protocol/usi.rs`のインラインテスト）へ次のケースを追加する。

- `position startpos`後の`moves`を空白で分解し、`Game::legal_moves()`のUSI表記集合と一致することを確認する。
- 2段移動、居喰い、じっと、成りを含む既存の全合法手往復試験が、`moves`で使う表記を引き続き覆うことを確認する。
- `state`がactive規則、2欄SFEN、進行中または終局状態を正確に出力することを確認する。
- 対局開始前の`moves`と`state`が明示的なエラーを返すことを確認する。
- `gameover`後に`RuleSet`を変更して次局を開始し、`state`の`rules`が新しい規則を返すことを確認する。
- 既存のUSI台本とlishogi棋譜リプレイが変化しないことを確認する。

`moves`の試験は集合を比較し、列挙順を固定しない。
`state`は単一行の契約なので、出力を完全一致で確認する。

## 検証

- `cargo test`を実行する。
- `cargo clippy --all-targets`を警告なしで通す。
- `cargo fmt --all -- --check`を通す。
- `git diff --check`を通す。

## 完了条件

- `WinReason`と`DrawReason`の`BareKing`分割が完了し、E3の裁定だけが`bare-king`として出力される。
- `moves`が現局面の`Game::legal_moves()`を既存USI表記で返す。
- `state`がactive規則、表示用2欄SFEN、終局状態を1行で返す。
- `src/core/`とCECPの変更が裁定理由enumの分割とその追従に限られ、2コマンド自体は`src/protocol/usi.rs`に閉じている。
- 既存のUSI台本とlishogi棋譜リプレイを含む全テストが成功する。
- `docs/protocols/usi-lishogi.md`にminase固有拡張として記録されている。

# butterfly historyと捕獲履歴は対局内で減衰させて持ち越し、continuation historyとpawn historyは探索チームで共有して減衰させずに持ち越し、low ply historyは探索ごとに初期化する

## ステータス

置き換え済み。決定日は2026年10月19日。[butterfly historyと捕獲履歴は対局内で減衰させて持ち越し、continuation history、pawn history、および局面を鍵にした補正表は探索チームで共有して減衰させずに持ち越し、continuation correction historyはワーカーごとに持ち越し、low ply historyは探索ごとに初期化する](../carry-histories-and-correction-histories-within-a-game.md)によって置き換えられた。

## コンテキスト

[旧ADR](carry-history-tables-within-a-game.md)は、2026年10月10日に、butterfly history（手番、移動元、移動先ごとの静かな手の成績）と捕獲履歴を、置換表と同じく探索をまたいで受け渡し、新しい根の探索のたびにそれぞれの減衰率を掛け、新規対局などの時点で初期化すると決めた。
旧ADRは、PSTとFMを既定の評価関数としていた時期の測定に基づき、continuation historyの設計を別の設計書に委ねていた。

[historyの形の設計書](../../plans/archived/history-forms.md)は、2026年10月17日のエンジンの開発者の決定（形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れる）に従い、continuation history、pawn history、およびlow ply historyを新設した（[表の構成のADR](../look-up-quiet-histories-in-butterfly-continuation-pawn-and-low-ply-tables.md)）。
Stockfishは、根の探索のたびにbutterfly historyだけを729/1024倍し（`src/search.cpp:341-343`）、continuation historyとpawn historyは同じNUMAノードのワーカーで共有して減衰させず（`src/history.h:201-225`）、low ply historyは反復深化のループに入る前に102にする（`src/search.cpp:339`）。

## 決定

butterfly historyと捕獲履歴は、旧ADRと同じくワーカーごとに持ち、新しい根の探索のたびにそれぞれ`HistoryDecay`%と`CaptureHistoryDecay`%を掛けて減衰させ、対局内で持ち越す。
初期値は、Stockfishと同じくbutterfly historyを−5、捕獲履歴を−742とする（`src/search.cpp:714-715`）。

continuation historyとpawn historyは、探索チームの全ワーカーで共有する1組の表とし、対局内で減衰させずに持ち越す。
初期値は、Stockfishと同じくcontinuation historyを−586、pawn historyを−1338とする（`src/search.cpp:717-727`）。
要素には不可分の16ビット整数を使って`Relaxed`順序で読み書きし、ワーカーの間で競合した更新が失われることを許す。
pawn historyのエントリの数は、1024にワーカー数以上の最小の2のべきを掛けた数とする。

4つの表は、新規対局、受理された規則変更、受理された置換表容量の変更、ワーカー数の変更、および引き分けの評価値`DrawValue`の変更の時点で初期値に戻し、拒否された設定変更では値を保つ。
low ply historyはワーカーごとに持ち、探索ごとにワーカーの探索状態を作る時点で初期値の102にし、反復深化と窓の読み直しでは初期化しない。
killer表と補正表は、旧ADRと同じく持ち越さない。

## 検討した代案

continuation historyをワーカーごとに持つ案は、表が約140 MBあり、ワーカー数に比例してメモリが増えるので採らなかった。
共有の表をStockfishと同じくNUMAノードごとに分ける案は、minaseがNUMAの配置を扱わないので採らなかった。
continuation historyとpawn historyも根の探索のたびに減衰させる案は、Stockfishが減衰させるのはbutterfly historyだけなので採らなかった。
continuation historyを根の探索ごとに初期化する案は、[棋力向上段階5](../../plans/archived/strength-stage5.md)の[診断](../../measurements/strength-stage5/strength-stage5-continuation-bench.md)で、根の探索ごとに初期化する寿命では表が疎だったので採らなかった。
pawn historyのエントリの数をワーカー数によらず固定する案は、Stockfishが表の大きさをワーカー数に比例させるので採らなかった。
USIの`Threads`の上限256では表だけで約4.4 GBになるが、多数のワーカーを使う機械は大きなメモリも持つと判断した。
low ply historyを根の探索の窓ごとに初期化する案は、Stockfishが反復深化のループの外で初期化するので採らなかった。

## 結果

表の寿命は`crates/minase/src/search/alphabeta/history.rs`の`HistoryTables`、`ContinuationHistory`、および`PawnHistory`と、`team.rs`の減衰のdocコメントにあり、USI層の初期化の時点は`crates/minase/src/protocol/usi.rs`にある。
1プロセスの常駐メモリは、ワーカー1つで約179 MB増えた（[探索ノード数と探索速度の記録](../../measurements/history-forms/history-forms-bench.md)）。
lishogiのBotは`Threads`を4としているので、1局あたりの増分は約209 MBの見込みである。

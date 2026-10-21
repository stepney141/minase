# butterfly historyと捕獲履歴は対局内で減衰させて持ち越し、continuation history、pawn history、および局面を鍵にした補正表は探索チームで共有して減衰させずに持ち越し、continuation correction historyはワーカーごとに持ち越し、low ply historyは探索ごとに初期化する

## ステータス

承認済み。決定日は2026年10月20日。

## コンテキスト

[旧ADR](archive/carry-histories-within-a-game-and-share-continuation-and-pawn-histories.md)は、2026年10月19日に、butterfly historyと捕獲履歴をワーカーごとに持って根の探索のたびに減衰させて対局内で持ち越し、continuation historyとpawn historyを探索チームの全ワーカーで共有して減衰させずに持ち越し、low ply historyを探索ごとに初期化すると決めた。
旧ADRは、correction historyの補正表を、その前の[ADR](archive/carry-history-tables-within-a-game.md)と同じく持ち越さないとしていた。
持ち越さない根拠は、既定の評価関数がPSTだった時期に、補正をfutility pruningにだけ使う前提で測った発動率（STC相当で0.69%）であり、補正を枝刈りと減深のすべての判定に使うようになった後には測っていない。

[correction historyの表の設計書](../plans/archived/correction-history-tables.md)は、補正表を局面を鍵にした5つの表とcontinuation correction historyに改めた（[補正表のADR](sum-material-placement-and-continuation-correction-histories-and-apply-them-to-the-qsearch-stand-pat.md)）。
2026年10月17日のエンジンの開発者の決定により、同書では、表の大きさと共有の仕方をStockfishと同じにし、表を対局の間減衰させずに持ち越す。
Stockfishは、局面を鍵にした補正表を同じNUMAノードのワーカーで共有し、その大きさを65,536にワーカー数以上の最小の2のべきを掛けた要素数とし（`src/history.h:40, 201-225`、`src/thread.cpp:213`）、continuation correction historyはワーカーごとに持ち（`src/search.h:335`）、どちらも新規対局でだけ初期化する（`src/search.cpp:713-736`）。

## 決定

butterfly historyと捕獲履歴は、旧ADRと同じくワーカーごとに持ち、新しい根の探索のたびにそれぞれ`HistoryDecay`%と`CaptureHistoryDecay`%を掛けて減衰させ、対局内で持ち越す。
continuation historyとpawn historyは、旧ADRと同じく探索チームの全ワーカーで共有する1組の表とし、対局内で減衰させずに持ち越す。
初期値と、pawn historyのエントリの数の決め方は旧ADRのままとする。

局面を鍵にした5つの補正表（材料、pawn、minor、先手と後手のnon-pawn）は、探索チームの全ワーカーで共有する1組の表とし、対局内で減衰させずに持ち越す。
5表は1つの配列に束ね、手番側ごとの要素数を、65,536にワーカー数以上の最小の2のべきを掛けた数とする。
continuation correction historyはワーカーごとに持ち、対局内で減衰させずに持ち越す。
共有の表の要素には不可分に読み書きする16ビット整数（`AtomicI16`）を使って`Relaxed`順序で読み書きし、ワーカーの間で競合した更新が失われることを許す。

これらの表は、新規対局、受理された規則変更、受理された置換表容量の指定（同じ値の再指定を含む）、引き分けの評価値`DrawValueBlack`または`DrawValueWhite`の変更、およびワーカー数の変更の時点で初期値に戻し、拒否された設定変更と同じワーカー数の再指定では値を保つ。
low ply historyは、旧ADRと同じくワーカーごとに持ち、探索ごとに初期値の102にする。
killer表は持ち越さない。

## 検討した代案

補正表を持ち越さない旧ADRの形は、表を対局の間持ち越すというエンジンの開発者の決定に反し、その根拠の発動率も補正をfutility pruningにだけ使う前提の値なので採らなかった。
補正表もbutterfly historyのように根の探索のたびに減衰させる案は、Stockfishが減衰させないので採らなかった。
continuation correction historyも共有する案は、Stockfishがワーカーごとに持つので採らなかった。
共有の表をStockfishと同じくNUMAノードごとに分ける案は、旧ADRと同じく、minaseがNUMAの配置を扱わないので採らなかった。
continuation historyとpawn historyの持ち方について旧ADRが棄却した案は、棄却したままとする。

continuation correction historyはワーカー1つあたり約36 MBであり、USIの`Threads`の上限256では約9.2 GBになる。
pawn historyの約4.4 GBと合わせて大きいが、旧ADRと同じく、多数のワーカーを使う機械は大きなメモリを積んでいると判断した。
`Threads`の上限を受理するかのテストは、256ワーカーの表を確保しないように、上限を判定する関数を直接呼んで行う。

## 結果

表の寿命は`crates/minase/src/search/alphabeta/history.rs`の`HistoryTables`、`correction.rs`の`CorrectionValues`と`ContinuationCorrectionHistory`、および`team.rs`の減衰のdocコメントにあり、USI層の初期化の時点は`crates/minase/src/protocol/usi.rs`にある。
`minase dev bench`は局面ごとに`HistoryTables::clear`を呼ぶので、持ち越しと共有はbenchの単一ワーカーの結果を変えない。
1プロセスの最大常駐メモリは、ワーカー1つで約39 MB増えた（[benchの記録](../measurements/correction-history-tables/correction-history-tables-bench.md)）。
lishogiのBotは`Threads`を4としているので、1局あたりの増分は約149 MB（ワーカー4つ分のcontinuation correction historyの約144 MBと、共有の補正表の約5 MB）の見込みである。

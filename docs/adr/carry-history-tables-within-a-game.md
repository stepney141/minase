# butterfly historyと捕獲履歴を対局内で減衰させて持ち越す

## ステータス

承認済み。決定日は2026年10月10日。

## コンテキスト

minaseは当初、手の成績の表を`go`のたびに0から始めていた。
Stockfishは手の成績の表を対局の間は消さずに持ち越し、主historyだけを新しい根のたびに縮めている（[段階12の設計書の参考資料](../plans/strength-stage12.md#参考資料)）。
[棋力向上段階12](../plans/strength-stage12.md)は、butterfly history（手番、移動元、移動先ごとの静かな手の成績）、補正表、および捕獲履歴の持ち越しを、保存済みの対局の着手列に沿った再生で[診断](../measurements/strength-stage12-activation-diag.md)した。
butterfly historyが静かな手の段の順序を変えたノードは、STC相当で96.35%、LTC相当で99.43%だった。
ここで段とは、手選択器が置換表の手、捕獲手、killer手、および残りの静かな手を種類ごとにまとめて返す区切りをいう。
補正表がfutilityの判定で枝刈りの可否を変えた手は0.69%と0.80%で、基準の5%に届かなかった。
butterfly historyだけを根ごとに3/4倍して持ち越す版は、STCが2,539有効ペア、得点率49.8%の`H0`だった（[測定記録](../measurements/strength-stage12-history-carry-stc.md)）。
2026年10月4日には、[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)が減衰率をSPSAで調整し直し、butterfly historyの持ち越しを一括で採用した8項目の1つとして採用した（[採用のADR](adopt-revived-search-items-as-a-bundle.md)）。
2026年10月10日には、[手の順序付けの改善](../plans/move-ordering.md)が捕獲履歴も同じ経路で持ち越す候補を加え、[診断](../measurements/move-ordering-diag.md)で、捕獲手の段のうち順序が変わるものがSTC相当で10.40%あることを確かめた。
この候補を含む[手の順序付けの改善の7候補](bound-capture-history-and-use-it-in-qsearch.md)の構成は、STCとLTCの`H1`で一括して採用した（[STC](../measurements/move-ordering-all-stc.md)、[LTC](../measurements/move-ordering-all-ltc.md)）。
本ADRは、10月4日に採用したbutterfly historyの持ち越しを、10月10日に捕獲履歴へ広げた現行の形を記録する。

## 決定

butterfly historyと捕獲履歴を、置換表と同じく探索の間で受け渡し、新しい根の探索を始めるたびにそれぞれの減衰率を掛け、補助ワーカーの表はワーカーの番号ごとに持ち越す。
表は新規対局、受理された規則変更、受理された置換表容量の変更、およびワーカー数の変更の時点で初期化し、拒否された設定変更では保つ。
killer表と補正表は持ち越さず、探索ごとに初期化する。

## 検討した代案

補正表を持ち越す案は、上記のとおり発動率が基準に届かなかったので見送った。
killer表を持ち越す案は、killer表がplyに結び付いた値であり、根が1手進むとplyの意味がずれるので採らなかった。
規則変更の後も表を保つ案は、規則が変われば合法手と探索値が変わり、以前の規則で学んだ成績が当てはまらないので採らなかった。

## 結果

減衰率は`crates/minase/src/search/alphabeta/params.rs`の`HistoryDecay`と`CaptureHistoryDecay`であり、表の受け渡しの詳細は`history.rs`の`HistoryTables`のdocコメントにある。
`CaptureHistoryDecay`は`HistoryDecay`と同じ値から始めた既定値のままであり、次のSPSAの調整の対象である。
対局ハーネスは局ごとにエンジンを起動し直すので、局をまたぐ持ち越しは起きない。
持ち越しにより、counter move historyとcontinuation historyの再診断の前提が整い、両表の設計は別の設計書に委ねられた。

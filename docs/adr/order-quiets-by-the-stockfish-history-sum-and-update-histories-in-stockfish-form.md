# 静かな手はStockfishと同じhistoryの和と直接の王手と脅威の項で並べ、historyはStockfishの上限つき1次式と更新の場面で更新する

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

[旧ADR](archive/order-quiets-with-malus-and-pst-tiebreak.md)は、2026年10月10日に、β打ち切りを起こした静かな手に加点し、同じノードで先に探索した静かな手を減点し、加点と減点を別の尺度で深さの2乗に比例させて重力式で更新し、history値が同じ手を評価関数の線形経路の差分と生成順で並べると決めた。
旧ADRは、PSTを既定の評価関数としていた時期の測定に基づき、同点破りの効果をNNUEで測り直していないことと、尺度がSPSAの調整を待つ既定値であることを記していた。
深さの2乗の加点は、起点の既定値で深さ10でもbutterfly historyの上限の約7.7%、深さ2では約0.3%にとどまり、Stockfishの静かな最善手への加点（深さ10で約15%、深さ2で約2.3%）より小さかった。

2026年10月17日のエンジンの開発者の決定により、形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れ、minaseにあってStockfishにない仕組みのうち中将棋の診断や測定で決めたものは残す。
[historyの形の設計書](../plans/history-forms.md)は、この決定に従って、静かな手の並べ替えとhistoryの更新をStockfishの形へ寄せた。

## 決定

静かな手の段の並べ替えの値は、butterfly historyとpawn historyの2倍、1、2、3、4、6手前の文脈のcontinuation historyの和、根からのplyが5未満ならlow ply historyの`8/(1 + ply)`倍、直接の王手への加点、および価値の低い相手の駒の利きへの出入りの項の和とする（Stockfish `src/movepick.cpp:233-251`）。
同点破り（線形経路の差分、生成順）は旧ADRから引き継ぐ。

直接の王手は、相手の王駒がちょうど1枚あり、現在の占有のまま到達升に着手後の駒種の駒を置くと、その駒の疑似利きが王駒に届く手である。
Stockfishの`check_squares`と同じく、ノードで静かな手の段を開くときに駒種ごとの升の集合を必要な駒種についてだけ求め、開き王手と王駒が2枚の場合は加点しない。
加点はSEEがStockfishの尺度で−75以上の手に限り、規則に依存して判定できない交換は閾値以上として扱う。
価値の低い駒の脅威は、相手の駒のうち盤上の駒コードの駒価値が動かす駒より厳密に小さいものの疑似利きの和集合とし、王駒の手には使わない。

historyは、最善値が探索開始時のαを上回ったすべてのノードで更新する（`src/search.cpp:1602-1608`）。
加点は`min(傾き × d − 切片, 上限)`に、最善手が置換表の手なら定数を、このノードへ至った手のstatScoreの1/28を足し、零窓のノードでは探索済みの手の数に比例して増やす（同2004-2011行）。
減点は`min(傾き × d − 切片, 上限)`とする。
静かな最善手にはbutterfly、low ply、continuation、およびpawn historyへ加点の899/1024を与え、探索済みの静かな手には減点の1159/1024を1手ごとに逓減させて与える（同2013-2023行、2049-2089行）。
探索済みの手は、ノードの手数が32以下で、読んだ時点でαを上回らなかった手に限る（同75行、1578行）。
このほか、置換表の打ち切りの条件が成り立ったとき、fail lowのとき、前の最善手が反駁されたとき、直前の手の前後で静的評価が変わったとき、およびLMRで減深した探索の値がαを上回ったときにも、Stockfishと同じ量で更新する（同902-910行、1001-1012行、1423行、1611-1643行、2031-2035行）。
killer表の更新は、旧ADRまでと同じくβ打ち切りのときだけ行う。
各表は、更新量をその表の上限で切り詰めてから重力式で更新する。

傾き、切片、上限の手前で効く量、重み、および倍率は係数としてSPSAで調整し、表の上限、平均の重み、および更新の内部の定数はStockfishの値で固定する。

## 検討した代案

深さの2乗の加点と減点を残す案は、Stockfishの形に合わせる決定に反するので採らなかった。
同点破りを外す案は、旧ADRの直接対局で決めた仕組みなので、エンジンの開発者の決定により採らなかった。

並べ替えの王手の判定に、RULES.md第8条どおりの`Position::gives_check`をすべての静かな手で使う案は、実装したうえで測ると探索時間の約19%を占めたので採らなかった（[探索ノード数と探索速度の記録](../measurements/history-forms/history-forms-bench.md)）。
直接の王手の集合はStockfishの判定と同じ範囲であり、枝刈りでは規則どおりの`gives_check`を使い続ける（[王手の判定のADR](judge-gives-check-on-the-post-move-occupancy-without-precomputation.md)）。

Stockfishは、現在のノードか直前の手のノードが王手を受けていれば、静的評価の変化による更新とfail lowの加点の一部の項を行わない（`src/search.cpp:1001, 1618-1619`）。
王手の局面ではStockfishの静的評価が2手前の値の写しだからである（同848行）。
minaseはすべてのノードで静的評価を求めるので、この条件を置かなかった。
王駒に相手の疑似利きが届くかどうかで代わりの条件を置く案は、親と子の両方の判定を毎ノード求めて保持する費用がかかるので採らなかった。

## 結果

並べ替えと更新の式は`crates/minase/src/search/alphabeta/ordering.rs`の`quiet_ordering_value`、`DirectCheckSquares`、`LesserPieceThreats`、`HistoryUpdateCoefficients`、`update_quiet_histories`、および`update_continuation`のdocコメントにあり、更新の場面は`negamax.rs`と`root.rs`にある。係数の現行値は`params.rs`にある。
表の形と寿命は、[表の構成のADR](look-up-quiet-histories-in-butterfly-continuation-pawn-and-low-ply-tables.md)と[持ち越しのADR](carry-histories-within-a-game-and-share-continuation-and-pawn-histories.md)が定める。
変更の効果は、[SPSAの記録](../measurements/history-forms/history-forms.md)と[STCの記録](../measurements/history-forms/history-forms-stc.md)にある。

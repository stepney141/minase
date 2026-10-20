# 静かな手のhistoryは、butterfly表、continuation history、pawn history、およびlow ply historyで引く

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

[旧ADR](archive/keep-only-the-butterfly-history-table.md)は、2026年9月7日に、静かな手のhistoryを手番側、移動元、および移動先で引くbutterfly表だけで構成し、直前の手と組み合わせる表は対象外とすると決めた。
旧ADRは、PSTの時期の測定に基づき、駒種と移動元と移動先の4次元の表が疎だという判断は、表を根の探索ごとに初期化していた時期のものであり、再提案するときは現行の表の寿命で測り直す必要があると留保していた。
[手の順序付けの改善](../plans/archived/move-ordering.md)の[診断](../measurements/move-ordering/move-ordering-diag.md)では、表を対局内で持ち越し、根の探索ごとに23%へ減衰させる条件で、直前の手を文脈とする表が静かな手の順序を変えるノードがSTC相当で44.6%（1手前）と61.3%（2手前）あった。

2026年10月17日のエンジンの開発者の決定により、形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れる。
Stockfishは、butterfly historyのほかに、1、2、3、4、6手前の手を文脈とするcontinuation history、歩兵の配置の鍵で引くpawn history、および根に近いplyで引くlow ply historyを持つ（`src/history.h:128-146`、`src/search.cpp:1140-1142`）。

## 決定

静かな手のhistoryを、butterfly表、continuation history、pawn history、およびlow ply historyの4つの表で構成する。
continuation historyは、文脈の手が捕獲手か、文脈の手を指したのが表を引く手と同じ側か、文脈の手の着手前の駒種と到達升、および現在の手の着手前の駒種と到達升で引く1組の表とし、1、2、3、4、6手前の文脈は同じ表の中の区画を引く。
表を引く手を指す側が後手なら、文脈の手と現在の手の升をともに180度回転する。中将棋の初期配置と規則が、180度の回転と手番の入れ替えについて対称だからである。
文脈の手がない項（根より前のply、null moveの直後）は参照も更新もしない。
pawn historyは、規則書が「価値ある駒」から除く2種（RULES.md第3条第11項）である歩兵と仲人の配置の鍵で引き、手番側と駒種を合わせた駒コードと、到達升で引く表を持つ。鍵は盤の絶対の配置なので回転しない。
low ply historyは、根からのplyが5未満のノードで、plyと移動元と到達升で引く。

## 検討した代案

butterfly表だけを使う旧ADRの形は、Stockfishにだけある項目は入れるという決定に反するので採らなかった。
駒種と到達升で引く表（piece-to history）を加える案は、Stockfishにもなく、[棋力向上段階5](../plans/archived/strength-stage5.md)のSTCで`H0`だったので、旧ADRと同じく採らなかった。
continuation historyを、文脈の手を指した局面が王手を受けていたかで分ける案は、Stockfishにある次元だが（`src/search.cpp:690-691`）、minaseは王手を受けている局面を区別しない（王駒を利きへ置く手も合法であり、RULES.md第8条第3項から第5項）ので採らなかった。
文脈の手が捕獲手かどうかの次元を持たず、表を約70 MBに半減する案は、Stockfishとの一致を崩すので採らなかった。
文脈の手がない場合にStockfishと同じく初期値の区画を引く案は、引く区画を決める文脈の手がないので採らず、その項を足さない。

## 結果

表の添字は`crates/minase/src/search/alphabeta/ordering.rs`の`continuation_index`、`history.rs`の`ContinuationHistory`、`PawnHistory`、`pawn_key`、および`pawn_key_after_move`のdocコメントにあり、着手の記録は`searcher.rs`の`MoveRecord`にある。
表の使い方は[並べ替えと更新のADR](order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md)、[LMRのADR](reduce-all-later-moves-and-root-moves-by-lmr-with-stat-score-and-correction.md)、および[continuation historyによる枝刈りのADR](prune-quiet-moves-by-continuation-history-and-shift-lmr-depth-by-history.md)が、寿命は[持ち越しのADR](carry-histories-and-correction-histories-within-a-game.md)が定める。
[correction historyの表の設計書](../plans/correction-history-tables.md)のC2は、pawn historyの鍵を再利用する。

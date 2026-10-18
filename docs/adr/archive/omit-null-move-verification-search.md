# null moveのverification searchを実装しない

## ステータス

置き換え済み。決定日は2026年9月6日。[null moveの打ち切りは残り深さ16以上で検証探索により確かめる](../verify-null-move-cutoffs-at-remaining-depth-16-and-above.md)によって置き換えられた。

## コンテキスト

null move pruningは、手番を相手へ渡しても縮めた深さの探索の値がβ以上なら、そのノードを打ち切る手法である。
この打ち切りは、手番を渡すことが不利になる局面（zugzwang）で誤る。
verification searchは、null moveの打ち切りを、null moveを禁じた縮めた深さの通常探索で確かめ、値がβを下回れば通常の探索へ戻す機構である。
[棋力向上段階4](../../plans/archived/strength-stage4.md)は、確かめた探索がβを下回る割合を矛盾率とし、1%未満なら実装しないという基準を測定前に定めた。
中将棋は駒数が多くzugzwangが稀なので、見送りになる見込みが高いと考えていた。
[診断bench](../../measurements/strength-stage4/strength-stage4-nmp-verify-bench.md)は、futility pruningを採用した構成で、縮めた深さが1以上のnull moveの打ち切りを数えた。
根の深さ5では723回、根の深さ7では7,455回の打ち切りがあり、矛盾はどちらも0件だった。

## 決定

null move pruningにverification searchを加えない。
null moveの縮めた探索がβ以上を返せば、そのまま打ち切る。

## 検討した代案

縮めた深さが1以上の打ち切りを、null moveを禁じた縮めた深さの通常探索で確かめる案は、矛盾率が0%で結果を変えないので実装しなかった。

## 結果

null move pruningの実装は`crates/minase/src/search/alphabeta/negamax.rs`にあり、検証の探索を持たない。
[不採用だった探索部の改良の再調整](../../plans/archived/search-revival-spsa.md)も、verification searchは係数ではなく発動率の低さで見送った改良なので、対象から外した。
診断は、null moveの減深量がSPSAで調整される前で、評価関数がNNUEへ置き換わる前の構成で行った。
減深量が大きくなるほど縮めた探索は浅くなるので、再提案するときは現行の構成で矛盾率を測り直す必要がある。

# null moveより後の局面だけで反復を検出する

## ステータス

承認済み。決定日は2026年9月29日。

## コンテキスト

null move pruningは、手番を仮に相手へ渡して浅く読み、それでもβを超えるなら枝を打ち切る手法である。
探索は、対局履歴と探索経路にある局面キーとの一致を同一局面の再現とみなし、引き分けの値を返す。
2026年9月28日の[探索部の不具合監査](../audits/search-bugs-2026-09-28.md)は、null moveの直後に相手がじっとを指すと盤面が元に戻り、これを反復として引き分けと判定していることを指摘した。
null moveは実際の対局に現れないので、この引き分けは実在しない。
監査によれば、benchを深さ9で探索すると、null moveをまたぐ反復が1,986件あった。
Stockfishは反復の検出を直前のnull moveまでに限り、ElephantEyeも遡りをnull moveで止める（[反復の先行実装の調査](../research/search-repetition-prior-art.md)）。
[探索部の不具合修正](../plans/search-bug-fixes.md)は、この修正を探索値の正しさの回復と位置づけ、+10 Eloの向上ではなく非劣性で採否を判定した。
masterを候補、修正を基準にしたGSPRTは、[STC](../measurements/search-bug-fixes-null-move-noninferiority-stc.md)でmasterの得点率45.7%、[LTC](../measurements/search-bug-fixes-null-move-noninferiority-ltc.md)で48.0%となり、いずれも`H0`であった。
修正によって、benchの深さ9の総ノード数は8.0%減った（[測定記録](../measurements/search-bug-fixes-bench.md)）。

## 決定

null moveの部分木の中では、直近のnull move直後の局面以降に探索経路へ積んだ局面キーとの一致だけを反復とする。
null move直後の局面キーは探索経路へ積むが、そのキー自体では反復を判定しない。
null moveの外では、対局履歴と探索経路の全体を照合する。

## 検討した代案

null move直後の局面キーを探索経路へ積まない案は採らなかった。
部分木の中で双方がじっとを指してnull move直後の局面へ戻る手順は、null moveを含まない本物の反復であり、これを検出できなくなるからである。
null moveをまたいで対局履歴まで遡る従来の方式は、実在しない引き分けを返すので採らなかった。

## 結果

null moveが入れ子になる場合は内側の境界を使い、部分木から戻るときに外側の境界を元に戻す。
境界の保持と照合は、`crates/minase/src/search/alphabeta/searcher.rs`の`is_repetition`と、`negamax.rs`の`search_null_move`にある。
非劣性の判定は、10 Elo未満の小さな低下までは否定しない。
この決定の根拠は棋力の向上ではなく探索値の正しさなので、評価関数が変わっても再提案の理由にはならない。
反復の値が置換表を通じて別の経路へ漏れる問題は本決定の範囲外であり、[探索部の反復負け回避](../plans/search-repetition.md)が扱う。

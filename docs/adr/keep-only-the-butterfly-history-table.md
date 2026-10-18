# 静かな手のhistoryはbutterfly表だけで引く

## ステータス

承認済み。決定日は2026年9月7日。

## コンテキスト

historyは、βカットを起こした静かな手の統計であり、静かな手の順序付けとLMRの減深量の増減に使う。
minaseのhistoryは、手番側、移動元、および移動先で引くbutterfly表である。
[棋力向上段階5](../plans/archived/strength-stage5.md)は、手番側、駒種、および到達升で引くpiece-to表を加え、両表の値の合計をhistory値とする案を測った。
この案は[STC](../measurements/strength-stage5/strength-stage5-piece-history-stc.md)で`H0`となった。
同じ段階では、移動元を駒種に置き換える案と、手番側、駒種、移動元、および移動先の4次元表も検討した。
前者は144升の盤で移動元の情報を失い、後者は約120万要素になって、1回の探索で各要素が受ける更新が少なすぎると判断した。
[不採用だった探索部の改良の再調整](../plans/archived/search-revival-spsa.md)は、piece-to表が数値係数を持たないことを理由に対象から外した。

## 決定

静かな手のhistoryは、手番側、移動元、および移動先で引くbutterfly表だけで構成する。
駒種と到達升で引く表は加えない。

## 検討した代案

butterfly表にpiece-to表を加える案は、STCで`H0`だったので採らなかった。
移動元を駒種に置き換える案は、移動元の情報を失うので採らなかった。
駒種と移動元と移動先の4次元表は、更新が疎で順序付けの信号にならないので採らなかった。

## 結果

表は`crates/minase/src/search/alphabeta/history.rs`の`HistoryTable`であり、順序付けとLMRは同じ値を引く。
直前の手や2手前の手と組み合わせるcounter move historyとcontinuation historyは、本決定の対象外である。
段階5の測定の後に、history表は探索をまたいで持ち越すように変わり、減点と更新式も変わった。
4次元表が疎だという判断は表を根探索ごとに初期化していた時期のものなので、再提案するときは現行の表の寿命で測り直す必要がある。

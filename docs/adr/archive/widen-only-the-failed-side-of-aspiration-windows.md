# aspiration windowsは外れた側だけを広げ、fail-high時に浅く読み直さない

## ステータス

置き換え済み。決定日は2026年9月12日。[aspiration windowsは深さ2以上で直前の最善手の平均を中心に置き、初期の半幅を根の値の大きさとともに広げ、窓外れでは外れた側の端を返った値から移して反対側の端も寄せる](../center-aspiration-windows-on-the-best-move-average-and-move-both-edges-on-failure.md)および[aspiration windowsはfail highの回数と反復をまたぐ持ち越しで読み直しの窓の深さを減らし、完了した反復は反復の深さの結果として扱う](../reduce-aspiration-re-search-depth-after-fail-highs-and-keep-the-iteration-depth.md)によって置き換えられた。

## コンテキスト

aspiration windowsは、反復深化の各反復で、前回の評価値を中心にした狭い窓で根を読み始め、窓を外れたら広げて同じ深さを読み直す手法である。
[探索部](../../plans/archived/search.md)の時点では、深さ5以上の反復が採否測定の条件で常用されなかったので見送った。
[棋力向上段階6](../../plans/archived/strength-stage6.md)では、段階5の保存記録でSTCの完了深さの最頻値が6、LTCが8になっていたので、深さ5以上を対象に実装した。
STCは223有効ペアでLLR +2.979、LTCは180有効ペアでLLR +2.961の`H1`であり、異常は0件だった（[STC](../../measurements/strength-stage6/strength-stage6-aspiration-stc.md)、[LTC](../../measurements/strength-stage6/strength-stage6-aspiration-ltc.md)）。
USIには窓外れを伝える`lowerbound`と`upperbound`のキーワードがある。
しかし対局ハーネスは最後の`info`行の深さと経過時間を手の記録として保存するので、窓外れの行を出すと、読み直しの途中で探索時間の上限であるhard limitに達した手の記録が未完了の反復の値になる。
2026年9月28日に完了した[棋力向上段階12](../../plans/archived/strength-stage12.md)では、連続したfail-highの回数だけ読み直しを浅くする変更を測った。
この変更はStockfishとEtherealで効果が記録されていた（[未導入の棋力向上手法の調査](../../research/unplanned-strength-techniques-survey.md#優先候補は既存機構の小改良に集まる)）。
しかしminaseでは、STCが1,360有効ペア、得点率49.1%の`H0`だった（[測定記録](../../measurements/strength-stage12/strength-stage12-aspiration-stc.md)）。

## 決定

直前に完了した反復の評価値が詰み帯の外にある深さ5以上の反復で窓を使い、補助ワーカーはそのワーカー自身の直前の完了値を中心にする。
窓を外れた側の端だけを広げて同じ深さを元の深さのまま読み直し、窓の内側で完了した結果だけを反復の完了として採用する。
窓外れの結果はプロトコルへ出力しない。

## 検討した代案

外れなかった側の端を中点へ寄せる方式は、読み直しの回数を減らす一方で窓の意味を複雑にするので採らなかった。
連続したfail-highの回数だけ読み直しを浅くする方式は、上記のとおりSTCで`H0`だったので採らなかった。
窓外れを`lowerbound`または`upperbound`の`info`行で出力する方式は、上記のとおり保存記録に残る手の深さと経過時間を未完了の反復の値にするので採らなかった。
補助ワーカーに主ワーカーの評価値を使わせる方式は、`Threads`の既定値が1で採否測定も1スレッドで行うので、実装を共有する利点を優先して採らなかった。

## 結果

窓の初期幅と拡大率は`crates/minase/src/search/alphabeta/params.rs`の係数であり、SPSAで調整される。
窓の作り方と読み直しの詳細は、`root.rs`の`AspirationWindow`と`search_iteration`のdocコメントにある。
fail-high時の減深を再び提案する場合は、本ADRの測定より新しい構成で、別の主張として段階ゲートにかける必要がある。

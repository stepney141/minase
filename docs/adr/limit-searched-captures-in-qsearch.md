# 静止探索は実際に読んだ捕獲手の数で打ち切る

## ステータス

承認済み。決定日は2026年10月4日。

## コンテキスト

minaseの探索では、静止探索のノードが全ノードの大半を占める。
[棋力向上段階12](../plans/strength-stage12.md)は、delta pruningとSEEの枝刈りを通過して実際に読んだ捕獲手が`N`手に達した後の捕獲手を、例外を除いて読まない手数制限を測った。
`N`は、失う良い結果の割合が10%以下の候補のうち最小のものとして[診断](../measurements/strength-stage12-activation-diag.md)から選び、`N=1`で失う割合は4.83%だった。
`N=1`の実装は深さ6の総ノード数を21.03%、探索時間を15.9%減らした（[bench](../measurements/strength-stage12-qsearch-limit-bench.md)）。
しかしSTCは147有効ペアで得点率27.0%、ロジスティック換算で約−173 Eloの`H0`だった（[測定記録](../measurements/strength-stage12-qsearch-limit-stc.md)）。
静止探索は深さの上限なく再帰するので、1ノードで失う良い結果の割合が小さくても損失が積み重なる（[教訓](../lessons/recall-loss-does-not-bound-recursive-pruning.md)）。
[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)は、上限を範囲1〜7の整数の係数としてSPSAで調整し、上限4で採用した（[調整の記録](../measurements/search-revival-t.md)、[採用のADR](adopt-revived-search-items-as-a-bundle.md)）。

## 決定

静止探索では、置換表の手を含め、delta pruningとSEEの枝刈りを通過して実際に読んだ捕獲手を数え、その数が上限に達した後の捕獲手を読まない。
相手の最後の王駒を取る手、直前の相手の捕獲手の到達升にある駒を取り返す手、および獅子が1手で2枚を取る手は、上限を超えても読み、読んだ手の数に含める。
上限の値は、失う良い結果の割合ではなく自己対局の調整で決める。

## 検討した代案

失う良い結果の割合で選んだ`N=1`は、上記のとおり大敗したので採らなかった。
Stockfishのように生成した全捕獲手を数えて`N=2`とする方式は、未測定の候補として残した。

## 結果

上限は`crates/minase/src/search/alphabeta/params.rs`の`QsearchMoveLimit`である。
取り返しは、直前の手の捕獲升ではなく到達升と比べて判定する。
居喰いでは取った升と取った駒の最終位置が異なるからであり、詳細は`searcher.rs`の`Searcher::previous_capture`のdocコメントにある。
静止探索の枝刈りの閾値を局所の損失の割合で選ぶ提案は、上記の教訓により採否の根拠にならない。
[bench](../measurements/strength-stage12-qsearch-limit-bench.md)では、取り返しと2枚取りの例外は実際に働いたが、最後の王駒を取る手の例外はこの局面集では働かなかった。

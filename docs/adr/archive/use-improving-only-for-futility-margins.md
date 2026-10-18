# improvingはfutilityの余裕値を縮めることにだけ使う

## ステータス

置き換え済み。決定日は2026年10月4日。[improvingは補正後の静的評価で判定し、futilityとreverse futilityの余裕値、late move pruningの上限、およびLMRの減深量に使う](judge-improving-on-the-corrected-static-eval-and-use-it-in-pruning-and-lmr.md)によって置き換えられた。

## コンテキスト

improvingは、現局面の静的評価が同じ手番の2手前より高いかを表すフラグであり、チェスのエンジンは良化していないノードで枝刈りと減深を強める。
[棋力向上段階8](../../plans/archived/strength-stage8.md)は、良化していないノードでfutility pruningの余裕値を縮める変更と、後方の静かな手のLMRの減深量を1増やす変更を1項目として設計した。
LMRの追加減深は、[再診断](../../measurements/strength-stage8/strength-stage8-improving-bench.md)で失う割合が15.0%から30.0%へ15.0ポイント増え、基準の10ポイントを超えたので外した。
余裕値の変更だけの版は、STCが694有効ペアでLLR −2.9907の`H0`だった（[測定記録](../../measurements/strength-stage8/strength-stage8-improving-stc.md)）。
[不採用だった探索部の改良の再調整](../../plans/archived/search-revival-spsa.md)は、この項目を含む8項目を戻し、深さごとの縮小率を既存の係数とまとめてSPSAで調整した。
調整した版は、現行のmasterに対してSTCとLTCで`H1`となり、8項目を除いた版に対してもSTCで`H1`となったので、improvingを含めて採用した（[採用のADR](../adopt-revived-search-items-as-a-bundle.md)）。

## 決定

良化していないノードでは、浅いノードのfutility pruningの余裕値に深さ別の縮小率を掛け、良化の判定には補正前の静的評価を使う。
2手前がない場合と、比較の区間にnull moveが入る場合は良化しているとみなし、余裕値を縮めない。
LMRの減深量はimprovingによって変えない。

## 検討した代案

良化していないノードでLMRの減深量を1増やす案は、上記の診断で失う割合の増分が基準を超えたので採らなかった。
補正後の静的評価で比べる案は、correction historyの学習につれて比較の基準が動くので採らなかった。

## 結果

縮小率は`crates/minase/src/search/alphabeta/params.rs`の`NonImprovingFutility1`から`NonImprovingFutility3`であり、判定の詳細は`negamax.rs`の`improving`のdocコメントにある。
現行のnull moveは手を指さずに`ply`を進めるので、比較の区間に仮想の着手が入る場合を除外する必要がある。

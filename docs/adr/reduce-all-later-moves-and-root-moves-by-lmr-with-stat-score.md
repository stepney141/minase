# LMRは先頭の手を除くすべての手と根に適用し、減深量に対数表、3段階のhistory補正、Stockfishの項、およびstatScoreの連続量を1/100手単位で加える

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

LMR（late move reductions）は、順序付けの後方にある手を浅く読み、αを上回ったときだけ本来の深さで読み直す手法である。
[旧ADR](archive/reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)は、2026年10月18日に、LMRを置換表の手とkiller手を除く非捕獲手に適用して根では行わず、減深量を対数表と3段階のhistory補正にStockfishの項を加えて1/100手単位で求めると決めた。
旧ADRは、Stockfishの項のうちstatScore（手の成績を表す値）の項と、LMRを捕獲手、killer手、および根へ広げることを、[historyの形の設計書](../plans/history-forms.md)に委ねていた。
旧ADRより前の[ADR](archive/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)は、history値に比例して減深量を連続的に増減する案を、比例係数と切り詰めの2つの自由度を絞る診断が要るとして棄却していた。

2026年10月17日のエンジンの開発者の決定により、形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れ、minaseにあってStockfishにない仕組みのうち中将棋の診断や測定で決めたものは残す。
historyの形の設計書は、この決定に従ってLMRのhistoryの使い方をStockfishの形へ寄せた。

## 決定

LMRは、残り深さ2以上のノードで、先頭の手を除くすべての手に適用する（Stockfish `src/search.cpp:1395`）。
対象とする手には、捕獲手、killer手、成る手、および先頭以外に来た置換表の手を含み、根のノードでも適用する。
減深量は、旧ADRの式（対数表、3段階のhistory補正、Stockfishの項、および上限`LmrMaxReduction`）から、statScore`s`に比例する連続量`trunc(s × LmrHistoryScale × 100/(4096 × 1024))`を、allNode（すべての手がα以下に終わると見込む零窓のノード）で減深量を拡大する処理の前に引いて求める（同1371-1381行）。
statScoreは、静かな手ではbutterfly historyと1手前と2手前のcontinuation historyの重みつきの和、捕獲手では取る駒の価値の換算値と捕獲履歴の和であり、LMRを適用するかどうかによらず実際に読むすべての手で求めて記録する。
子のノードがhistoryを更新するときに、このノードへ至った手のstatScoreを使うからである。

3段階の補正では、捕獲手でないすべての手（killer手と静かな置換表の手を含む）のbutterfly historyを閾値と比べ、捕獲手には補正を掛けない。
statScoreは3段階の補正の判定に使わない。
根では、PVノードとttPvを真、cutNodeを偽とし、Stockfishと同じく並べ替え後の先頭の手を置換表の手として扱い（同839行）、置換表の値と深さの項には実際の照合の結果を使う。
減深した探索の値がαを上回った後の再探索、負の減深量による延長、およびPVノードの1手の加算は、旧ADRの規則のままとする。
連続量の比例係数`LmrHistoryScale`はSPSAで調整し、statScoreの重みはStockfishの値で固定する。

## 検討した代案

3段階の補正を外してStockfishの連続量だけにする案は、補正が[棋力向上段階5](../plans/archived/strength-stage5.md)の診断と測定で決めた仕組みなので、エンジンの開発者の決定により採らなかった。
閾値を調整の対象に残しているので、SPSAが閾値を上げて3段階の補正を弱めることもできる。
連続量を、比例係数と切り詰めの診断の後に入れる案は採らず、比例係数をStockfishの値から始めてSPSAで調整した。連続量の上下限の切り詰めは、Stockfishと同じく設けない。
LMRを静かな手に限り、根では行わない旧ADRの形は、Stockfishにある項目は入れるという決定に反するので採らなかった。
根の途中結果の採用（`RootResults`）には再探索の後の値だけが記録されるので、根へ広げても途中結果の契約は変わらない。

根のLMRでは、減深した探索の値がαを上回った手を、浅くする調整の後に残り深さ0の全窓で再探索することがあり、その手の主変化は1手で終わる。
この場合に予想手を置換表などから補う案は、[予想手のADR](take-ponder-move-from-the-pv.md)の決定を変えるので採らなかった。

## 結果

式の詳細は`crates/minase/src/search/alphabeta/pruning.rs`の`LmrCoefficients::lmr_reduction`、`applies_lmr`、`lmr_history_value`、および`lmr_depth`と、`root.rs`の`search_root`のdocコメントにあり、statScoreの式は`ordering.rs`の`quiet_stat_score`と`capture_stat_score`にある。係数の現行値は`params.rs`にある。
historyの更新でstatScoreを使う箇所は[historyの更新のADR](order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md)が定める。
補正量の項は[correction historyの表の設計書](../plans/correction-history-tables.md)が扱い、本決定には含めない（`src/search.cpp:1353`）。
変更の効果は、[探索ノード数と探索速度の記録](../measurements/history-forms/history-forms-bench.md)、[SPSAの記録](../measurements/history-forms/history-forms.md)、および[STCの記録](../measurements/history-forms/history-forms-stc.md)にある。

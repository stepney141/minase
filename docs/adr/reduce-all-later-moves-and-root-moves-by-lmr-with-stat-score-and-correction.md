# LMRは先頭の手を除くすべての手と根に適用し、減深量に対数表、3段階のhistory補正、Stockfishの項、statScoreの連続量、および補正量の大きさの項を1/100手単位で加える

## ステータス

承認済み。決定日は2026年10月20日。

## コンテキスト

LMR（late move reductions）は、順序付けの後方にある手を浅く読み、αを上回ったときだけ本来の深さで読み直す手法である。
[旧ADR](archive/reduce-all-later-moves-and-root-moves-by-lmr-with-stat-score.md)は、2026年10月19日に、LMRを先頭の手を除くすべての手と根に適用し、減深量を対数表、3段階のhistory補正、Stockfishの項、およびstatScoreの連続量から1/100手単位で求めると決めた。
旧ADRは、Stockfishの項のうち補正量の大きさの項（`src/search.cpp:1353`）を、[correction historyの表の設計書](../plans/archived/correction-history-tables.md)に委ねていた。
旧ADRの前の[ADR](archive/reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)は、2026年10月18日に、LMRを置換表の手とkiller手を除く非捕獲手に適用して根では行わず、減深量を対数表と3段階のhistory補正にStockfishの項を加えて1/100手単位で求めると決めた。
このADRは、Stockfishの項のうちstatScore（手の成績を表す値）の項と、LMRを捕獲手、killer手、および根へ広げることを、[historyの形の設計書](../plans/archived/history-forms.md)に委ねていた。
さらに前の[ADR](archive/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)は、history値に比例して減深量を連続的に増減する案を、比例係数と切り詰めの2つの自由度を絞る診断が要るとして棄却していた。

2026年10月17日のエンジンの開発者の決定により、形や値がStockfishとYaneuraOuで違えばStockfishに合わせ、Stockfishにだけある項目は入れ、minaseにあってStockfishにない仕組みのうち中将棋の診断や測定で決めたものは残す。
historyの形の設計書は、この決定に従ってLMRのhistoryの使い方をStockfishの形へ寄せた。

## 決定

LMRは、残り深さ2以上のノードで、先頭の手を除くすべての手に適用する（Stockfish `src/search.cpp:1395`）。
対象とする手には、捕獲手、killer手、成る手、および先頭以外に来た置換表の手を含み、根のノードでも適用する。
減深量は、2026年10月18日のADRの式（対数表、3段階のhistory補正、Stockfishの項、および上限`LmrMaxReduction`）から、statScore`s`に比例する連続量`trunc(s × LmrHistoryScale × 100/(4096 × 1024))`を、allNode（すべての手がα以下に終わると見込む零窓のノード）で減深量を拡大する処理の前に引いて求める（同1371-1381行）。
statScoreは、静かな手ではbutterfly historyと1手前と2手前のcontinuation historyの重みつきの和、捕獲手では取る駒の価値の換算値と捕獲履歴の和であり、LMRを適用するかどうかによらず実際に読むすべての手で求めて記録する。
子のノードがhistoryを更新するときに、このノードへ至った手のstatScoreを使うからである。

同じくallNodeによる拡大の前に、補正量の大きさの項`trunc(LmrCorrection × |S| / 102400)`も引く（同1353行）。
`S`は、ノードの入口で補正後の静的評価を求めるときに読んだ、補正表の切り捨てる前の重み付きの和であり（[補正表のADR](sum-material-placement-and-continuation-correction-histories-in-stockfish-form.md)）、この項は歩兵価値1枚分の補正量あたり`LmrCorrection`の1/100手を減らす。
補正量の絶対値が大きいノードでは静的評価が当てにならないとみなし、浅く読む量を減らすためである。
根でも、根で読んだ`S`による同じ項を引き、枝刈り用のlmrDepthの式には入れない。

3段階の補正では、捕獲手でないすべての手（killer手と静かな置換表の手を含む）のbutterfly historyを閾値と比べ、捕獲手には補正を掛けない。
statScoreは3段階の補正の判定に使わない。
根では、PVノードとttPvを真、cutNodeを偽とし、Stockfishと同じく並べ替え後の先頭の手を置換表の手として扱い（同839行）、置換表の値と深さの項には実際の照合の結果を使う。
減深した探索の値がαを上回った後の再探索、負の減深量による延長、およびPVノードの1手の加算は、2026年10月18日のADRの規則のままとする。
連続量の比例係数`LmrHistoryScale`と補正量の項の係数`LmrCorrection`はSPSAで調整し、statScoreの重みはStockfishの値で固定する。

## 検討した代案

3段階の補正を外してStockfishの連続量だけにする案は、補正が[棋力向上段階5](../plans/archived/strength-stage5.md)の診断と測定で決めた仕組みなので、エンジンの開発者の決定により採らなかった。
閾値を調整の対象に残しているので、SPSAが閾値を上げて3段階の補正を弱めることもできる。
連続量を、比例係数と切り詰めの診断の後に入れる案は採らず、比例係数をStockfishの値から始めてSPSAで調整した。連続量の上下限の切り詰めは、Stockfishと同じく設けない。
LMRを静かな手に限り、根では行わない2026年10月18日のADRの形は、Stockfishにある項目は入れるという決定に反するので採らなかった。
根の途中結果の採用（`RootResults`）には再探索の後の値だけが記録されるので、根へ広げても途中結果の契約は変わらない。

根のLMRでは、減深した探索の値がαを上回った手を、浅くする調整の後に残り深さ0の全窓で再探索することがあり、その手の主変化は1手で終わる。
この場合に予想手を置換表などから補う案は、[予想手のADR](take-ponder-move-from-the-pv.md)の決定を変えるので採らなかった。

補正量の項を、切り捨てた後の補正量から求める案は、Stockfishが割る前の和を使うので採らなかった（[補正表のADR](sum-material-placement-and-continuation-correction-histories-in-stockfish-form.md)）。

## 結果

式の詳細は`crates/minase/src/search/alphabeta/pruning.rs`の`LmrCoefficients::lmr_reduction`、`applies_lmr`、`lmr_history_value`、および`lmr_depth`と、`root.rs`の`search_root`のdocコメントにあり、statScoreの式は`ordering.rs`の`quiet_stat_score`と`capture_stat_score`にある。係数の現行値は`params.rs`にある。
historyの更新でstatScoreを使う箇所は[historyの更新のADR](order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md)が定める。
statScoreの連続量までの変更の効果は、[historyの形の探索ノード数と探索速度の記録](../measurements/history-forms/history-forms-bench.md)、[SPSAの記録](../measurements/history-forms/history-forms.md)、および[STCの記録](../measurements/history-forms/history-forms-stc.md)にある。
補正量の項を加えたコミットは、benchの深さ3、5、7の探索ノード数の合計を8.7%増やした（[benchの記録](../measurements/correction-history-tables/correction-history-tables-bench.md)）。
補正量の項の係数`LmrCorrection`は、Stockfishの換算値101から始め、30,000ペアのSPSAで68になった（[SPSAの記録](../measurements/correction-history-tables/correction-history-tables.md)）。

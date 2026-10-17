# 静かな手の順序を減点つきの重力式と線形経路の差分の同点破りで決める

## ステータス

承認済み。決定日は2026年10月10日。

## コンテキスト

通常探索の静かな手は、butterfly historyの値の降順に並ぶ。
[棋力向上段階5](../plans/archived/strength-stage5.md)のmalusは、β打ち切りを起こした静かな手に加点し、同じノードで先に探索した静かな手を同じ大きさで減点したが、STCが上限3,000ペアでLLR −1.578の判定保留となり不採用だった（[測定記録](../measurements/strength-stage5/strength-stage5-malus-stc.md)）。
当時のLMRはhistory値を参照していなかった。
[手の順序付けの改善](../plans/archived/move-ordering.md)は、malusを加点と減点の別の尺度で再導入した。
[診断](../measurements/move-ordering/move-ordering-root-diag.md)は、本番の順序を変えずに減点つきの更新だけを並行して行う比較用の表（影の表）を使った。
この影の表で静かな手の順序が変わるノードは深さ6で92.3%あり、負のhistory値でLMRの減深が1増える経路に入る手は3.9〜20.0%だった。
同じ設計書は、history値が同じ手の同点を、駒の移動による評価の線形部分の差で破る候補を立てた。
[診断](../measurements/move-ordering/move-ordering-diag.md)では、この同点破りで先頭の静かな手が変わるノードが深さ6で12.41%あり、β打ち切りを起こすかαを上げた手の順位は、前進538件が後退393件を上回った。
両候補を含む[手の順序付けの改善の7候補](bound-capture-history-and-use-it-in-qsearch.md)の構成は、STCとLTCの`H1`で一括して採用した（[STC](../measurements/move-ordering/move-ordering-all-stc.md)、[LTC](../measurements/move-ordering/move-ordering-all-ltc.md)）。
同点破り、malus、および静止探索の群内の捕獲履歴の3候補を外した構成は、全7候補の構成との直接対局で得点率48.2%の`H0`であり、全7候補の構成が5 Elo以上強いと判定された（[測定記録](../measurements/move-ordering/move-ordering-no358-vs-all-stc.md)）。

## 決定

β打ち切りを起こした静かな手に加点し、同じノードで先に探索した静かな手を減点する。
値は上限に近づくほど更新量が小さくなる重力式で更新し、加点と減点の尺度は別の係数にする。
history値が同じ静かな手は、評価関数の線形経路で引いた移動先と移動元の差の降順、なお同じなら生成順に並べ、非線形経路の寄与は含めない。

## 検討した代案

同点を生成順のままにする案と、malusを使わない案は、上記の直接対局で3候補を外した構成が負けたので採らなかった。
同点破りにFMまたは非線形経路の寄与を含める案は、設計の時点で線形の差分だけに限ったので、診断も採否測定も行っていない。

## 結果

同点破りの差分は、設計の時点ではPSTとFMからなる評価関数のPSTで引いていた。
2026年10月12日に既定の評価関数が[NNUE](../plans/archived/nnue-replacement.md)へ置き換わった後は、NNUEが内包する線形経路で引く。
成る手を成った後の駒で引くことを含む差分の詳細は、`crates/minase/src/eval/nnue/mod.rs`の`quiet_move_pst_delta`のdocコメントにある。
NNUEの線形経路は旧PSTを初期値として学習し直した値なので、同点破りの効果を現行の評価関数で測り直してはいない。
加点と減点の尺度は既定値のままであり、次のSPSAの調整の対象である。
直接対局は3候補をまとめて外したので、各候補単独の寄与は分からない。

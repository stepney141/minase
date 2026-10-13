# 静的評価に基づく枝刈りは王駒に利きが届くノードでは行わない

## ステータス

承認済み。決定日は2026年10月4日。

## コンテキスト

通常探索は、静的評価を根拠に枝を読まずに済ませる前向き枝刈りを4つ持つ。
reverse futility pruning、futility pruning、late move pruning、およびrazoringである。
中将棋では王駒を相手の利きへ動かす着手も、王手を放置する着手も合法である（RULES.md第8条）。
王駒が取られ得る局面の静的評価は、最後の王駒の捕獲を詰みの値とする探索値から大きく外れる。
[棋力向上段階4](../plans/strength-stage4.md)は、これら4つの枝刈りに共通する適用条件と、reverse futility pruningの返り値を定めた。
王駒の捕獲を禁じる規則はないので、王駒の升では、相手駒の疑似利きと実際に捕獲できる駒の集合が一致する。
[段階4の診断bench](../measurements/strength-stage4-pruning-bench.md)では、非PVかつ詰み帯でないノードのうち、いずれかの王駒に利きが届くノードは残り深さ1、2、3で1.56%、1.87%、2.99%だった。
最後の王駒だけに限ると1.52%、1.80%、2.99%であり、差は小さかった。
段階4ではfutility pruningだけが[STC](../measurements/strength-stage4-futility-stc.md)と[LTC](../measurements/strength-stage4-futility-ltc.md)で`H1`となり、reverse futility pruningは[STC](../measurements/strength-stage4-rfp-stc.md)で`H0`だった。
[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)は、reverse futility pruningなどを同じ適用条件のまま戻して係数を調整し直し、[LTC](../measurements/search-revival-tstar-vs-m-ltc.md)で`H1`となって採用した。

## 決定

静的評価に基づく4つの枝刈りは、残り深さ3以下、探索窓の幅が1の非PVノードで、αとβがいずれも詰み帯の外にあり、手番側のいずれの王駒にも相手の疑似利きが届かないノードに限って行う。
王駒を2枚持つ側も、どちらか1枚に利きが届けば枝刈りを控える。
null move pruningには、王駒への利きの判定を加えない。
reverse futility pruningは、条件を満たしたノードで静的評価をそのまま返し、その値を置換表に保存しない。

## 検討した代案

判定を最後の王駒に限る案は、除外率がほとんど変わらないので採らなかった。
reverse futility pruningの返り値をβへ丸める案は、境界としては常に正しいが、上位ノードが得る情報を減らすので採らなかった。
診断が保証するのは境界判定の一致だけなので、返す値はnull move pruningやLMRの返り値と同じくfail-softの近似として扱う。

## 結果

条件の判定は`crates/minase/src/search/alphabeta/negamax.rs`にあり、王駒への利きは他の条件が成立したときだけ`royal_under_attack`で調べてノード内で再利用する。
SEEによる捕獲手の枝刈りも、同じ対象ノードと王駒への利きの判定を共有する。
王駒への利きを調べる費用は係数で消せないので、枝刈りの効果が小さい構成では費用だけが残る。

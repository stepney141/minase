# 浅いノードの前向き枝刈りは王駒に利きが届くノードでは行わない

## ステータス

承認済み。決定日は2026年9月6日。

## コンテキスト

通常探索は、浅いノードで枝を読まずに済ませる前向き枝刈りを持つ。
reverse futility pruning、futility pruning、およびrazoringは静的評価を根拠にし、late move pruningは順序付けの後方にある手の数を根拠にする。
中将棋では王駒を相手の利きへ動かす着手も、王手を放置する着手も合法である（RULES.md第8条）。
王駒が取られ得る局面の静的評価は、最後の王駒の捕獲を詰みの値とする探索値から大きく外れる。
[棋力向上段階4](../plans/strength-stage4.md)は、reverse futility pruning、futility pruning、late move pruning、およびrazoringの4項目に共通する適用条件と、reverse futility pruningの返り値を定めた。
王駒の捕獲を禁じる規則はないので、王駒の升では、相手駒の疑似利きと実際に捕獲できる駒の集合が一致する。
[段階4の診断bench](../measurements/strength-stage4-pruning-bench.md)では、非PVかつ詰み帯でないノードのうち、いずれかの王駒に利きが届くノードは残り深さ1、2、3で1.56%、1.87%、2.99%だった。
最後の王駒だけに限ると1.52%、1.80%、2.99%であり、差は小さかった。
段階4ではfutility pruningだけが[STC](../measurements/strength-stage4-futility-stc.md)と[LTC](../measurements/strength-stage4-futility-ltc.md)で`H1`となり、reverse futility pruningは[STC](../measurements/strength-stage4-rfp-stc.md)で`H0`だった。
2026年9月21日には、[棋力向上段階8](../plans/strength-stage8.md)がSEEによる捕獲手の枝刈りを同じ対象ノードに加えた（[ADR](prune-losing-captures-by-see-at-shallow-nodes.md)）。
2026年10月4日には、[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)が、reverse futility pruning、razoring、および定義を改めたlate move pruningを同じ適用条件のまま戻して係数を調整し直し、[LTC](../measurements/search-revival-tstar-vs-m-ltc.md)で`H1`となって採用した（[採用のADR](adopt-revived-search-items-as-a-bundle.md)）。
本ADRは、この拡張を含む現行の適用条件を記録する。

## 決定

浅いノードの前向き枝刈り（reverse futility pruning、futility pruning、late move pruning、razoring、およびSEEによる捕獲手の枝刈り）は、残り深さ3以下の零窓のノードで、αとβがいずれも詰み帯の外にあり、手番側のいずれの王駒にも相手の疑似利きが届かない場合に限って行う。
手を展開しないfutility pruning、late move pruning、およびSEEによる捕獲手の枝刈りは、さらにそのノードで探索済みの最善値が負の詰み帯の外にあることを要求し、razoringは残り深さ2以下に限る。
reverse futility pruningは条件を満たしたノードで静的評価をそのまま返して置換表に保存せず、null move pruningには王駒への利きの判定を加えない。

## 検討した代案

判定を最後の王駒に限る案は、除外率がほとんど変わらないので採らなかった。
reverse futility pruningの返り値をβへ丸める案は、境界としては常に正しいが、上位ノードが得る情報を減らすので採らなかった。

## 結果

診断で確かめたのは境界判定の一致だけなので、reverse futility pruningが返す値は、null move pruningやLMRの返り値と同じくfail-softの近似として扱う。
探索済みの最善値の条件により、各ノードで少なくとも1手は探索されるので、全手を枝刈りしたノードを合法手のない負けと取り違えない。
条件の判定は`crates/minase/src/search/alphabeta/negamax.rs`の`negamax`にあり、王駒への利きは他の条件が成立したときだけ`royal_under_attack`で調べてノード内で再利用する。
各枝刈りが対象にしない手の範囲などの詳細は、`negamax`と`royal_under_attack`のdocコメントにある。
王駒への利きを調べる費用は係数で消せないので、枝刈りの効果が小さい構成では費用だけが残る。

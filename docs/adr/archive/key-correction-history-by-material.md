# correction historyは駒種別の枚数を鍵にし、futilityの判定にだけ使う

## ステータス

置き換え済み。決定日は2026年9月21日。[correction historyは駒種別の枚数を鍵にし、各ノードで1回求めた補正後の静的評価を枝刈りと減深のすべての判定に使う](key-correction-history-by-material-and-apply-it-to-all-static-eval-checks.md)によって置き換えられた。

## コンテキスト

correction historyは、探索値と静的評価のずれを局面の特徴ごとに学習し、枝刈りの判断に使う静的評価を補正する手法であり、StockfishとYaneuraOuが実装している（[既存実装の調査](../../research/forward-pruning-prior-art.md)）。
[棋力向上段階8](../../plans/archived/strength-stage8.md)は、鍵と更新の条件の候補を、探索の進行に沿って表を更新しながら次の観測の予測誤差で比べた（[再診断](../../measurements/strength-stage8/strength-stage8-correction-bench.md)）。
観測点の78.3%は最善手が捕獲手のノードであり、全観測点で更新する変種Aの改善率は−4.84%から1.99%にとどまった。
最善手が捕獲手でなく、手番側の王駒に相手の利きが届かないノードでだけ更新する変種Bでは、駒種別の枚数の組を鍵にした改善率が15.41%、非ゼロ補正の参照率が66.9%だった。
捕獲手による駒得は評価の偏りではないので、StockfishとYaneuraOuも最善手が捕獲手のノードでは補正を更新しない（[既存実装の調査](../../research/forward-pruning-prior-art.md)）。
STCは398有効ペアでLLR +2.9903、LTCは679有効ペアでLLR +2.9846の`H1`であり、異常は0件だった（[STC](../../measurements/strength-stage8/strength-stage8-correction-stc.md)、[LTC](../../measurements/strength-stage8/strength-stage8-correction-ltc.md)）。
2026年9月28日に完了した[棋力向上段階12](../../plans/archived/strength-stage12.md)は、[診断](../../measurements/strength-stage12/strength-stage12-activation-diag.md)で鍵の追加と表の持ち越しを調べた。
歩兵と仲人の配置、および先手と後手の歩兵と仲人以外の駒の配置を鍵にした表を加えると、平均絶対誤差は0.20〜7.77%悪化した。
表を対局内で持ち越しても、futilityの判定で枝刈りの可否が変わる静かな手はSTC相当で0.69%、LTC相当で0.80%だった。

## 決定

補正表は双方の駒種別の枚数の組と手番側を鍵とし、探索ごとに初期化して反復深化の間だけ持ち越す。
探索値が詰み帯の外にあり、置換表へ保存する値の境界の種類から補正の向きが定まり、最善手が捕獲手でなく、手番側の王駒に相手の利きが届かないノードでだけ更新する。
補正値は、浅いノードのfutility pruningの判定で静的評価に加えるだけに使い、置換表の保存値と静止探索のstand-patは変えない。

## 検討した代案

全観測点で更新する案（変種A）は、上記のとおり予測を改善しなかったので採らなかった。
王駒の升の組を鍵にする案と、王駒の升の組と枚数の組を組み合わせて鍵にする案は、変種Bでの改善率が15.27%と14.98%で枚数の組を下回ったので採らなかった。
駒配置の鍵の表を加える案は、予測誤差を悪化させ、改善率の基準10%に届かなかったので見送った。
補正表を対局内で持ち越す案は、発動率が基準の5%に届かなかったので見送った。

## 結果

表の大きさ、補正値の上限、および更新の重みは`crates/minase/src/search/alphabeta/correction.rs`と`params.rs`にあり、上限と重みはSPSAで調整される。
鍵と更新の詳細は`correction.rs`の`CorrectionTable`と`material_key`のdocコメントにある。
駒配置の鍵の診断では、各表の補正を平均せずに足し合わせる形を評価していないので、その形は未測定の候補として残る。

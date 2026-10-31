# AlphaZero型探索の強化学習の自己対局は、Gumbel AlphaZeroの探索をQの尺度1.0で行い、指す手のGumbel乱数にだけ手数で減る倍率を掛ける

## ステータス

承認済み。決定日は2026年10月31日。
[Gumbel AlphaZeroをmctxの既定値で行うADR](archive/use-gumbel-alphazero-for-zero-self-play-search.md)を置き換える。

## コンテキスト

強化学習（RL）の自己対局は、1手32回の少ない探索で方策の目標を作るため、Gumbel AlphaZero（Danihelka et al., "Policy improvement by planning with Gumbel", ICLR 2022）の探索をDeepMindのmctxのコミット428bfb7に合わせて実装している。
初版は、completed Q（未訪問の手のQを混合価値で補ったもの）の変換σ(q̂)=(50＋最多訪問回数)×c_scale×q̂の尺度c_scaleをmctxの既定値0.1とし、最後に指す手を選ぶ得点にも候補の抽出と同じGumbel乱数をそのまま足していた。
2026年10月28日から31日のサイクル1〜37では、方策は改善したが、検証の対局で対局結果に対する価値の損失は、勝敗を常に五分と予測したときの値ln 2（約0.693）をわずかに下回るまでしか下がらなかった。
[学習効率と強さに効く技法の調査](../research/alphazero-rl-training-efficiency.md)は、mctxの既定値0.1がAtariに使った値であり、論文が囲碁とチェスに使った値は1.0で、Gumbel dlshogiも2025年12月24日に0.1から1.0へ改めたことを確かめた。
同じ調査は、KataGoが指す手の温度を対局の進行とともに下げることも挙げた。
利用者は2026年10月31日に、この2つを含む9つの技法を1つずつ比べずにまとめて採ると決めた（[学習技法の導入の設計書](../plans/archived/alphazero-restart-techniques.md)）。

## 決定

自己対局の探索は、引き続きGumbel-Top-kによる候補の抽出、Sequential Halving、およびcompleted Qによる方策の目標からなるGumbel AlphaZeroとし、式はmctxのコミット428bfb7の`gumbel_muzero_policy`と`qtransform_completed_by_mix_value`に合わせる。
候補数の上限は16、訪問回数の係数は50とし、Qの尺度は1.0とする。
尺度は探索の構築時の引数とし、根の逐次半減、最後に指す手、方策の目標、および探索木の内部の手の選択に同じ値を使う。
最後に指す手を選ぶ得点のGumbel乱数にだけ、倍率T(t)=0.15＋0.85×0.5^(t/12)を掛ける。
tは初期局面からの手数（乱択の序盤を含む）であり、候補の抽出と逐次半減の比較には倍率1.0の乱数をそのまま使う。
終値0.15と、19路盤の19手を盤面積で換算した半減期12手は、KataGoの`chosenMoveTemperature`と`chosenMoveTemperatureHalflife`の既定値による。
初期値1.0は論文がそのまま使った倍率である。
倍率の式は、Claudeが補った値である。

## 検討した代案

通常のPUCTとディリクレ雑音で1手800回探索する案は、置き換えたADRと同じく採らなかった。
中将棋の合法手は1局面あたり平均約81手あり、800回でも根の子を一巡できず、探索回数を減らす余地もない。

参照値をmctxから採らずに、論文の式から手計算した期待値だけで試験する案も採らなかった。
Sequential Halvingの整数への切り捨てや、未訪問の手に入れる混合価値の細部は論文の本文から一意に決まらず、照合の相手を実装そのものにしないと差を検出できない。

尺度を定数のまま1.0に書き換える案は採らなかった。
mctxとの一致を確かめる試験は0.1の参照値でも式の誤りを検出できるので、引数にして0.1と1.0の両方の参照値で式を固定する。

倍率を候補の抽出と逐次半減の比較にも掛ける案は採らなかった。
Gumbel-Top-kの抽出と逐次半減は、同じ乱数を使うことで方策の改善を保証する仕組みであり、乱数を弱めると探索そのものが変わる。
最後の選択だけに掛ければ、方策の目標softmax(logit＋σ)は乱数を含まないので変わらず、指す手の列と対局結果だけが変わる。

一定の手数の後に倍率を0にする案は採らなかった。
終盤でも乱数を少し残し、同じ手順の繰り返しを避けるKataGoの形に合わせた。

## 結果

mctxは勝敗の確定を扱わないが、minaseの探索木は確定を伝播するので、最後に指す手だけは確定した勝ちを優先し、確定した負けを候補から除く。
方策の目標はmctxの式のまま変えない。
Gumbelの探索は仮想損失を使わず、1つの探索木で評価を待つ葉は1つまでとするので、自己対局生成器は多数の局の探索木から葉を1つずつ集めて推論にまとめる（[葉をまとめるADR](batch-zero-self-play-leaves-across-many-games.md)）。
候補の集合と探索の結果も同じ乱数に左右されるので、残った候補の間で指す手の分布がsoftmax((logit＋σ)/T)に一致する保証はない。
乱択の序盤が終わる8〜16手目の倍率は約0.69〜0.49、50手目では約0.20になる。
mctxの参照値は`tools/train/zero/gumbel_reference.py`が尺度0.1と1.0の両方で書き出し、`crates/minase/src/search/mcts/testdata/gumbel-mctx.json`としてコミットする。
尺度1.0では完成したQが(50＋最多訪問回数)倍の大きさになるので、照合の許容誤差は尺度に比例させる。
playout cap randomizationは[探索回数の乱択のADR](randomize-zero-self-play-playout-caps-and-learn-the-policy-only-from-full-searches.md)が定める。
実装は`crates/minase/src/search/mcts/gumbel.rs`と`crates/minase/src/bin/minase/zero_selfplay/game.rs`にある。

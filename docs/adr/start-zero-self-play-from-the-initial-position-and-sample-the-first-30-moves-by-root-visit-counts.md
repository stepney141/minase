# AlphaZero型探索の強化学習の自己対局は、標準の初期局面から始め、最初の30手を根の訪問回数に比例して抽選し、以後はGumbel AlphaZeroの最後の選択で指す

## ステータス

承認済み。決定日は2026年11月15日。
[手数で減る倍率のADR](archive/use-gumbel-alphazero-with-a-unit-q-scale-and-a-decaying-final-move-temperature.md)を置き換える。

## コンテキスト

自己対局の探索は、1手32回の少ない探索で方策の目標を作るため、Gumbel AlphaZero（Danihelka et al., "Policy improvement by planning with Gumbel", ICLR 2022）の探索をDeepMindのmctxのコミット428bfb7に合わせて実装している。
置き換えたADRは、Qの尺度を1.0とし、最後に指す手を選ぶ得点のGumbel乱数にだけ、手数とともに0.15へ減る倍率を掛けると定めた。
倍率の終値と半減期は囲碁のエンジンKataGoの既定値に倣い、式はClaudeが補ったものである。
2回目の学習の準備では、標準の初期局面から8〜16手を一様に乱択してから自己対局を始めると決めていた（2026年11月13日の決定）。
2026年11月15日に学習の設定を先行実装のソースと論文で照合したところ、どちらの方式もチェスと将棋のエンジンに先例がなかった（[学習レシピを先行実装に揃える設計書](../plans/archived/zero-recipe-precedents.md)）。
序盤の手を探索の訪問回数に比例して抽選する方法は、AlphaZero、Gumbel AlphaZeroの論文のチェスの設定、Gumbel dlshogi、dlshogi、およびAobaZeroに共通する。
Gumbel dlshogiは、逐次半減の後の根の訪問回数に比例して最初の30手を抽選し（Gumbel dlshogi、コミット5538f7bの`gumbel_dlshogi/selfplay.py`の`sample_by_visit_counts`）、Danihelkaの博士論文（University College London）の付録C.5も、チェスで最初の30手を探索的な着手（explorative actions）とする。
エンジンの開発者は同じ日に、先例のないminase独自の方式を破棄すると決めた。

## 決定

自己対局は、全サイクルで標準の初期局面から始める。
初期局面からの手数pを先手と後手の手を通して0から数え、pが0〜29の30局面では、探索を終えた後に、根の各子の訪問回数に比例して指す手を抽選する（温度1）。
訪問回数には逐次半減で外れた手の訪問も含め、訪問のない手は選ばない。
pが30以降の局面では、Gumbel AlphaZeroの最後の選択で指す。
Gumbel乱数の倍率は、候補の抽出、逐次半減、および最後の選択のすべてで1とする。
探索は、Gumbel-Top-kによる候補の抽出、逐次半減（Sequential Halving）、およびcompleted Qによる方策の目標からなるGumbel AlphaZeroのままとし、候補数の上限16、訪問回数の係数50、およびQの尺度1.0は置き換えたADRと同じとする。
最後の選択は、置き換えたADRと同じく、確定した勝ちの手を優先し、確定した負けの手を除く。
記録する方策の目標は、抽選の有無によらず、未訪問の手を補ったQを正規化した値q̂とその変換σを用いたsoftmax(logit＋σ(q̂))とする。

## 検討した代案

初期局面から8〜16手を一様に乱択してから始める案は採らなかった。
チェスと将棋のエンジンに先例がないからである。

出村と金子の2手の一様な乱択（RandomStart2、Demura and Kaneko, ICGA Journal 46(2), 2024）は採らなかった。
単独の論文の方式であり、複数のエンジンに共通する、訪問回数による抽選と食い違うからである。

最後の選択のGumbel乱数に手数とともに減る倍率を掛ける方式を保つ案は採らなかった。
倍率の値はKataGoの既定値に倣ったもので、チェスと将棋のエンジンに先例がないからである。

訪問回数で抽選する手数を中将棋の対局の長さに合わせて30手から変える案は採らなかった。
30手はGumbel dlshogiと論文の値であり、中将棋の値が先例で決まらないときは最も近い先例を写す規則（学習レシピを先行実装に揃える設計書の「判定の基準」の節）に従うからである。

## 結果

pが0〜29で実際に指す手は、最後の選択の手と異なることがある。
その局面で記録する価値と投了の判定に使う値は、最後の選択の手の値とする（[記録する価値と投了のADR](calibrate-the-zero-self-play-resignation-threshold-as-aobazero-does-and-record-the-gumbel-choice-value.md)）。
組み替えた初期配置（[組み替えのADR](shuffle-the-first-three-ranks-within-each-rank-for-zero-self-play-start-positions.md)）は、生成器の`--start-position shuffled-setup`として残すが、2回目の学習では使わない。
初期局面から8〜16手を乱択する手順`minase::datagen::opening`は、αβ探索の評価関数（NNUE）の教師データの生成器が使うので残す。
mctxの参照値による照合（`tools/train/zero/gumbel_reference.py`と`crates/minase/src/search/mcts/testdata/gumbel-mctx.json`）は変えない。
実装は`crates/minase/src/search/mcts/gumbel.rs`と`crates/minase/src/bin/minase/zero_selfplay/game.rs`にある。

# 評価モデル、教師生成、学習方法の一次資料

中将棋エンジンminaseの評価関数を改善するため、先行する将棋エンジンとチェスエンジンの評価モデル、教師生成、学習方法を調査した。
本書は2026年9月26〜27日に確認した一次資料を、表現できる関係、学ぶ局面と教師値、重みを更新する方法の3つに分けて整理したものである。
各資料から確認できた事実と、minaseへの適用を検討するための推論を併記する。
minaseの不採用結果と次の比較は[評価改善方針](evaluation-improvement-strategy.md)、入力と容量の具体案は[評価アーキテクチャ候補](evaluation-architecture-candidates.md)を参照する。

局面 \(s\) を重み \(\theta\) で評価するモデルを \(F_\theta(s)\)、学習で最小化する損失を \(L(\theta)\) と書く。
玉1枚と駒2枚の関係を使うKPPや、効率よく差分更新するニューラルネットワークであるNNUEは、前者の構造を定める。
教師値への回帰や候補手の順位比較は、後者を定める。
同じモデルでも、局面の採り方、教師値、損失、学習中の探索を変えられるため、これらを別の実験条件として扱う。

## 評価モデルの表現と実行費用

### 駒関係表とNNUEの共有

将棋エンジンBonanzaは、駒価値に加えて、玉と駒の2駒関係、2玉と1駒、1玉と2駒の関係を評価し、Minimax Tree Optimizationという方法で係数を学習した。
KPPは玉1枚と駒2枚の配置を入力するため、係数の線形和でも3駒の条件を表現できる。
原論文は大きなモデルで正則化と評価尺度の制約を使い、合計52,000局の固定ノード対局などで棋力を測っている。
係数について線形な評価モデルでも、複数の駒の関係を入力すれば表現力を増やせることを示す先例である。[Hoki and Kaneko, 2014, §3〜4](https://www.jair.org/index.php/jair/article/download/10871/25935)

チェスエンジンStockfishのNNUE解説には、まれな特徴の学習を助ける**特徴分解**がある。
玉位置ごとの特徴へ、玉位置によらない共通の駒配置特徴を学習時だけ加え、学習後に共通重みを各特徴へ加算する。
推論時の入力数を増やさず共有できるが、公式解説は主に学習初期に役立つとし、分解の追加が常に改善するわけではないとも述べる。[NNUE公式解説のFeature factorization節](https://official-stockfish.github.io/docs/nnue-pytorch-wiki/docs/nnue.html#feature-factorization)

将棋エンジンNineDayFeverの2019年大会資料も、3駒関係の共通要素を分解して学習する方法と、手番を考慮した評価を記している。
この資料からは、駒関係をどう表現するかに加えて、共通要素の重みをどう共有して学習するかが設計対象になっていたことを確認できる。[作者の大会資料、PDFの104〜105ページ](https://www.apply.computer-shogi.org/wcsc29/appeal/appeal_round2_190503.pdf#page=104)

minaseで提案した局所2駒表の低ランク学習、王駒と遮蔽物の射線表、小型NNUEは、これらの原理を参考にした独自の構成案である。
学習後に通常の表へ展開すれば行列演算を推論へ持ち込まずに済むが、展開後の表容量と参照費用は残る。
これらの構成そのものについては、先行エンジンでの採用実績も、minaseでの棋力改善も未確認である。

### 利き特徴の実行費用と棋力

Stockfishは2025年11月12日に、利きで結ばれた駒の組を入力するFullThreatsを採用した。
導入コミットには、短時間63,424局、長時間27,876局、さらに長時間12,458局の逐次確率比検定を通過した記録がある。
ネットワークと量子化も変更しているため、利き特徴だけの単独効果を測った結果ではない。[採用コミット](https://github.com/official-stockfish/Stockfish/commit/8e5392d79a36aba5b997cf6fb590937e3e624e80)

開発者の添付報告では、固定ノードの棋力は約30 Elo改善し、速度はx86で5〜15%低下、ARMでは同程度だった。
初期の移植は失敗し、利きの差分追跡と利き特徴の重みの8ビット化が、固定時間での改善に必要だったと記されている。
これらは作者の報告であり、本調査で再測定した数値ではない。[導入経緯と速度の報告](https://github.com/user-attachments/files/23478634/Stockfish.threat.inputs.PR.summary.pdf)

Stockfishの公式FAQも、NNUE版は古い手作り評価の版より強い一方、同じハードウェアで速度が半分未満になる場合があると説明する。
したがって、1秒あたりの探索ノード数（NPS）だけで表現を採否判定せず、固定ノードと固定時間の棋力を分けて測る根拠になる。
固定ノード対局でも探索木は変わるため、静的評価の精度だけを分離する実験にはならない。[Stockfish公式FAQ](https://official-stockfish.github.io/docs/stockfish-wiki/Stockfish-FAQ.html#stockfish-is-slower-than-expected)

利き特徴の圧縮と対象の限定には、それぞれ先例がある。
将棋エンジンやねうら王の確認版`c1b80eaa09fe13d5f12b1599d1ae4d53c224de30`にあるHalfKPE9は、双方の利き数を0、1、2以上へ圧縮した9状態を使い、動かなかった駒も利きの前後差に応じて更新する。
利き表の維持を前提とするため、状態数が少ないことは抽出費用が小さい証拠ではない。[HalfKPE9の更新処理](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/eval/nnue/features/half_kpe9.cpp)

Stockfishの確認版`0a215d6c9e48856ef630013b8ab8312941a59057`のFullThreatsは、攻撃元と攻撃先の双方から王を除き、歩の対象もナイトとルークに限定するなど、駒の組を選別している。
王位置は座標の向きを決めるために使われる。[特徴の表](https://github.com/official-stockfish/Stockfish/blob/0a215d6c9e48856ef630013b8ab8312941a59057/src/nnue/features/full_threats.h)、[列挙処理](https://github.com/official-stockfish/Stockfish/blob/0a215d6c9e48856ef630013b8ab8312941a59057/src/nnue/features/full_threats.cpp)
中将棋への適用では、遮蔽物による射線の変化と、獅子の2段階の着手、居喰い、捕獲制限を含めて費用と意味を調べる必要がある。

## 教師局面と教師値の生成

### 局面の多様性と教師の意味

将棋エンジンelmoの作者が公開した学習手順は、初期局面集から教師を生成し、シャッフルして学習する工程を分けている。
NineDayFeverの大会資料も、学習用対局の開始局面を生成する実験や、自己対局結果を使う方法を記している。
いずれも、重み更新の式だけでなく、どの局面を学ばせるかを設計している先例である。[elmo作者の学習手順](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/README.md)、[NineDayFeverの大会資料](https://www.apply.computer-shogi.org/wcsc29/appeal/appeal_round2_190503.pdf#page=104)

教師の探索値と対局結果は異なる情報を与える。
前者は特定の評価と探索が判断した有利不利、後者はその対局方策で得た決着であり、どちらもゲーム理論的価値そのものとは限らない。
教師と結果の混合の先行調査は[教師と混合比の調査](chess-evaluation-targets.md)、将棋の具体的な生成工程は[GenSfenとtataraの調査](teacher-generation-prior-art.md)を参照する。

### 静止探索末端への置換

駒の取り合いが続く局面で探索を延長する**静止探索**は、戦術的な変化を読み進めてから局面を評価するために使われる。
やねうら王の公式解説は、静止探索で得た最善応手列（PV）の末端へ教師局面を置換し、探索時に評価する局面へ学習対象を近づける方法を説明する。
入力局面の変換自体はニューラルネットワークの構造を使わず、駒関係表や駒升表（PST）にも適用を検討できる。[公式の学習解説](https://github.com/yaneurao/YaneuraOu/wiki/評価関数の学習)

確認版`c1b80eaa09fe13d5f12b1599d1ae4d53c224de30`の`qsearch_psv`は元の教師値を保持し、奇数手進んだ場合に手番側視点の評価値と勝敗ラベルの符号を反転する。
1手以上進んだ場合は元局面の最善手を無効化する。
末端の抽出には全探索窓で静止探索し、置換表の読み取りを無効にしたうえで、その探索が構築したPVを使う。[局面とラベルの変換](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/engine/yaneuraou-engine/yaneuraou-search.cpp#L377-L435)、[末端抽出](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/engine/yaneuraou-engine/yaneuraou-search.cpp#L2178-L2221)

この変換では元の教師値を引き継ぐため、元局面と末端の深い探索値が等しいかどうかは未検証のまま残る。
静止探索と深い教師で最善手順が違えば、符号の修正だけでは末端自身の価値に対応しない場合がある。
元のラベルを引き継ぐ近似、末端を教師で再評価する方法、元局面の探索出力を学習して末端へ勾配を渡す方法を区別する必要がある。
公式の2026年5月13日版の手順も、深層学習モデルで再評価する前の局面加工を用途として挙げている。[固定版の学習手順](https://github.com/yaneurao/YaneuraOu/wiki/やねうら王の学習手順/1e3194ec5f778d12650ef82a6f3c3001ce30d503)

末端だけへの適合では、探索全体への改善を保証できない。
やねうら王の実装は静止探索の途中でも、現在の静的評価で探索を打ち切るstand pat判定に評価を使い、通常探索でも静的評価を枝刈りに使う。[静止探索の判定](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/engine/yaneuraou-engine/yaneuraou-search.cpp#L4691-L4837)、[通常探索の枝刈り](https://github.com/yaneurao/YaneuraOu/blob/c1b80eaa09fe13d5f12b1599d1ae4d53c224de30/source/engine/yaneuraou-engine/yaneuraou-search.cpp#L3090-L3236)
minaseの[探索中の局面を収集する教師生成実験](../plans/search-aware-evaluation.md)では静止探索中に静的評価を呼んだ局面を集めており、PV末端だけを集める方法とは学習対象の分布が異なる。

## 損失と学習中の探索

### elmoの混合損失と重み更新

elmoの公開コードでは、学習のたびに現在の評価で静止探索し、PV末端の静的評価を元局面の手番側へ換算する。
この予測を勝率尺度へ写した \(p\)、保存済みの深い教師値を同じ尺度へ写した \(q\)、対局結果 \(z\) に対し、更新方向を次の差から求める。

\[
(p-z)+0.5(p-q)
=1.5\left(p-\left(\frac23z+\frac13q\right)\right).
\]

原コードの`LAMBDA = 0.5`は、結果に対する誤差を重み1としたときの、教師値に対する誤差の重みである。
両者の重みの和を1に正規化すると、教師値の比率は1/3になる。
これは二値交差エントロピーを結果と教師へ混合した損失の勾配に対応する。
minaseの教師混合比は正規化後の比率を表すため、elmoのこの設定に対応する値は1/3となる。[作者の学習コード、確認版`f825ff0`の691〜724行](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L691-L724)

同じ実装は、係数ごとの累積二乗勾配の平方根で更新量を割るAdaGrad型の更新を使う。
したがって、elmoの方法には、静止探索末端での学習、教師と結果の混合、最適化手法という別々の選択が含まれる。
最適化手法をAdamなどへ変更する実験では、学習する局面と教師値を固定することで、重みの更新方法そのものの効果を比較できる。[係数更新の実装](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L586-L600)

NNUE以前のやねうら王にも、同様に静止探索のPV末端で駒関係表の特徴へ勾配を与える実装がある。
2017年の確認版では、学習中の評価で静止探索し、深い教師との差から勾配を計算して、末端で手番を考慮する駒関係表であるKPPTの更新処理を呼ぶ。
重み更新後も末端を採り直すため、固定評価で局面を1回だけ置換する前処理とは異なる。[2017年の学習処理](https://github.com/yaneurao/YaneuraOu/blob/f6d3ef082a2947ffac43b13248028d1e88c2eda3/source/learn/learner.cpp#L1341-L1410)、[KPPTの勾配処理](https://github.com/yaneurao/YaneuraOu/blob/f6d3ef082a2947ffac43b13248028d1e88c2eda3/source/learn/evaluate_kppt_learn.cpp#L28-L35)

### RootStrapとTDLeafとTreeStrap

RootStrap、TDLeaf、TreeStrapは、探索結果を用いて評価関数を学習する手法である。
各手法は、どの局面の予測をどの値へ近づけるかを定めている。
以下では対局の \(t\) 手目の局面を \(s_t\)、\(V_\theta^D(s)\) を深さ \(D\) の探索値、\(\ell_\theta^D(s)\) をそのPV末端とし、値の視点を統一する。

| 方法 | 更新する予測 | 教師となる値 |
| --- | --- | --- |
| RootStrap | 現在局面の静的評価 \(F_\theta(s_t)\) を更新する。 | 同じ局面の探索値 \(V_\theta^D(s_t)\) を使う。 |
| TDLeaf | 現在局面のPV末端の評価 \(F_\theta(\ell_\theta^D(s_t))\) を更新する。 | 実際に対局が進んだ後の局面の探索値を使い、TDLeaf(λ)では時間差を重み付きで累積する。 |
| TreeStrap | 探索木の各内部局面の静的評価を更新する。 | その局面を根とする部分木の探索値、または上下界を使う。 |

RootStrapの原論文は探索値を定数として扱い、静的評価との二乗誤差で重みを更新する。
TreeStrapのαβ版は、静的評価が既知の上下界の外側にあるときだけ、対応する境界へ近づける片側の損失を使う。[Veness et al., 2009, §2〜4](https://proceedings.neurips.cc/paper_files/paper/2009/file/389bc7bb1e1c2a5e7e147703232a88f6-Paper.pdf)
TDLeafの原論文は、現在のPVが局所的に変わらないものとして勾配を末端へ伝え、実対局の時間差を学習に用いる。[Baxter et al., 1999](https://arxiv.org/abs/cs/9901001)

将棋エンジンの開発では、自己生成した探索結果などを評価関数の学習に使う工程を「雑巾絞り」と呼ぶことがある。
この呼称が指す具体的な処理は実装に依存する。
elmoでは、学習時に採り直す静止探索末端の評価を、保存済みの教師値と対局結果の混合に近づける処理が確認できる。
NineDayFeverについて確認した2019年大会資料からは、開始局面の生成や特徴分解といった工夫を把握できる一方、損失式、勾配を与える局面、最適化手法の詳細は特定できていない。

TreeStrapの原論文では、同じ実験条件でRootStrapやTDLeafより効率よく学習した。
ただし、学習時の探索は通常対局より単純化し、null moveなどの高度な枝刈りを避け、静止探索でも知識に基づく枝刈りをしなかった。
minaseでは局面の評価に応じて枝を省略する選択的探索を使うため、TreeStrapを適用する前に、学習へ渡す値が原論文と同じ意味で上下界となるかを確認する必要がある。[Veness et al., 2009, §5〜6](https://proceedings.neurips.cc/paper_files/paper/2009/file/389bc7bb1e1c2a5e7e147703232a88f6-Paper.pdf)

### Bonanzaの探索後順位学習

Bonanzaは、棋譜の着手が探索後も他の候補手より高く評価されるように係数を調整する。
現行の係数で各候補手を探索し、選ばれた手順の末端評価の微分を使って更新する操作を反復する。
原論文の学習では深さ1の探索と静止探索を使っている。[Hoki and Kaneko, 2014, §3、§4.1](https://www.jair.org/index.php/jair/article/download/10871/25935)

minaseで深い教師が安定して区別する候補手の順位を学ぶ案は、この原理を自己生成教師へ移す提案である。
原論文の人間の棋譜を深い教師に置き換える効果は未検証であり、末端の採り直しと評価尺度の維持も必要になる。
教師値への回帰、探索後順位の学習、最適化手法の変更を独立に比較することで、改善した要因を特定しやすくなる。

minaseの完了済み実験は、教師への検証損失を下げることだけでは棋力改善を保証しないと示した。
一方、局所2駒表は未実装で、静止探索末端への置換や学習中の末端更新も直接検証されていない。
これらの未検証の手法については、[評価改善方針の実験結果と比較順序](evaluation-improvement-strategy.md)に基づいて、minaseでの学習効率と対局棋力を測る必要がある。

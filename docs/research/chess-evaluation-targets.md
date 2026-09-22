# チェス評価関数の教師と混合比

2026年9月22日に、公開コード、開発者の説明、および採用ネットの学習記録を確認した。
対象は、minaseの「探索値75%と対局結果25%」に対応する教師の作り方であり、学習器の既定値、実際の学習設定、および混合比を変えた比較を区別する。

## 結論

チェスエンジンには、対局結果だけを教師にする方式、評価値だけを教師にする方式、および両者を混ぜる方式がある。
75:25を学習終端の設定にした先例はStockfishにあるが、チェスに共通する標準値ではない。
手作りの評価項目の係数を求めるというminaseの目的に近いTexelとEtherealの公開方式は、対局結果だけを教師にする（[Texel作者の説明](https://www.talkchess.com/forum/viewtopic.php?start=26&t=50823)、[Etherealのラベル](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/src/tuner.c#L206-L226)、[Stockfishの75%を含む採用設定](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a)）。

ニューラル評価関数では、同じエンジンでもネットや学習段階によって比率を変えている。
Stockfishには学習中の比率を変える採用例があり、Viridithasには同じ学習基点から混合比を変えて対局比較した記録がある。
したがって、先行例から得られるのは混合と比較の方法であり、minaseでも75:25が最適だという根拠ではない（[Stockfishの設定](https://github.com/vondele/nettest/blob/75f28ef20995d22507e5094e0ac1721eea5ad285/threats.yaml#L93-L104)、[Viridithasの比較記録](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/notes/networkhistory.txt#L1103-L1210)）。

## 比較の記法

比率の向きを統一するため、教師評価値を0から1へ換算した値を \(q\)、勝ち1、引き分け0.5、負け0の対局結果を \(z\) とし、次の式で表す。

\[
 t_\alpha=\alpha q+(1-\alpha)z.
\]

minaseの現行設定は \(\alpha=0.75\) である。
この \(q\) は必ずしも自己探索の値ではなく、別のネットの評価出力で作る場合もある。
結果をこの尺度で回帰した値は、厳密には勝率ではなく、勝率に引き分け率の半分を足した得点期待値である。
勝ち、引き分け、負けの3確率を別々に出すLeelaの混合は、同じ考え方をベクトルに適用する。

| 対象 | 確認した教師の作り方 | 資料の性質 |
|---|---|---|
| Texel原法とEtherealの公開チューナー | 対局結果だけを教師にする。 | 手作りの評価項目の係数学習であり、詳細は次節の作者資料と実装による。 |
| Stockfish | 評価値だけの採用例も、学習中に評価値側の比率を変える採用例もある。 | 公式学習器と実際の採用記録を照合した。 |
| Viridithas | 結果側を40〜90%で比較した記録と、結果100%で追加学習した公開ネットがある。 | 作者の実験記録とネットのリリース説明を確認した。 |
| bulletの基本例 | 評価値25%と結果75%を混ぜる。 | 配布例の設定であり、特定エンジンの採用設定を意味しない。 |
| Leela Chess Zero | 結果側の教師と探索後の分布を混ぜる機能、および結果ラベルを補正する前処理がある。 | 公開実装と設定例を確認した。現行の配布ネット全体に共通する比率は確定していない。 |

## 手作り評価項目の係数学習

自己対局から手作り評価項目の係数を求める先例と学術研究は、[専用の調査](handcrafted-evaluation-selfplay.md)にまとめた。

Texel作者のPeter Österlundが2014年に説明した方法は、自己対局の局面について、係数 \(w\) を持つ評価関数で静止探索を行い、その予測を対局結果へ近づけるものである。
予測を \(s_w(x)\)、シグモイド変換を \(\sigma\) とすれば、平均二乗誤差 \(N^{-1}\sum_i[\sigma(s_w(x_i))-z_i]^2\) を小さくする。
静止探索の値は学習する予測側にあり、固定した探索値を教師に混ぜるわけではない（[作者の原説明](https://www.talkchess.com/forum/viewtopic.php?start=26&t=50823)）。

Etherealの公開チューナーも、駒価値、可動性、王の安全性などの重みを、対局結果との平均二乗誤差を使って学習する。
係数の更新には、過去の勾配の大きさに応じて更新幅を調整するAdaGradを使う（[評価項目](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/src/tuner.c#L46-L118)、[ラベル](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/src/tuner.c#L206-L226)、[損失](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/src/tuner.c#L343-L358)、[更新則](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/src/tuner.c#L147-L158)）。

Ethereal作者の解説では、自己対局から取った局面を深く探索し、最善とされた手順の終端局面へ変換してから学習に使う。
この場合もラベルは元の対局結果であり、探索は入力局面の前処理に使われている。
作者は、変換後の局面と元の結果との対応が弱まり得る点も認めている（[作者解説§2.2](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/Tuning.pdf)）。

両作者は、学習後の棋力を実対局で検証している。
ただし、Etherealの2017年の結果報告は外部の局面データを使った初期の実験であり、後年の自己対局データによる方式の効果とは区別する必要がある。
教師への適合と棋力の改善を別に確認する点はminaseの方針と共通するが、ここで説明したのは公開された手作り評価関数の調整であり、現在の商用Etherealのネット学習を説明するものではない（[Texelの結果報告](https://www.talkchess.com/forum/viewtopic.php?start=26&t=50823)、[Ethereal作者の結果報告](https://www.talkchess.com/forum/viewtopic.php?t=65538#p736294)）。

## Stockfish

Stockfishの混合比は固定の75:25ではなく、採用ネットごとに設定されている。
公式学習器 `nnue-pytorch` の現行の既定値は `lambda=1.0` だが、2026年9月の採用ネットは学習中に比率を変える。
既定値と採用ネットの設定が異なるため、コードの既定値だけでは運用を説明できない。[既定値](https://github.com/official-stockfish/nnue-pytorch/blob/3276b3b5aa9e109751c95d434764d62aaa3909a8/model/config.py#L41-L72)、[採用コミット](https://github.com/official-stockfish/Stockfish/commit/bc944563790bf0b46a7acd8d9de139315d1b974f)

公式学習器の `lambda` は本書の \(\alpha\) に相当し、混合の式は \(t=\lambda q+(1-\lambda)z\) である。
したがって、\(\lambda=1\) は評価値だけ、\(\lambda=0\) は結果だけを教師にする。
現行実装は2つのシグモイドで評価値を得点期待値へ換算し、予測と混合後の教師の差の絶対値をべき乗した損失を使う。[混合と損失の実装](https://github.com/official-stockfish/nnue-pytorch/blob/3276b3b5aa9e109751c95d434764d62aaa3909a8/model/nnue.py#L39-L68)

過去の採用記録には、評価値だけで学習した例と、結果の割合を徐々に増やした例がある。
以下の引数は公開された学習コマンドから比率に関係する部分だけを抜き出したものである。

| 採用時点 | 公開された設定 | 読み取れる範囲 |
|---|---|---|
| 2021年6月3日 | `--lambda=1.0` | 当該学習は評価値だけを教師にする。[採用記録](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c) |
| 2022年7月1日 | `--start-lambda=1.0 --end-lambda=0.75` | 学習開始時と終了時の比率を指定する。[採用記録](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a) |
| 2023年6月7日 | 最終の再学習で `--start-lambda 1.0 --end-lambda 0.7` | 終了時の設定を結果30%とする。ただし、採用されたのは800エポック予定の619エポック目である。[採用記録](https://github.com/official-stockfish/Stockfish/commit/932f5a2d657c846c282adcf2051faef7ca17ae15) |
| 2026年9月13日 | `start-lambda: 1.0`、`end-lambda: 1.0`、`lambda-cycle-delta: -0.3` | 余弦曲線による変化と小さな乱数変動を加える。[採用された設定](https://github.com/vondele/nettest/blob/75f28ef20995d22507e5094e0ac1721eea5ad285/threats.yaml#L93-L104) |

2026年9月の設定では、乱数変動を除く基本曲線の \(\lambda\) は1から0.7へ下がり、1へ戻る。
変化に指定した長さは5,052,000更新で、その25%の時点に0.7へ達し、指定更新数以降は1を保つ。
これは設定と学習器の式から計算できる変化であり、全学習を75:25で行うという意味ではない。[周期の設定](https://github.com/vondele/nettest/blob/75f28ef20995d22507e5094e0ac1721eea5ad285/threats.yaml#L93-L104)、[更新数の設定](https://github.com/vondele/nettest/blob/75f28ef20995d22507e5094e0ac1721eea5ad285/threats.yaml#L159-L185)、[使用した学習器](https://github.com/TonyCongqianWang/nnue-pytorch/blob/c8c6f7d4f227cfef46477f4c5a0131c76a11393e/model/lambda_utils.py#L22-L84)

教師の評価値の由来にも注意が必要である。
2026年9月の採用ネットの学習は、Leela Chess ZeroのBT4ネットの価値出力で評価値を付け直したデータを使う系統であり、`score`を常にStockfish自身の探索値と解釈できない。[付け直しの導入](https://github.com/official-stockfish/Stockfish/commit/9fcd47a717cfdf8c44127710fdb3325218a12259)、[9月の入力データ](https://github.com/vondele/nettest/blob/75f28ef20995d22507e5094e0ac1721eea5ad285/threats.yaml#L1-L45)

これらの採用記録は、設定全体が対局試験を通過した証拠である。
2022年の例ではデータと損失も変更され、2026年9月の例では学習時間を延ばしているため、特定の混合比だけの効果や最適性は分離されていない。
今回確認した一次資料からは、75:25を普遍的な最適値とする比較根拠は得られなかった。[2022年の変更範囲](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a)、[2026年9月の変更範囲](https://github.com/official-stockfish/Stockfish/commit/bc944563790bf0b46a7acd8d9de139315d1b974f)

## Viridithas

Viridithasは11.0.0以後bulletで学習しており、現在のネットは自前の自己対局データだけで学習する。
初期のネットでは自己対局棋譜を自エンジンの低深度探索で再評価していた。
この来歴は作者自身の [README、固定コミットの51〜79行](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/README.md#L51-L79) に記される。

作者の [比較記録](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/notes/networkhistory.txt#L1103-L1210) には、同じ学習済みネットretrochronから同量の追加学習を行い、対局結果側の重みを0.4から0.9まで変えたbasilisk群の結果がある。
以下はすべて共通の基準delendaとの比較であり、basilisk同士の直接対局ではない。
±は記録の95%区間に対応する。

| ネット | 結果側の重み | 探索側の重み | 1手25,000ノードでのElo差 | 40秒＋1手0.4秒でのElo差 |
|---|---:|---:|---:|---:|
| basilisk.4 | 0.4 | 0.6 | +0.71 ± 0.75 | −8.80 ± 4.35 |
| basilisk.5 | 0.5 | 0.5 | −0.31 ± 1.76 | −7.40 ± 4.00 |
| basilisk.6 | 0.6 | 0.4 | −4.85 ± 3.74 | −5.47 ± 3.55 |
| basilisk.7 | 0.7 | 0.3 | −13.95 ± 5.93 | −5.77 ± 3.59 |
| basilisk.8 | 0.8 | 0.2 | −19.27 ± 7.09 | −10.67 ± 4.86 |
| basilisk.9 | 0.9 | 0.1 | −26.61 ± 8.26 | −22.81 ± 6.95 |

この記録は、実際に混合比を変えて自己対局で測る開発が存在する証拠になる。
同じ学習基点と同じ追加学習量は明記されるが、乱数種を含む全条件や学習反復の分散まではこの表から確認できない。
1手25,000ノードでは結果側の重みを大きくするほど不利だったが、この比較から全エンジンに共通する最適比率を決めることはできない。

公開ネットv100のeleisonは2025年11月27日公開で、結果側の比率を0.4から1.0へ線形に増やす。
ほかに学習率、入力の扱い、データの間引きなども変更しており、混合比だけの比較ではない。
続くネットv101のinimicalは、2025年12月18日に公開された。
この関係は [v100の作者説明](https://github.com/cosmobobak/viridithas-networks/releases/tag/v100)、[v101の作者説明](https://github.com/cosmobobak/viridithas-networks/releases/tag/v101)、[固定されたネット履歴1604〜1682行](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/notes/networkhistory.txt#L1604-L1682) にある。

v102のnoumenaは2026年1月4日公開で、inimicalを対局結果100%で追加学習したネットである。
持ち時間40秒＋1手0.4秒、1スレッド、置換表128 MBで36,436局を指し、inimicalに対して+2.38 ± 1.74 Eloだった。
[リリース説明](https://github.com/cosmobobak/viridithas-networks/releases/tag/v102) と [固定されたネット履歴1765〜1781行](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/notes/networkhistory.txt#L1765-L1781) に確認できる。
[対局463](https://test.expositor.dev/test/463/) のHTMLも直接取得し、同一エンジンコミット7bd9381dで、noumena-b1200とinimical-b800を比較したことを確認した。
したがって追加学習後のネットの優位は示しているが、学習を継続した効果と、結果100%にした効果は分離されていない。
「結果100%が他の比率より強いと実証した」とは書けない。

## bulletの基本例

パラメータ `wdl` は対局結果側の重みを表す。
その値を \(w\) とすると、現行bulletの単一出力学習器は \(t=wz+(1-w)q\) を教師値とする。
この向きは [value.rs、固定コミットの103〜115行](https://github.com/jw1912/bullet/blob/2ea3d2d0f7e597b0d645f6e8040cf37818f51bce/crates/bullet_lib/src/value.rs#L103-L115) で確認できる。

[基本例simple.rs、40〜67行](https://github.com/jw1912/bullet/blob/2ea3d2d0f7e597b0d645f6e8040cf37818f51bce/examples/simple.rs#L40-L67) はw=0.75を一定にし、シグモイド変換したネット出力とtの二乗誤差を最小化する。
同例でq = sigmoid(search_score / 400)である。
これは探索25%と結果75%なので、minaseの75:25とは逆である。
冒頭の [1〜6行](https://github.com/jw1912/bullet/blob/2ea3d2d0f7e597b0d645f6e8040cf37818f51bce/examples/simple.rs#L1-L6) は、適切な混合比や学習率のスケジュールがデータセットに依存すると説明する。
これは学習器の配布例であり、Viridithasの特定の採用ネットがこの0.75を使ったという証拠にはならない。
またこの配布例の0.75の由来や、それだけを選んだ比較実験は確認していない。

## Leela Chess Zero

Leela Chess Zeroの公開学習コードにも探索値と結果の混合はあるが、75:25という一律の規則はない。
2026年9月22日に確認した学習リポジトリのコミットは `7c5d756ea6bb3531fb14a9b4df231577b1aa1081` であり、TensorFlow版と新しいJAX版の両方を含む。

TensorFlow版は、探索後の勝ち、引き分け、負けの分布を `q`、結果側の分布を `z` とし、`q_ratio * q + (1 - q_ratio) * z` を教師にする。
`q_ratio` の既定値は0であり、勝敗分布を出力する場合の損失は交差エントロピーである。
これは学習器の既定値であって、現在配布されるすべてのネットの学習設定を示すものではない（[混合と損失](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/tf/tfprocess.py#L485-L508)、[入力の変換](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/tf/chunkparser.py#L386-L403)）。

結果側の教師を、加工前の最終結果と同一視することもできない。
同リポジトリが参照するlc0のコミット `395573837488074361fad72096bc6feca345d646` には、実際に指した手と探索上の最善手の評価差から悪手を検出し、当該局面とそれ以前の局面の結果ラベルを探索値で補正する `ApplyDeblunder` がある。
したがって、`q_ratio=0` でも前処理を含めれば探索値を教師に使っている場合がある（[教師補正の実装](https://github.com/LeelaChessZero/lc0/blob/395573837488074361fad72096bc6feca345d646/src/trainingdata/rescorer.cc#L989-L1062)）。

新しいJAX版では、結果、最善手、実際に指した手などを教師として選ぶ損失を複数指定できる。
公開設定例は結果を学ぶ評価出力を持つが、同時に悪手に基づく教師補正も指定している。
これも本番の全ネットの設定ではなく、公開された設定例である（[損失の設定形式](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/proto/training_config.proto#L95-L121)、[交差エントロピーの実装](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/src/lczero_training/model/loss_function.py#L180-L207)、[設定例の教師補正](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/docs/example.textproto#L40-L55)、[設定例の損失](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/docs/example.textproto#L163-L180)）。

混合する理由について、開発者による2018年10月10日の解説は、探索後の予測を学ぶことを知識蒸留と捉え、最終結果を併用すれば探索の限界と自己強化による誤りを抑える可能性があると述べる。
この説明は混合の動機を与えるが、75:25の最適性を示す比較ではない（[開発者の解説](https://lczero.org/blog/2018/10/understanding-training-against-q-as/)）。

公開された比較として、2019年6月16日の公式報告は、探索値の混合と合法手のマスクを加えたtest52がtest51とおおむね同じ強さだったと記す。
複数の変更を含むため、混合比だけの効果は分離できない（[当時の学習報告](https://lczero.org/blog/2019/06/whats-going-on-with-training/)）。
本調査では、現行の配布ネットに使われた混合率を一意に確定できていない。

## 対局成績を直接使う係数調整

同時摂動確率近似法（SPSA）では、係数を少し増減した2つのエンジンを対局させ、勝敗差から係数の改善方向を推定する。
これは保存済み局面の教師値に適合させる方式とは別であり、評価値と結果を混ぜた教師を必要としない。
Stockfishの公式資料は、この手順と、得た係数を改めて対局検証する手順を説明している（[Fishtestの公式説明](https://official-stockfish.github.io/docs/fishtest-wiki/Creating-my-first-test.html#tuning-with-spsa)）。

## minaseへの含意

手作りの評価項目を対局結果だけから調整する方法にも、評価値を混ぜて学習する方法にも先例がある。
どちらを選ぶかは特徴の作り方だけでは決まらず、教師評価の精度、対局中の誤着、および学習するモデルに依存するというのが、以上の資料からの推論である。
75:25は比較候補として置けるが、先行例だけを根拠に固定することはできない。

混合比を比較する際は、各候補をそれぞれ異なる教師値に対する学習損失だけで順位付けできない。
目標そのものが変わっているからであり、診断には共通の対局結果への予測誤差などを使い、棋力への採否は同じ対局条件で確かめる必要がある。
Viridithasの実験は、学習基点と追加学習量をそろえ、独立した対局で比率を比べる具体例になる。

Stockfishの係数を移す前には、教師の由来の違いも考慮する必要がある。
minaseの[段階9の分析](../plans/strength-stage9.md#目的)は、教師の探索値にも王の危険を捉えない問題があると述べる。
別の大きなネットの評価で教師を付け直すStockfishの条件はこれと異なるため、同じ75%でも、継承する情報の質は同じとは限らない。
これは比率の変更だけでminaseの問題が解けるという結論でもなく、比率と教師の質を別々に確認すべきだという含意である。

## 調査の限界

本調査は公開資料の照合であり、学習や対局の再実行はしていない。
Viridithasの混合比比較では同じ学習基点と追加学習量が確認できるが、全候補の乱数条件と再学習によるばらつきまでは確認できない。
Stockfishの採用例の多くは教師データや学習時間なども変更しており、比率だけの効果は分離されていない。
Leelaの現行の配布ネット全体について、実際に使われた混合率を一意に確定していない。
したがって、いずれの結果も中将棋へそのまま移せる最適比率を示すものではない。

## 確認した版

| 対象 | コミット | 用途 |
|---|---|---|
| Stockfish | `17a6c8f1eb0da45c2ca405321919519bf4e211ba` | 取得時の先頭であり、既定ネットが `nn-134a887f4c8f.nnue` であることを確認した。[定義](https://github.com/official-stockfish/Stockfish/blob/17a6c8f1eb0da45c2ca405321919519bf4e211ba/src/evaluate.h#L32-L36) |
| nnue-pytorch | `3276b3b5aa9e109751c95d434764d62aaa3909a8` | 取得時の先頭であり、式と既定値を確認した。[コミット](https://github.com/official-stockfish/nnue-pytorch/commit/3276b3b5aa9e109751c95d434764d62aaa3909a8) |
| nettest #462 | `75f28ef20995d22507e5094e0ac1721eea5ad285` | 2026年9月の採用ネットの公開設定を確認した。[設定](https://github.com/vondele/nettest/blob/75f28ef20995d22507e5094e0ac1721eea5ad285/threats.yaml) |
| 採用ネットの学習器 | `c8c6f7d4f227cfef46477f4c5a0131c76a11393e` | 公開設定に固定された開発者の学習器で、余弦曲線と混合の方向を照合した。[実装](https://github.com/TonyCongqianWang/nnue-pytorch/blob/c8c6f7d4f227cfef46477f4c5a0131c76a11393e/model/lambda_utils.py) |
| Ethereal公開チューナー | `0e47e9b67f345c75eb965d9fb3e2493b6a11d09a` | 手作り評価関数の教師と損失を確認した。 |
| Viridithas | `13a3fe18fbfde3856262850d599b6b8465ee7329` | 作者のネット履歴と比率比較を確認した。 |
| bullet | `2ea3d2d0f7e597b0d645f6e8040cf37818f51bce` | 混合の向きと配布例を確認した。 |
| lczero-training | `7c5d756ea6bb3531fb14a9b4df231577b1aa1081` | TensorFlow版とJAX版を確認した。 |
| lczero-trainingが参照するlc0 | `395573837488074361fad72096bc6feca345d646` | 結果ラベルの補正を確認した。 |

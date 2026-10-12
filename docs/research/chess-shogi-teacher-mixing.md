# チェスと将棋の教師混合比

2026年10月12日に、公開された学習コード、開発者の説明、および学習設定の記録を確認した。
探索による評価と対局結果を混ぜる学習には、チェスと将棋の両方で実際の採用例がある。
Stockfishには学習中に混合比を変えた採用ネットがあり、将棋では少なくとも2017年のelmoに大会使用の記録がある。[Stockfishの採用記録](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a)、[elmoの大会設定](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L703-L724)
ただし、コードが提供する機能、学習器の既定値、実際に配布された評価関数の学習条件は区別する必要がある。
以下は確認した版と採用例の比較であり、各エンジンの全世代に共通する混合比を示すものではない。
チェスの教師の由来や混合比の比較実験については、先行する[チェス評価関数の教師と混合比](chess-evaluation-targets.md)にも記録がある。

## 比較の記法

エンジン間で比率を比べるため、探索側の教師を0から1へ換算した値を \(q\)、同じ側から見た対局結果を \(r\)、学習する予測を \(p\) とする。
対局結果は勝ち1、引き分け0.5、負け0に対応させる。
探索側の教師には、別の評価関数で付け直された値を使う場合もあるため、個々の資料ではその由来も確認する。
混合の向きはminaseの説明に合わせ、次の式に統一する。

\[
t_\lambda=\lambda q+(1-\lambda)r.
\]

この記法では、\(\lambda=1\) は探索側だけ、\(\lambda=0\) は対局結果だけを教師にする。
一方、ソースコードの変数名が `lambda` でも、正規化していない係数を指す場合がある。
以下では、元のパラメータと、この式に換算した比率を分けて示す。

| 対象 | minaseの記法での探索側の重み | 根拠の種類 |
|---|---|---|
| Stockfishの2022年7月の採用ネット | 1.0から0.75へ近づける。 | 採用コミットに学習コマンドがあり、使用した学習器も確認した。 |
| elmoの2017年大会版 | 1/3とする。 | 作者のコードに大会で使った係数が明記される。 |
| やねうら王の旧学習器 | 既定は0.33とする。 | 2026年5月に削除された実装の既定値である。 |
| BulletOu | 既定は1.0とする。 | 確認版の学習器の既定値である。 |
| dlshogi | 公開学習器の既定は0.333とする。 | 大会資料にも混合の記述があるが、大会での比率は特定できない。 |
| AobaZero | 2021年12月から0.5とする。 | 2022年の大会資料に変更内容が記載される。 |
| Viridithasの公開ネットeleison | 0.6から0へ線形に減らす。 | 2025年11月27日公開のネットの学習設定である。 |
| Leela Chess Zero | `q_ratio`を探索側の重みとする。 | 混合機能と過去の使用報告があり、共通の採用値は確定していない。 |

各行の一次資料と、混合比の換算は以下に示す。

## Stockfishの採用ネット

Stockfishの公式学習器は、教師評価値を得点期待値へ変換した値に`lambda`を、対局結果に`1-lambda`を掛けて足す。
名前も重みの向きもminaseと同じであり、確認版の設定は1を評価値だけ、0を結果だけと説明している。[公式学習器の混合](https://github.com/official-stockfish/nnue-pytorch/blob/3276b3b5aa9e109751c95d434764d62aaa3909a8/model/nnue.py#L39-L68)、[係数の定義](https://github.com/official-stockfish/nnue-pytorch/blob/3276b3b5aa9e109751c95d434764d62aaa3909a8/model/config.py#L41-L72)

2022年7月4日の採用コミットは、学習の開始値を1.0、終了側の値を0.75に指定したネットを既定にした。
使用した学習器の確認版では、エポック番号を予定エポック数で割った進行率で線形補間する。
採用されたのは予定800エポックに対する番号799のネットなので、当該エポックの係数は0.7503125となる。
したがって、評価値100%で始め、終盤には評価値約75%と対局結果約25%にする実採用例と説明できる。[採用コミットと学習コマンド](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a)、[当該学習器の補間と混合](https://github.com/vondele/nnue-pytorch/blob/06fd2ef24418cb0070930a599b2e9bf83238c33e/model.py#L298-L322)

一方、2021年6月3日の採用ネットには、`lambda=1.0`で学習した記録がある。
また、2022年の更新ではデータセットと損失関数なども変えている。
これらは特定の設定全体の採用を示し、75:25だけの効果や、Stockfish全体に共通する最適比率を示さない。[2021年の採用例](https://github.com/official-stockfish/Stockfish/commit/d53071eff4e75bc77dc86f65c52358d8014cb71c)、[2022年の変更内容](https://github.com/official-stockfish/Stockfish/commit/85f8ee6199f8578fbc082fc0f37e1985813e637a)

採用コミットの件名、学習コマンド、勝敗数はローカルのStockfishリポジトリでも照合し、得点率からElo差を再計算した。
2022年のコミットの作成日は7月1日だが、取り込まれた日付は7月4日であり、本書では後者を採用日の表記に使う。

## elmoの大会使用例

elmoの公開コードには、2017年の第27回世界コンピュータ将棋選手権で `LAMBDA = 0.5` を使ったという作者のコメントがある。
その学習処理は、保存された教師評価値と現在の評価をそれぞれシグモイド関数で変換し、対局結果も用いて勾配を計算する。
引き分けの局面は、この版では学習前に除外される。[大会設定と更新式](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L703-L724)、[引き分けの除外](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L679-L691)

この `LAMBDA` は、重みの和を1にした混合比ではない。
原コードの係数を \(\mu\) と書けば、更新に使う差は \((p-r)+\mu(p-q)\) であり、\(\mu=0.5\) の場合は次のように変形できる。

\[
(p-r)+0.5(p-q)
=1.5\left[p-\left(\frac13q+\frac23r\right)\right].
\]

したがって、教師値の重みをminaseと同じ尺度で表すと、探索側が1/3、対局結果が2/3となる。
一般には \(\lambda=\mu/(1+\mu)\) であり、原コードの0.5を「探索50%、結果50%」と読むと比率を取り違える。
上の換算は混合後の目標値についてのもので、元の式には勾配全体の1.5倍が残る。
やねうら王の開発者も、2017年5月27日の説明でこの正規化を明示している。[原実装](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L718-L724)、[開発者による正規化の説明](https://yaneuraou.yaneu.com/2017/05/27/elmoは何故2周目の雑巾を絞れないのか/)

この例は、学習器に混合機能があるというだけでなく、作者が大会使用を明記した証拠である。
ただし、この比率だけを変更した比較実験を示すものではなく、1/3が他の評価関数にも最適であるとは結論できない。
確認したコードは2017年6月7日のコミット `f825ff057165826c421d2d7ca9a841bdac2b9088` である。[確認版](https://github.com/mk-takizawa/elmo_for_learn/commit/f825ff057165826c421d2d7ca9a841bdac2b9088)

## やねうら王の旧学習器

やねうら王の旧学習器は、elmoの式を正規化した形で実装している。
混合に使うパラメータは `lambda` であり、勾配の計算は \((1-\lambda)(p-r)+\lambda(p-q)=p-t_\lambda\) となる。
損失の表示にも、混合後の教師値 \(t_\lambda\) に対する交差エントロピーを用いているため、ここでの `lambda` はminaseの混合比と同じ向きである。[勾配と損失の実装](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L1094-L1155)

確認版の既定値は `lambda=0.33`、`lambda2=0.33`、`lambda_limit=32000` である。
教師評価値の絶対値が `lambda_limit` 以上の局面では `lambda2` を使い、それ未満では `lambda` を使う。
既定では両者が等しいので混合比は変わらないが、設定を変えれば局面の評価値に応じて比率を切り替えられる。
これは学習器の設定機能と既定値の確認であり、配布された全評価関数が33:67で学習されたという証拠ではない。[既定値と分岐](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L1094-L1124)、[学習引数の読み取り](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L2677-L2681)

この旧学習器は、2026年5月13日に本体から削除された。
上記のコードは削除直前のコミット `15c1cc6ae0ee87998af0251149dc6f1b175d95a3` に固定して確認したものである。
2026年6月1日更新の公式Wikiは、やねうら王のニューラルネット評価関数の学習にBulletOuを案内しているため、旧コードの0.33を現行の学習器の既定値として紹介することはできない。[削除コミット](https://github.com/yaneurao/YaneuraOu/commit/0a6dd2cbd051dbe0951a7bc7d2a7471436a3d72a)、[公式Wikiの学習器案内](https://github.com/yaneurao/YaneuraOu/wiki/評価関数の学習#学習器)

## BulletOuの現行学習器

BulletOuも、評価値と対局結果を混ぜる `--lambda` を提供している。
引数の説明だけでなく、教師を作る処理でも `result_blend = 1.0 - lambda`、`score_blend = lambda` とし、両者の加重平均を計算している。
したがって、minaseと同じく `--lambda 0.75` は探索側75%、対局結果25%を意味する。[引数の定義](https://github.com/yaneurao/BulletOu/blob/29af3d083dc42fc9d89047be4071f37a58579ca7/examples/bulletou.rs#L4966-L4981)、[教師値の計算](https://github.com/yaneurao/BulletOu/blob/29af3d083dc42fc9d89047be4071f37a58579ca7/crates/bulletou_lib/src/value/teacher_batch.rs#L1708-L1748)

確認版の既定値は1.0であり、対局結果を直接混ぜず評価値側だけを教師にする。
これは2026年10月6日のコミット `29af3d083dc42fc9d89047be4071f37a58579ca7` の引数定義から確認した値で、やねうら王の特定の配布ネットの学習比率を確定するものではない。
現行コードにも混合の概念が残る一方、旧学習器と既定値が異なることから、エンジン名だけで学習条件を決めつけず、学習器の版と学習時の引数を確認する必要がある。[既定値](https://github.com/yaneurao/BulletOu/blob/29af3d083dc42fc9d89047be4071f37a58579ca7/examples/bulletou.rs#L4980-L4981)、[確認版](https://github.com/yaneurao/BulletOu/commit/29af3d083dc42fc9d89047be4071f37a58579ca7)

## dlshogiの損失の混合

dlshogiの公開学習器も、探索評価と対局結果を混ぜて価値を学習する。
2026年10月4日の確認版では、`val_lambda`の既定値は0.333である。
価値予測の損失は、対局結果への二値交差エントロピーに`1-val_lambda`、評価値から得た教師への二値交差エントロピーに`val_lambda`を掛けて足す。
したがって、既定の探索側33.3%、対局結果側66.7%という向きはminaseと同じである。
指し手を学ぶ損失も別に加えるが、ここでは価値予測の部分だけを比較している。[既定値](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/0cc16854474ab3c7c4f7be66c624f9f9001f7543/dlshogi/train.py#L44-L47)、[損失の混合](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/0cc16854474ab3c7c4f7be66c624f9f9001f7543/dlshogi/train.py#L320-L339)

データ読み込みも、対局結果と評価値から変換した教師を別々に渡している。
学習器には、指定したエポック数までに`val_lambda`を0へ減らす機能があり、公開設定例にもこの指定がある。
ただし、既定値と設定例は、すべての配布モデルの実際の学習条件を表さない。[教師データの対応](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/0cc16854474ab3c7c4f7be66c624f9f9001f7543/cppshogi/python_module.cpp#L45-L72)、[学習器への受け渡し](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/0cc16854474ab3c7c4f7be66c624f9f9001f7543/dlshogi/data_loader.py#L267-L283)、[比率の減衰](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/0cc16854474ab3c7c4f7be66c624f9f9001f7543/dlshogi/train.py#L304-L313)、[公開設定例](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/0cc16854474ab3c7c4f7be66c624f9f9001f7543/dlshogi/config.yaml#L16-L19)

実際の採用については、2023年3月26日付の第33回世界コンピュータ将棋選手権の説明資料にも、勝敗と探索評価値を教師とする交差エントロピーの和で学習することが記載される。
同資料は、探索側にモンテカルロ木探索で得た最善手の価値を使うと説明しているが、混合係数の具体値は示していない。
したがって、混合という方法の採用は確認できる一方、公開コードの現在の0.333を2023年大会の設定として遡って当てはめることはできない。[大会提出資料の5ページ第5.13節と6ページ第5.17節](https://www.apply.computer-shogi.org/wcsc33/appeal/appeal_final_230504.pdf#page=5)

## AobaZeroの50:50の採用例

AobaZeroは、2021年12月に価値の教師を最終勝敗だけから、最終勝敗と探索勝率の平均へ変更した。
2022年の第32回世界コンピュータ将棋選手権の資料に、この変更時期と方法が記録されている。
教師が\((q+r)/2\)なので、minaseの記法では\(\lambda=0.5\)に相当する。[大会提出資料の1ページ表1](https://www.apply.computer-shogi.org/wcsc32/appeal/AobaZero/2022aobazero_appeal.pdf#page=1)

同資料の第5節は、この変更による棋力の変化はなかったと報告する。
これは混合方式を実際に採用した証拠であると同時に、混合を取り入れるだけで棋力が上がるとは限らない例でもある。[同資料の2ページ第5節](https://www.apply.computer-shogi.org/wcsc32/appeal/AobaZero/2022aobazero_appeal.pdf#page=2)

## Viridithasの比率変更

Viridithasが使う学習器bulletでは、`wdl`または`blend`が対局結果側の重みを表す。
この係数を\(w\)とすると教師は\(w r+(1-w)q\)であり、minaseの\(\lambda\)は\(1-w\)に対応する。
2025年11月27日公開のネットeleisonでは、この結果側の重みを0.4から1.0へ線形に増やす。
minaseと同じ向きに換算すると、探索側の重みを0.6から0へ減らす学習である。[作者による学習器の説明](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/README.md#L51-L79)、[bulletの教師値の式](https://github.com/jw1912/bullet/blob/2ea3d2d0f7e597b0d645f6e8040cf37818f51bce/crates/bullet_lib/src/value.rs#L103-L115)、[eleisonの公開時の説明](https://github.com/cosmobobak/viridithas-networks/releases/tag/v100)

作者は、同じ学習済みネットから同量の追加学習を行い、結果側の重みを0.4から0.9まで変えたbasilisk群の比較も公開している。
minaseの記法では、\(\lambda\)を0.6から0.1まで変えた比較に当たる。
各候補の対局相手は共通の基準ネットであり、候補同士の直接対局ではない。
この記録は比率を実際に比較して選ぶ開発の証拠だが、乱数種によるばらつきまで分離した実験とは確認できない。[作者の比較記録](https://github.com/cosmobobak/viridithas/blob/13a3fe18fbfde3856262850d599b6b8465ee7329/notes/networkhistory.txt#L1103-L1210)

## Leela Chess Zeroの分布の混合

Leela Chess ZeroのTensorFlow版学習器も、`q_ratio`を探索側の重みとして、探索後の評価と結果側の教師を混ぜる。
勝ち、引き分け、負けの3確率を予測する場合は分布を混ぜるので、minaseの単一の数値とは出力形式が異なるが、同じ加重平均である。
確認版の既定値は0であり、配布されるすべてのネットの学習比率を示す値ではない。[混合の実装](https://github.com/LeelaChessZero/lczero-training/blob/7c5d756ea6bb3531fb14a9b4df231577b1aa1081/tf/tfprocess.py#L485-L508)

2019年6月16日の公式報告は、実験系列test52で探索後の値を最終結果に混ぜたことを記す。
混合の具体比率は記載されず、合法手の扱いも同時に変えているため、混合比だけの効果は特定できない。
また、結果側の教師は前処理で補正される場合があり、混合式の結果側を常に加工前の勝敗とみなすこともできない。[公式の学習報告](https://lczero.org/blog/2019/06/whats-going-on-with-training/)、[結果ラベルの補正](https://github.com/LeelaChessZero/lc0/blob/395573837488074361fad72096bc6feca345d646/src/trainingdata/rescorer.cc#L989-L1062)

## 教師値と損失の混合

これらの実装では、教師値を先に混ぜる場合と、2つの教師に対する誤差を混ぜる場合がある。
二値交差エントロピーを \(H(a,p)=-a\log p-(1-a)\log(1-p)\) と定義すると、同じ予測 \(p\) に対して次の恒等式が成り立つ。

\[
\lambda H(q,p)+(1-\lambda)H(r,p)
=H\bigl(\lambda q+(1-\lambda)r,p\bigr).
\]

したがって、この損失では「教師を混ぜる」と「教師ごとの損失を混ぜる」は同じ目的関数になる。
予測をシグモイド関数 \(p=\sigma(x)\) で表せば、右辺の \(x\) に関する微分は \(p-t_\lambda\) であり、旧やねうら王の更新式に対応する。
これは上の定義からの代数的な帰結である。
損失関数が異なる場合は同じ恒等式が成立するとは限らないため、変数名や見た目の式だけで同一視せず、実際の損失を確認する必要がある。

## minaseへの含意

先行例は、探索側と対局結果側の両方を学習に使う方法、比率を学習中に変える方法、および片方だけを使う方法が実用されていることを示す。
一方、教師評価値の由来、学習するモデル、データの分布は異なるため、他エンジンの数値だけを根拠にminaseの最適比率を決めることはできない。
Leela Chess Zeroの開発者は、探索によって改善した判断を評価関数へ学ばせることを知識蒸留として説明し、対局結果との併用が探索の限界や誤りの自己強化を抑える可能性を述べている。
これは混合する理由の説明であり、特定比率の最適性を示す比較結果ではない。[開発者の説明](https://lczero.org/blog/2018/10/understanding-training-against-q-as/)

本調査は公開資料とソースコードの照合であり、学習や対局を再実行していない。
minaseで比率を選ぶ場合は、データ、学習量、初期値などをそろえた候補を作り、共通の対局条件で比較する必要がある。
異なる比率で作った教師への損失は目標自体が異なるため、その数値だけで候補の棋力を順位付けできない。

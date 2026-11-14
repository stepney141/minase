# 強豪のNNUE学習器の学習レシピとminaseの学習条件の比較

本書のminaseに関する記述は2026年11月6日時点のものである。
「minaseの標準学習レシピの候補」の節が先に測るとした予定の総長と書き出すネットの比較は、同日のSTCでどちらも`H0`だった（[第2世代の教師での学習率の予定と書き出すネットの比較](../measurements/nnue-generation-reset/nnue-generation-reset-gen2-cosine10.md)）。
その後に起案した[NNUEの標準レシピv1を対局で決め、v0から世代反復をやり直す設計書](../plans/archived/nnue-standard-recipe.md)では、線形の経路の学習率、学習後の補正の要否、および直前の世代の教師の併用の3項目だけを対局で決め、ほかの学習条件は現行のまま固定する。

本書は、中将棋エンジンminaseのNNUE（着手ごとに差分で更新できる小さなニューラルネット評価関数）の標準学習レシピを開発者が決める判断材料として、強豪の学習器の学習レシピとハイパーパラメーターを一次資料で調べた調査メモである。
資料は2026年11月5日に参照した。
本書の作成では学習、対局、および長時間の計算を行っていない。
教師の生成は[強豪エンジンの教師生成の調査](teacher-generation-strong-engines.md)、tataraの構造と技法の転移は[将棋のNNUE学習器tataraからminaseへ移せる技法](tatara-nnue-transfer.md)、探索値と対局結果の混合比は[チェスと将棋の教師混合比](chess-shogi-teacher-mixing.md)が扱っているので、本書は学習率の予定と学習量を中心に書き、重なる項目はリンクにとどめる。

本書の語を次のとおり定める。
周回数は、学習に流した局面の延べ数をデータの一意な局面数で割った値である。
更新回数は、最適化器が重みを更新した回数である。
エポックは学習器ごとに意味が違うので、学習器の用語として引くときだけ使う。
superbatchはbulletとtataraの学習量の単位であり、既定では16,384局面のバッチを6,104回流す約1億局面である。
学習率の倍率は初期値を1とした値であり、×0.1は初期値の10分の1を表す。
学習率の総和は、更新ごとの学習率を全更新にわたって足した値である。
Adam系の最適化器は1更新で各重みをおおむね学習率の大きさだけ動かすので、本書はこの総和を重みが動ける距離の目安として使う（推論）。
λは教師値に占める探索値の割合であり、minaseの記法で統一する。
tataraとbulletの`wdl`は対局結果の割合なので、λ = 1 − `wdl`である。
群はパラメータの集まりごとに別の学習率を与える単位（parameter group）である。
P型はminaseの入力特徴集合の1つで、駒と升の組だけを入力とする。
AdamWは重み減衰を勾配から分離したAdamであり、RangerはRAdamの更新に低速の重みへの補間（lookahead）を組み合わせた最適化器である。
STCとLTCはそれぞれ短い持ち時間と長い持ち時間の対局の条件であり、SPRTとGSPRTは対局の勝敗から強さの差を逐次判定する検定である。
`H1`は候補が有意に強いという判定、`H0`は有意に強いとは言えないという判定である。
nEloは先後を入れ替えた2局の組の結果から求めた正規化Eloであり、Eloは得点率からロジスティック曲線で換算する単位である。
2つの単位は換算しない。
Stockfishとnodchip氏のSPRTのEloは、記録された勝ち、負け、引き分けの数から本書が再計算した値である。
逐次検定は判定が出た時点で止まるので、この点推定は真の差より大きく出やすい。
出典のない見積もりと設計上の含意には「推論」と付ける。

minaseの現行の学習は次のとおりである。
最適化手法はAdamW（β1=0.9、β2=0.999、ε=1e-8）で、学習率は第1層と出力部が0.001、線形の経路が0.03、重み減衰は1e-4（線形の経路は0）である（[options.rs](../../crates/minase-trainer/src/nnue/options.rs)、[adam.rs](../../crates/minase-trainer/src/nn/adam.rs)）。
バッチは16,384局面で、予定は`cosine:3:0.1`であり、3周で倍率を0.1まで下げて以後は0.1のままとする（[schedule.rs](../../crates/minase-trainer/src/nnue/schedule.rs)）。
検証損失が5周続けて改善しなければ止め、検証損失が最良の周の重みを書き出す（[args.rs](../../crates/minase-trainer/src/nnue/train/args.rs)）。
λ=1.0で学習した後に、線形の経路を対局結果へ補正する（[手引き](../guides/nnue-training.md#線形の経路を対局結果で補正する)）。
第2世代の教師は訓練96,462,343局面、検証5,162,867局面、1周は5,888更新である。

## 要約

第1に、`cosine:3:0.1`が極端かどうかは比べる単位で変わる。
周回数で比べると、2周の時点の倍率はminaseが×0.325、tataraの公開の標準レシピが×0.277であり、ほぼ同じである。
更新回数で比べると、倍率が半分になるまでがminaseは9,458更新、tataraは約52.7万更新であり、minaseは約56倍速く下げている。
下げる深さはminaseのほうが小さく、minaseの下限が×0.1であるのに対して、tanuki-の400 superbatchは×0.040、Stockfishの本番は×0.00004まで下げる。
したがって、minaseが速いのは更新回数で測った下げ方であり、深く下げているわけではない。

第2に、教師の量が近い先例は1通りではない。
Fairy-Stockfishの手引きが示す2億局面の例は1周で終わり、倍率は×0.877までしか下がらない。
Saplingの1.1億局面のネットは約58周、約39万更新を流し、倍率を×0.027まで下げた（周回数と更新回数はsuperbatchの既定の大きさを仮定した推論）。
minaseの第2世代は7周、41,216更新で止まり、学習率の総和は第1層で約12.1である。
Fairy-Stockfishの例は約17.3、Saplingは約138であり、`cosine:10:0.1`を10周まで流すと約32.4になる。
どの先例も、この予定そのものを他の予定と対局で比べた記録を残していない。

第3に、学習率を対局で測った記録は少ないが、次の傾向は複数の資料で一致する。
早く下げすぎた学習と短い学習は負けている。
nodchip氏のReduceLROnPlateauはR−30.6 ±9.7で、nodchip氏は高い学習率での学習が足りなかったためと考えた。
Viridithasの短い学習は、superbatch数を60から160へ増やすと、固定ノードでの基準との差が−107.30 Eloから−8.47 Eloまで縮んだ（データ量も同時に変わる）。
同じ学習の後のエポックが前のエポックに勝った記録もある。
一方で、低い学習率の追加学習は採否が分かれる。
Stockfishとtanuki-には採用の記録があるが、tanuki-はさらに下げた学習率で不採用が続き、新規学習の学習率を4分の1にした比較は検証損失が下がったのにEloは−5.73（95%区間−12.96〜+1.51）で判定不能だった。
いずれも学習率以外の変更を含むか、学習率だけの効果を分離していない。

第4に、検証損失で最良の重みを選んで書き出すことを標準とする現行の学習器は、調べた範囲ではminaseだけである。
やねうら王の旧学習器とBulletOuの任意の`plateau`は、検証損失で採否を決めて学習率を下げながら続ける別の方式である。
tataraの公開実験とtanuki-は事前に決めた学習量の最後のネットを使い、Stockfishは対局でネットを選ぶ。
nnue-pytorchのwikiは、棋力の評価には対局が要ると書いている。

第5に、検証損失が2周以内で最良になる現象を直接論じた先行実装の開発者の記述は見つからなかった。
tataraの公開実験では検証損失の最良は160 superbatchのうち157〜159 superbatchであり、この形の症状は出ていない。
最も近いのは、やねうら王の作者が教師の少ない学習について書いたコメントである。
minaseの第2世代は訓練損失もほぼ止まっており、訓練損失だけが下がる典型的な過学習の形ではない。

第6に、`--lr-gamma 0.992`、初期値8.75e-4、16,384×6,104、Rangerは、調べた範囲では対局で選ばれた値ではなく、先行実装を引き継いだ既定値である。
tataraの0.992は、nnue-pytorchに揃えた変更でありSPRTは未実施と明記されている。
これをminaseへ局面数で換算すると1周ごとに0.992倍となり、86周で半分になるので、そのまま使う根拠はない。

第7に、次に対局で測る価値が高い候補は、予定の総長と書き出すネットの選び方である。
第2世代の`--patience 5`を引き継ぐと、最良が早い周に出たときに倍率が0.1へ下がる前に止まる。
たとえば最良が3周なら8周で止まり、そのときの倍率は約×0.19であり、総長10周の効果を測ったことにならない。
このため、`cosine:10:0.1`の比較（測定名`nnue-generation-reset-gen2-cosine10-stc`）は、途中から`--epochs 10 --patience 50`へ改めて10周まで流す。
候補と比較の案は[候補の節](#minaseの標準学習レシピの候補)に書く。

## 調べた版

| 対象 | 版 | 確認の方法 |
|---|---|---|
| [tatara](https://github.com/SH11235/tatara/tree/87f19551ceeffa072b8274d5bec88981ab33ffe1) | `87f1955`（2026年11月5日） | shallow clone、PR、およびissue |
| [nnue-lab](https://nnue-lab.sh11235.com/t/sh11235)の公開実験 | 実験ごとのID | 公開APIの`/api/tenants/sh11235/experiments/<ID>`の記録 |
| [rshogiの実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)と[公開記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187) | 2026年7月 | `gh`の読み取りと本文 |
| [nnue-pytorch](https://github.com/official-stockfish/nnue-pytorch/tree/c625702757b58999611b006b5a03e3e1c5e142ea) | `c625702`（2026年11月1日） | shallow clone |
| [nnue-pytorchのwiki](https://github.com/official-stockfish/nnue-pytorch/wiki) | `0e18032`（2026年10月10日） | shallow clone |
| [nettest](https://github.com/vondele/nettest/tree/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f)（Stockfishの本番のレシピ） | `d0bacd0`（2026年11月4日） | 複製と`threats.yaml`の履歴、PR |
| Stockfish | `5c4cef0b`（2026年10月13日） | 手元の複製で件名と勝敗数を照合した |
| 本番のレシピが固定する学習器 | [`ec2ead9`](https://github.com/official-stockfish/nnue-pytorch/commit/ec2ead9feeb5677b828536fc53000d1379a1d207)（[threats.yaml 103〜104行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L103-L104)） | `c625702`との差は3コミットであり、本書が引いた行のうち変わるファイルは`model/nnue.py`の399行付近の1行の追加だけなので、引いた行の内容は同じである |
| [variant-nnue-pytorch](https://github.com/fairy-stockfish/variant-nnue-pytorch/tree/0536db20871d5d849402e1541c8a29722dc5a69d) | `0536db2`（2026年11月2日） | shallow clone |
| [variant-nnue-pytorchのwiki](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki) | `6688041`（2026年2月19日） | shallow clone |
| [bullet](https://github.com/jw1912/bullet/tree/5d70177680437b7c0ddf958bd4722d95afa78b0c) | `5d70177`（2026年11月5日） | shallow clone |
| [Viridithas](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt) | `7489e70`（2026年11月4日） | shallow cloneの`notes/networkhistory.txt` |
| [Sapling](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/Resources/WeightsHistory/log.txt) | `2b43285` | `gh`で`log.txt`を読んだ |
| [BulletOu](https://github.com/yaneurao/BulletOu/tree/bc0a32348b06efa9cda67510322f5ae021f838f7) | `bc0a323`（2026年11月5日） | shallow clone |
| BulletOu-minishogi | 手元の`cc202f3`（`stepney141/BulletOu`の`shogi-support`） | ブランチとファイル名を確かめた |
| [やねうら王の旧学習器](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp) | `15c1cc6a`（削除の直前の版） | 手元の複製。`learner/`は`0a6dd2cb`（2026年5月13日）で削除された |
| [nodchip/nnue-pytorch](https://github.com/nodchip/nnue-pytorch) | 先頭`5778b81`、公開実験が使った枝`395cfbb` | shallow cloneと`gh` |
| [nodchip氏のブログ](https://nodchip.hatenablog.com/) | 個別の記事のURLを本文に示す | 本文 |

## 学習器ごとの比較表

本節の表は、既定値と本番の値を別の行に分ける。
本番の行は、その設定で実際にネットを作った記録がある行である。
分からない項目は「未確認」と書く。

### 最適化手法と学習率

| 学習器 | 区分 | 最適化手法 | 初期学習率 | 減衰の形と単位 | 下限 | 群ごとの差 |
|---|---|---|---|---|---|---|
| tatara | 既定 | Ranger（β1=0.99、β2=0.999、ε=1e-8、k=6、α=0.5）。[cli.rs 600行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L600-L601)、[optimizer.rs 122〜130行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/crates/nnue-train/src/optimizer.rs#L122-L130) | 8.75e-4（[273行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L273-L274)） | 1 superbatchごとに×0.992（[290〜303行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L290-L303)）。一定、線形、余弦、指数、1周期も選べる | 階段の予定には下限の引数がない | 既定は同じ。倍率の引数はある（[633〜638行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L633-L638)） |
| tatara | 本番（公開実験、2026年7月） | Ranger | 8.75e-4 | ×0.992/superbatch。160 superbatchで×0.277（[実験](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTJ1J6LP022C3207CAC821EB)） | なし | なし |
| tanuki- | 本番（2026年10月の新規学習） | Ranger（tatara） | 8.75e-4 | ×0.992/superbatch、400 superbatchで×0.040（[記事](https://nodchip.hatenablog.com/entry/20261020-135531-79b45baf-bac3-46db-aa09-39d6b877eace)） | 記載なし | 記載なし |
| tanuki- | 本番（2025年の新規学習） | SGD、momentum 0.9（nnue-pytorchの派生） | 0.5 | warmup 1万バッチの後、500エポック（5億局面）ごとに検証損失が下がらなければ×0.5。倍率1e-5で打ち切り（[記事](https://nodchip.hatenablog.com/entry/2025/05/08/000000)） | 倍率1e-5 | 全群同じ。warmupと倍率の計算は[model.py 158〜201行](https://github.com/nodchip/nnue-pytorch/blob/395cfbb37ad0e365ffcbe12dac08e5d173fe1727/model.py#L158-L201) |
| やねうら王の旧学習器 | 既定 | SGD（momentumの既定0） | 引数`eta`。0を与えると30（[learning_tools.h 69行](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learning_tools.h#L69-L71)） | `newbob_decay`の既定は1.0で無効（[learner.cpp 2622行](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L2621-L2623)） | なし | 未確認 |
| やねうら王の旧学習器 | 本番の例（[2018年の記事](https://yaneuraou.yaneu.com/2018/12/30/nnue%E8%A9%95%E4%BE%A1%E9%96%A2%E6%95%B0%E3%81%AE%E5%AD%A6%E7%BF%92%E6%96%B9%E6%B3%95%E3%81%AB%E3%81%A4%E3%81%84%E3%81%A6/)、[tanuki- 2021年](https://nodchip.hatenablog.com/entry/2021/11/25/000000)） | SGD | `eta` 1.0 | 保存の間隔ごとに検証損失が下がらなければ最良の重みへ戻して×0.5（[learner.cpp 2171〜2197行](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L2171-L2197)）。既定では2回の不採用で止まる | なし | 未確認 |
| nnue-pytorch | 既定 | RangerLite（[config.py 19行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/config.py#L19-L21)）、lookahead 5回、α=0.5（[rangerlite_wrapper.py 17〜21行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/rangerlite_wrapper.py#L17-L21)） | 8.75e-4（[36行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/config.py#L36-L37)） | 1エポックごとに×0.992（[lr_scheduler.py 9行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/lr_scheduler.py#L9-L10)） | なし | 全群同じ（[nnue.py 140行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/nnue.py#L140)） |
| Stockfish | 本番（2026年10月31日のレシピ） | AdamW（β1=0.9、β2=0.999、ε=1e-8）（[threats.yaml 62行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L62)、[adamw_wrapper.py 11〜18行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/adamw_wrapper.py#L11-L18)） | 4e-4（[163行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L161-L163)） | 更新ごとの1周期。最初の5%で1.6e-5から上げ、5,108,000更新で1.6e-8まで余弦で下げる（[91〜92行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L91-L92)、[lr_scheduler.py 39〜50行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/lr_scheduler.py#L39-L50)） | 1.6e-8 | 全群同じ |
| Stockfish | 本番（2022年、2023年の再学習） | Ranger系（版は未確認） | 4.375e-4（第1段は8.75e-4に×0.992、または9.75e-4の一定） | ×0.995/エポック。平らにして長く学ぶためと説明（[cb9c2594](https://github.com/official-stockfish/Stockfish/commit/cb9c2594fcedc881ae8f8bfbfdf130cf89840e4c)、[932f5a2d](https://github.com/official-stockfish/Stockfish/commit/932f5a2d657c846c282adcf2051faef7ca17ae15)） | なし | 未確認 |
| variant-nnue-pytorch | 既定かつ手引き | Ranger（β=(0.9, 0.999)、ε=1e-7）（[model.py 310行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L310)）、k=6、α=0.5（[ranger.py 42〜44行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/ranger.py#L42-L44)） | 1.5e-3（[298行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L298)） | ×0.987/エポック（[312行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L312)） | なし | 出力層は10分の1（[306〜307行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L306-L307)） |
| bullet | 例（simple） | AdamW（β1=0.9、β2=0.999）（[adam.rs 29行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/crates/trainer/src/optimiser/adam.rs#L29)） | 1e-3 | 18 superbatchごとに×0.1（[simple.rs 68行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/simple.rs#L68)） | なし | なし |
| bullet | 例（advanced） | AdamW | 3段。1e-4→5e-3→1e-4、1e-3→1e-6、1e-5→1e-7（[main.rs 195〜219行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/advanced/main.rs#L195-L219)） | 線形 | 各段の終点 | 層ごとの重みの範囲だけ変える |
| Viridithas | 本番 | 未確認 | 1e-3 | 800 superbatchで余弦により2.7e-5まで（[1049〜1052行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1049-L1052)）。後に微調整を重ねる | 2.7e-5 | 未確認 |
| Sapling | 本番（1.1億局面） | 未確認 | 1e-3 | 16 superbatchごとに×0.3（[59〜64行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/Resources/WeightsHistory/log.txt#L59-L64)） | ×0.027 | 未確認 |
| Sapling | 本番（25.5億局面） | 未確認 | 1e-3 | 余弦で8.1e-6まで（[194〜198行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/Resources/WeightsHistory/log.txt#L194-L198)） | 8.1e-6 | 未確認 |
| BulletOu | 既定（検証で採否を決める`plateau`も選べる、[4924〜4928行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/examples/bulletou.rs#L4924-L4928)） | Ranger（[4900〜4904行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/examples/bulletou.rs#L4900-L4904)） | 8.75e-4（[4897行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/examples/bulletou.rs#L4897-L4898)） | エポックの最後に下限へ届く倍率を自動で計算し、次のエポックで初期値へ戻す（[tuning.md 144行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/docs/ja/advanced/tuning.md?plain=1#L144)） | 1e-5（[4943行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/examples/bulletou.rs#L4943-L4944)） | 未確認 |
| minase | 本番 | AdamW（β1=0.9、β2=0.999、ε=1e-8） | 第1層と出力部1e-3、線形の経路0.03 | 更新ごとの余弦。3周で×0.1 | ×0.1 | 線形の経路だけ30倍の値。単位が違うので大きさは他の学習器と比べられない |

### エポックの定義と学習量

| 学習器 | エポックの定義 | バッチ | 1単位の更新回数 | 総更新回数 | 延べ局面 | 一意な局面 | 周回数 |
|---|---|---|---|---:|---:|---:|---:|
| tatara（公開実験） | エポックの語を使わない。superbatchは6,104バッチ（[quickstart 184〜188行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/docs/training-quickstart.ja.md?plain=1#L184-L188)） | 16,384 | 6,104 | 976,640 | 160億 | [8,000,057,305](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTGQISAM5AD660DC041F20F7) | 2.00 |
| tanuki-（2026年） | 同上 | 16,384 | 6,104 | [2,441,600](https://nodchip.hatenablog.com/entry/goal-20260910-022356-3f339663) | 400億 | 8,000,049,489 | 約5.0 |
| nnue-pytorch（既定） | 固定局面数。1エポック1億局面（[config.py 94行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/config.py#L94)）。データの1周ではない | 16,384 | 6,103 | 4,882,400（800エポック） | 800億 | 指定による | 指定による |
| Stockfish（本番） | 固定局面数。1エポック2³⁰局面（[threats.yaml 63行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L63)）。2026年10月7日に2²⁷局面から変わった（[PR #497](https://github.com/vondele/nettest/pull/497)） | 131,072、最後の40エポックは65,536（[164〜188行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L165-L188)） | 8,192、後段は16,384 | 5,242,880 | 約6,442億 | 未確認（公開文は1,000億超） | 未確認 |
| variant-nnue-pytorch | 固定局面数。1エポック2,000万局面（[train.py 38行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/train.py#L38)）。手引きは2億局面に10エポックを勧める（[NNUE training 57行](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/NNUE-training/668804196697bb157ae3f066382e60a8c46f0215)） | 16,384。引数の説明の8192はコードと食い違う（[31行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/train.py#L31)、[69〜71行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/train.py#L69-L71)） | 1,220 | 12,200 | 2億 | 2億 | 1.0（読み飛ばしを含む読み込みは約4周） |
| bullet（simple） | エポックの語を使わない | 16,384 | 6,104 | 244,160（40 superbatch） | 40億 | 指定による | 指定による |
| bullet（advanced） | 同上 | 131,072（[main.rs 170〜171行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/advanced/main.rs#L170-L171)） | 763 | 839,300（1,100 superbatch） | 1,100億 | 指定による | 指定による |
| Sapling（1.1億局面） | superbatch | 未確認（既定なら16,384） | 未確認（既定なら6,104） | 約390,656（推論） | 約64億（推論） | 1.1億 | 約58（推論） |
| Sapling（1,000万局面） | superbatch | 同上 | 同上 | 約146,496（推論） | 約24億（推論） | 1,000万 | 約240（推論） |
| Sapling（25.5億局面） | superbatch | 16,384 | 6,104 | 3,662,400 | 600億 | 25.5億 | 23.5（記載） |
| BulletOu | `--superbatches`個のsuperbatchをまとめた単位で、学習率を初期値へ戻す単位でもある（[tuning.md 33行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/docs/ja/advanced/tuning.md?plain=1#L33)）。データの1周ではない | 既定65,536（[3525行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/examples/bulletou.rs#L3525)） | 指定による | 指定による | 指定による | 指定による | 指定による |
| やねうら王の旧学習器 | `loop`が教師の周回数（[記事のコメント](https://yaneuraou.yaneu.com/2018/12/30/nnue%E8%A9%95%E4%BE%A1%E9%96%A2%E6%95%B0%E3%81%AE%E5%AD%A6%E7%BF%92%E6%96%B9%E6%B3%95%E3%81%AB%E3%81%A4%E3%81%84%E3%81%A6/)）。保存と判定は`eval_save_interval`局面ごと | `batchsize` 100万、内側の更新は`nn_batch_size` 1,000 | 未確認 | 未確認 | 上限は`loop`×局面数 | 指定による | `loop`が上限。不採用が続くと途中で止まる |
| minase（第2世代） | 訓練局面全体の1周 | 16,384 | 5,888 | 41,216（7周で停止）。最良は11,776 | 約6.8億 | 96,462,343 | 7（最良は2） |

### 損失とλ

| 学習器 | 既定のλ | 本番のλ | 損失の形 | 勝率の尺度 |
|---|---|---|---|---|
| tatara | 1.0（`--wdl 0`、[cli.rs 348行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L348-L349)） | 公開実験は約0.667 | sigmoid後の二乗誤差か、勝率モデルの差の2乗（[574行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L574-L575)） | HalfKPは600、SFNNは340と380、オフセット270 |
| tanuki-（2026年） | 記載なし | 記載なし（教師は2つの評価器の平均） | 勝率モデルの差の2.75乗 | `wrm-nnue2score` 600 |
| tanuki-（2024〜2025年） | 1.0 | 1.0、または収束後に0.2〜0.5へ下げる（[2024年11月](https://nodchip.hatenablog.com/entry/2024/11/11/000000)） | 記事により異なる | 361、511、723などを比較 |
| nnue-pytorch | 1.0（[model/config.py 53行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/config.py#L53)） | 下段のStockfishの行 | 勝率モデルの差の2.5乗（[107行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/config.py#L107)、[nnue.py 50〜83行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/nnue.py#L50-L83)） | 入力340、出力380、オフセット270 |
| Stockfish | nnue-pytorchの既定 | 1.0→0.7→1.0の余弦と乱数の揺らぎ（[threats.yaml 93〜100行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L93-L100)） | 2.434乗と、過大評価側の損失を重くする係数0.229（[85〜86行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L85-L86)） | 入力295.65、出力379.99（[87〜90行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L87-L90)） |
| variant-nnue-pytorch | 1.0（[train.py 29行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/train.py#L29)） | 手引きは既定のまま | 探索値と結果の二乗誤差の加重和（[model.py 274〜276行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L274-L276)） | 入力410、出力361（[267〜268行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L267-L268)） |
| bullet | 例はsimpleが0.25、advancedが0.8→0.8〜0.5→0 | Viridithasは[0.6](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1049-L1052)が多く、最後に[0で微調整した例](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1765-L1772)もある | sigmoid後の二乗誤差（[simple.rs 44行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/simple.rs#L44)） | 400 |
| Sapling | 未確認 | 0→0.7（[log.txt](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/Resources/WeightsHistory/log.txt)） | 未確認 | 400 |
| やねうら王の旧学習器 | 0.33（[learner.cpp 2604〜2606行](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L2604-L2606)） | 0.5、1.0など | 交差エントロピー | 600 |
| minase | 引数で必須 | 1.0、その後に線形の経路を対局結果へ補正 | 二値交差エントロピー | 教師ごとに推定（第2世代は707.9） |

混合比の詳しい経緯は[チェスと将棋の教師混合比](chess-shogi-teacher-mixing.md)と[チェス評価関数の教師と混合比](chess-evaluation-targets.md)にある。

### 局面の間引き

| 学習器 | 間引き | 出典 |
|---|---|---|
| nnue-pytorch（既定） | 捕獲と王手の局面を除く`filtered`と、探索値と結果の矛盾で確率的に捨てる`wld_filtered`が有効。無作為の読み飛ばしと序盤の除外は無効で、序盤の確率的な除外だけ20手まで有効 | [data_loader/config.py 7〜16行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/data_loader/config.py#L7-L16)、[training_data_loader.cpp 1000〜1013行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/data_loader/cpp/training_data_loader.cpp#L1000-L1013) |
| Stockfish（本番） | `random-fen-skipping` 3（4局面に1局面を使う）、18手以内を除外、32手までの確率的な除外、駒数の分布の調整。2026年10月4日に10から3へ減らした | [threats.yaml 69〜84行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L69-L84)、[training_data_loader.cpp 871〜874行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/data_loader/cpp/training_data_loader.cpp#L871-L874)、[PR #490](https://github.com/vondele/nettest/pull/490) |
| variant-nnue-pytorch | `filtered`が有効で、`random-fen-skipping`の既定は3 | [train.py 35〜36行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/train.py#L35-L36)、[training_data_loader.cpp 588〜609行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/training_data_loader.cpp#L588-L609) |
| tatara（公開実験） | なし（「fen skipping / score clamp なし」）。評価値の大きい局面を除く引数はある | [実験](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTGQISAM5AD660DC041F20F7)、[cli.rs 410〜420行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L410-L420) |
| bullet | 読み込み器の関数で指定する。例は16手目以上、王手なし、評価値の絶対値1万以下、捕獲でない通常の手だけ。`advanced`は確率的な読み飛ばしも行う | [simple.rs 96〜102行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/simple.rs#L96-L102) |
| Viridithas | 6局面に5局面を読み飛ばす変更をwarmupと同時に測り、STCで+3.12 ±2.63 Elo | [1405〜1411行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1405-L1411) |
| minase | 生成時に最善手が捕獲または成りの局面を記録しない。学習時の読み飛ばしはなく、`--train-subsample`は固定の部分集合を選ぶ比較用の引数である | [teacher-generation-prior-art.md](teacher-generation-prior-art.md)、[手引き](../guides/nnue-training.md#学習する) |

Stockfishとvariant-nnue-pytorchの読み飛ばしは読むたびに抽選するので、周をまたいで別の局面を使う。
minaseの`--train-subsample`は同じ部分集合を毎周使うので、同じ操作ではない。

### 検証、ネットの選び方、および継続学習

| 学習器 | 検証 | 最終のネットの選び方 | 継続学習 |
|---|---|---|---|
| tatara | 任意。同じファイルの末尾か別のファイル。局面単位の切り分け（[held-out-validation 16〜30行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/docs/held-out-validation.ja.md?plain=1#L16-L30)）。公開実験はfloodgateの856,923局面 | 公開実験は160 superbatchの最後。検証損失の最良への差し替えはない | `--init-from`は重みだけ、`--resume`は最適化器の状態も（[quickstart 22行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/docs/training-quickstart.ja.md?plain=1#L22-L23)） |
| tanuki-（2026年） | 入力の末尾を除外して各superbatchで131,072件を評価 | 「途中で最もロスの小さい時点は選ばず」、事前に決めた400 superbatchの終了時（[記事](https://nodchip.hatenablog.com/entry/20261019-125745-44ba217c-0ac8-44d9-b248-905245c76bcc)）。採否はSPRT | 重みを引き継ぎ、最適化器の状態と学習率の進行は初期化（[記事](https://nodchip.hatenablog.com/entry/goal-20260909-171439-3292c923)） |
| tanuki-（2025年まで） | 検証損失で学習率を下げる | 打ち切りの後のネットを対局で判定 | 既存のネットからの学習と乱数からの学習の両方 |
| nnue-pytorch | 既定は無効。指定しなければ訓練データから流す（[wiki](https://github.com/official-stockfish/nnue-pytorch/wiki/Basic-training-procedure-(train.py)/0e18032a596878725bd4d4772c24283bb605a4e2)、[train.py 77〜91行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/train.py#L77-L91)） | 手順はなし | `--resume-from-model`は重みだけ、`--resume-from-checkpoint`は全状態 |
| Stockfish | 2億6,843万局面を10エポックごとに流すが、`threats.yaml`は別の検証データを指定しない | `fastchess`のSPRTで段ごとに測り、最も高いnEloのネットを取る（[execute_recipe.py 44〜55行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/nettest/execute_recipe.py#L44-L55)）。過去はエポックを対局で選んだ（[932f5a2d](https://github.com/official-stockfish/Stockfish/commit/932f5a2d657c846c282adcf2051faef7ca17ae15)） | 段をチェックポイントでつなぐ（[130〜132行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L130-L132)） |
| variant-nnue-pytorch | 別のファイルか同じファイル。1回100万局面（[train.py 39行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/train.py#L39)） | 毎エポック保存し、利用者が選んで対局で確かめる（[Step-by-Step Guide 41〜42行](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Step-by-Step-Guide/668804196697bb157ae3f066382e60a8c46f0215)） | `--resume-from-model` |
| bullet | 任意（[settings.rs 23行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/crates/bullet_lib/src/trainer/settings.rs#L22-L23)） | 例は定期の保存と最後の保存。Viridithasはエポックを対局で選ぶ | 同じ最適化器のまま次の段へ進む例がある |
| やねうら王の旧学習器 | `validation_set_file_name`、なければ先頭の局面の一部 | 検証損失が最良の重みへ戻しながら進み、不採用が続くと止まる（[learner.cpp 2171〜2201行](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L2171-L2201)） | 既存のネットを読んで続ける |
| minase | 対局単位で分けた約5% | 検証損失が最良の周の重み。採否は段階ゲート | 行わない。毎世代初期化する（[設計書](../plans/archived/nnue-generation-reset.md)） |

### 正則化、クリップ、量子化、因子化、および初期値

| 学習器 | 重み減衰 | 勾配クリップ | 重みのクリップ | 量子化を考慮した学習 | 入力の因子化 | 第1層の初期値 |
|---|---|---|---|---|---|---|
| tatara | 既定0（[cli.rs 605行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/cli.rs#L605-L606)、[training.rs 850行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/bins/nnue_train/src/training.rs#L850)）。公開実験の記録も0 | なし。[issue #287](https://github.com/SH11235/tatara/issues/287)の候補 | 層ごとに量子化から導く | `--qat dense`は追加学習の任意機能で棋力は未測定（[tatara調査](tatara-nnue-transfer.md)） | 既定で有効（[PR #303](https://github.com/SH11235/tatara/pull/303)） | LayerStackは半値幅√(1/入力数)、Simpleは±0.01（[init.rs 252〜254行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/crates/nnue-train/src/init.rs#L252-L254)、[PR #363](https://github.com/SH11235/tatara/pull/363)） |
| nnue-pytorch | 既定0（[config.py 24〜34行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/optimizers/config.py#L24-L34)） | ノルム2.0で固定（[train.py 390行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/train.py#L390)） | あり（[model.py 38〜46行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/model.py#L38-L46)） | 活性化と重みの疑似量子化が既定で有効（[model/config.py 125〜128行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/config.py#L125-L128)） | 入力の名前に`^`を付けると有効 | 未確認（バイアスは一様±√(1/入力数)、[composed_feature_transformer.py 61〜63行](https://github.com/official-stockfish/nnue-pytorch/blob/c625702757b58999611b006b5a03e3e1c5e142ea/model/modules/feature_transformer/composed_feature_transformer.py#L61-L63)） |
| Stockfish（本番） | 因子化の層だけ4e-5（[threats.yaml 68行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L68)） | 同上 | 同上 | 同上 | 有効（[47行](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L47)） | 同上 |
| variant-nnue-pytorch | 0 | 未確認 | 出力に近い3つの層（[model.py 131〜135行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L131-L135)） | なし | 既定の入力名による | 未確認 |
| bullet | 0.01（[adam.rs 29行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/crates/trainer/src/optimiser/adam.rs#L29)） | 例にはない | ±1.98、層ごとに変える例あり | 例では丸めて保存するだけ | 例によって有効 | 例では正規分布（[main.rs 87行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/advanced/main.rs#L87)） |
| minase | 1e-4、線形の経路は0 | なし | あり（[model/mod.rs 569〜605行](../../crates/minase-trainer/src/nnue/model/mod.rs)） | なし。書き出し時に誤差を検査する | P型には意味がない（[tatara調査](tatara-nnue-transfer.md)） | 入力の次元Dに対して標準偏差√(1/(3D))の正規分布で、tataraとnnue-pytorchの一様分布と同じ分散（[記録](../measurements/nnue-architecture-revision/nnue-architecture-revision-init-training.md)） |

## 学習率の予定を周回数と更新回数へ直した比較

次の表は、学習率の予定を「倍率が半分になるまで」「倍率が0.1になるまで」「学習の終わり」の3点で、周回数と更新回数に直したものである。
「エポックあたりの減衰率」は単位が学習器ごとに違うので比べない。
周回数と更新回数の値は、前節の表の条件から本書が計算した。
学習率の総和は、第1層の学習率について計算した。

| 予定 | 半分まで（周回数 / 更新回数） | ×0.1まで（周回数 / 更新回数） | 終わりの倍率 | 総周回数 / 総更新回数 | 学習率の総和 |
|---|---:|---:|---:|---:|---:|
| minase `cosine:3:0.1`、最良の2周まで | 1.61 / 9,458 | 届かない | ×0.325 | 2 / 11,776 | 8.7 |
| minase `cosine:3:0.1`、3周まで | 1.61 / 9,458 | 3.00 / 17,664 | ×0.1 | 3 / 17,664 | 9.7 |
| minase `cosine:3:0.1`、停止の7周まで | 1.61 / 9,458 | 3.00 / 17,664 | ×0.1 | 7 / 41,216 | 12.1 |
| minase `cosine:10:0.1`、10周まで | 5.35 / 31,527 | 10.00 / 58,880 | ×0.1 | 10 / 58,880 | 32.4 |
| variant-nnue-pytorch 2億局面 | 届かない | 届かない | ×0.877 | 1 / 12,200 | 17.3 |
| Sapling 1.1億局面（推論） | 14.5 / 97,664 | ×0.09が29.1 / 195,328 | ×0.027 | 58 / 390,656 | 138 |
| Sapling 25.5億局面 | 約11.8 / 約183万 | 約18.9 / 約294万 | ×0.0081 | 23.5 / 3,662,400 | 約1,846 |
| tatara公開実験 | 1.08 / 526,753 | 届かない | ×0.277 | 2 / 976,640 | 483 |
| tanuki- 400 superbatch | 1.08 / 526,753 | 3.58 / 約175万 | ×0.040 | 5 / 2,441,600 | 641 |
| nnue-pytorchの既定 800エポック | 未確定 / 約52.7万 | 未確定 / 約175万 | ×0.0016 | 未確定 / 4,882,400 | 666 |
| Stockfish 2022年の再学習（×0.995） | 未確定 / 約84.4万 | 未確定 / 約280万 | ×0.018 | 未確定 / 4,882,400 | 524 |
| Stockfish 本番（ピークから） | 未確定 / 2,681,700 | 未確定 / 約411万 | ×0.00004 | 未確定 / 5,242,880 | 1,024 |
| bullet advancedの第2段 | 未確定 / 305,200 | 未確定 / 549,360 | ×0.001 | 未確定 / 610,400 | 306 |

tanuki-の2025年までの予定は検証損失で決まるので、この表に入れていない。
SGDは1更新で動く大きさが勾配に比例するので、学習率の総和を比べられない。
Rangerの低速の重みへの補間は実効の移動量を減らすので、Rangerの行の総和はAdamWの行より大きめに出る（推論）。

この表から次のことが読める。
同じ2周の時点で、minaseの`cosine:3:0.1`は×0.325、tatara公開実験は×0.277、`cosine:10:0.1`は×0.914である。
周回数で見るかぎり、現行の予定はtatara公開実験の下げ方に近く、`cosine:10:0.1`のほうがtatara公開実験より緩い。
一方で、tatara公開実験の半減までの更新回数は現行の約56倍、`cosine:10:0.1`の約17倍である。
更新回数で見るかぎり、どちらの予定もtatara、tanuki-、およびStockfishより桁違いに短い。

minaseの第2世代の学習は、更新回数で見るとtatara公開実験の7 superbatch目までの長さにあたる。
tataraのSFNN-1536の検証の符号一致率は、[7 superbatchで0.6837、160 superbatchで0.6985](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTJ1J6LP022C3207CAC821EB)だった。
ただしtataraのこの区間はデータの9%しか見ていないので、更新のたびに新しい局面を見ている。
minaseの2周目以降は同じ局面の再利用なので、同じ更新回数で同じだけ学べるとは言えない（推論）。

minaseの第2世代の推移は次のとおりである（`data/nnue-generation-reset/gen-2/train/training.jsonl`）。

| 周 | 累計の更新回数 | 周の終わりの倍率 | 訓練損失（周の平均） | 検証損失 |
|---:|---:|---:|---:|---:|
| 0 | 0 | ×1.000 | | 0.541553 |
| 1 | 5,888 | ×0.775 | 0.523965 | 0.521427 |
| 2 | 11,776 | ×0.325 | 0.523048 | **0.521349** |
| 3 | 17,664 | ×0.100 | 0.522818 | 0.521352 |
| 4 | 23,552 | ×0.100 | 0.522742 | 0.521371 |
| 5 | 29,440 | ×0.100 | 0.522712 | 0.521378 |
| 6 | 35,328 | ×0.100 | 0.522686 | 0.521406 |
| 7 | 41,216 | ×0.100 | 0.522663 | 0.521405 |

2周目以降で検証損失が動いた幅は0.00006であり、訓練損失の低下も3周目以降は周あたり0.0001未満である。
損失には教師の探索値の曖昧さの分が含まれるので、この幅だけでは学習が止まったのか教師の限界に達したのかを読み分けられない（推論）。
倍率が×0.1に下がった区間で訓練損失の低下が小さいことは前者と整合するが、後者を否定しない（推論）。
`cosine:10:0.1`の結果は、この2つを見分ける材料になる。
総長を伸ばして検証損失が下がれば前者を支持し、同じ値で止まれば後者を支持する。

### データ量が少ない場合の先例

minaseの1世代は約1億局面である。
Stockfishの本番の一意な局面は公開文で1,000億超なので約3桁多く、tatara公開実験の80億局面は約2桁多い。
この差は結論を次のように制約する。

第1に、更新回数の比較は上限の目安にしかならない。
tatara公開実験の更新はすべて2周以内の局面に対するものであり、Stockfishも公開文の局面数が正しければ約6周以内である（推論）。
1億局面で同じ更新回数を流せば、同じ局面を数百周見ることになる。
第2に、周回数だけで比べると更新回数の不足を見落とす。
1周の更新回数は局面数に比例するので、1億局面の2周はtatara公開実験の2周の80分の1の更新回数である。
第3に、1局から取る局面の相関がさらに効く可能性がある。
minaseの1局は平均約366局面（第2世代の37本、1本7,500局からの計算）で、Saplingの25.5億局面（1,850万局）は1局あたり約138局面である。
minaseの実効の標本数は局面数の比より小さいと見込む（推論）。

教師の量が少ない先例で確認できた設定は次のとおりである。

| 先例 | 局面数 | 学習の設定 | 対局の記録 |
|---|---:|---|---|
| やねうら王の2018年の例 | 2億 | SGD、`eta` 1、λ=0.5、`newbob_decay` 0.5、`batchsize` 100万、乱数から（[記事](https://yaneuraou.yaneu.com/2018/12/30/nnue%E8%A9%95%E4%BE%A1%E9%96%A2%E6%95%B0%E3%81%AE%E5%AD%A6%E7%BF%92%E6%96%B9%E6%B3%95%E3%81%AB%E3%81%A4%E3%81%84%E3%81%A6/)） | 記事はelmo（WCSC27）よりR200程度弱いと書く。対局の条件は未確認 |
| Sapling 01 | 1,000万 | 1e-3、6 superbatchごとに×0.3、24 superbatch（[log.txt 19〜24行](https://github.com/Timmoth/Sapling/blob/2b43285d97fa4a9e49ce1d08fd4155a1fa950724/src/Sapling.Engine/Resources/WeightsHistory/log.txt#L19-L24)）。約240周（推論） | 世代の伸びは[調査](teacher-generation-strong-engines/zero-chess-and-variants.md)にある。予定の比較はない |
| Sapling 06 | 1.1億 | 1e-3、16 superbatchごとに×0.3、64 superbatch。約58周（推論） | 同上 |
| Fairy-Stockfishの手引き | 2億（1億以上を勧める） | 1.5e-3、×0.987/エポック、10エポック（[Training data generation 38行](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/Training-data-generation/668804196697bb157ae3f066382e60a8c46f0215)） | 手書きの評価関数に対する差は[一覧](https://github.com/fairy-stockfish/fairy-stockfish.github.io/blob/6c726ec48bbff68b09574082e432de78873c4bd1/nnue.markdown)にある（将棋系の例で`euroshogi` +601、`checkshogi` +700）。各ネットの学習の設定は未確認 |
| tanuki- 2021年 | 1億〜5億 | SGD、`eta` 1.0、`newbob_decay` 0.5、1億局面ごとに判定、`loop` 100、λ=0.5、既存のネットから（[記事](https://nodchip.hatenablog.com/entry/2021/11/25/000000)） | 基準に対してR−43.5（1億）からR−22.1（5億）。[2022年](https://nodchip.hatenablog.com/entry/2022/04/10/000000)に40億局面で頭打ちと結論 |
| Viridithas `compact` | 50億、80億 | 短い学習と余弦。60、82、160 superbatch（[514〜548行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L514-L548)） | 基準に対して−107.30（60、50億）、−43.56（60、80億）、−26.55（82、80億）、−8.47（160、80億） |
| Viridithas viri52 | 4.5億 | 未確認 | −16.40 ±9.40。作者は「The weakness of smaller datasets is apparent.」と書いた（[154〜157行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L154-L157)） |
| Zevra 2 | 約15億 | 非公開の学習器で確認できない（[README 100〜104行](https://github.com/sovaz1997/Zevra2/blob/cf5e8a5dbc2a1dcc5887fd1bfa9ea86387988021/README.md#L100-L104)） | 別の調査に世代の記録がある |

これらを合わせると、1億局面の規模で周回数を大きく取る設定に実例があり（Sapling）、1周で止める設定にも手引きがある（Fairy-Stockfish）。
両者とも学習率の予定を対局で比べた記録はないので、先例からどちらが良いかは決められない（推論）。
minaseの1局あたりの局面数が多いことは周回数を増やす側に不利に働き得るが、測られていない（推論）。

## 対局で測られた設定と慣習にすぎない設定

### 対局で測られた設定

次の表の比較はすべて対局の結果である。
表の「交絡」は、学習率や学習量の効果と分離できない同時の変更を指す。

| 設定 | 比較 | 結果 | 交絡 | 出典 |
|---|---|---|---|---|
| 最適化手法AdamW（Rangerの派生から） | Stockfish、同じ構成の比較 | STC非劣性の不合格（−0.63 Elo）、ノード数で時間を換算したSTCの非劣性は合格（+3.64）、VLTC +1.00、VVLTC +1.69で採用 | ネットの疎性（値が0になる活性の割合）が下がって少し遅くなる。作者は原因を不明と書いている。予定の調整も同時である | [f21610e5](https://github.com/official-stockfish/Stockfish/commit/f21610e51a436b69fabdcc89d8fe25ff0c2d5731)、[PR #448](https://github.com/vondele/nettest/pull/448) |
| 余弦の1周期への切り替え（×0.995/エポックから） | Stockfish | STC +3.88、LTC +3.32 Elo | 作者は学習器の変更が多く、λの予定をほぼ使わなくなり、損失の重みの引数を誤って落としたと書く。予定の形だけの効果は分離されていない | [dd321af5](https://github.com/official-stockfish/Stockfish/commit/dd321af5dfc0789de07c4e5c64915073995eb818)、[PR #329](https://github.com/vondele/nettest/pull/329) |
| 活性化と重みの疑似量子化 | Stockfish | STC +4.38、LTC +1.76 Elo | 量子化の尺度の方式も同時に変えた | [313ea4ab](https://github.com/official-stockfish/Stockfish/commit/313ea4ab0410872e99f0668c74edc7e49e9a0b6a)、[PR #346](https://github.com/vondele/nettest/pull/346) |
| 学習の延長（約20%） | Stockfish | STC +1.61、LTC +1.54 Elo。担当者は「1 ± 1 elo for +20% longer training」と書く | 1周期とλの予定の長さを同時に伸ばした（[差分](https://github.com/vondele/nettest/commit/9bc8d4f4f89b9126079176dfdcd901d7bf294f7d)） | [bc944563](https://github.com/official-stockfish/Stockfish/commit/bc944563790bf0b46a7acd8d9de139315d1b974f)、[PR #462](https://github.com/vondele/nettest/pull/462) |
| 微調整段の学習率を10分の1にして延長 | Stockfish | STC +0.64、LTC +1.83 Elo | 担当者は学習率と同時に入れた変更の寄与を分けられないと書き、レシピへ統合していない | [2d54ef33](https://github.com/official-stockfish/Stockfish/commit/2d54ef335ac40832140cfa079d2d814b2bfb7916)、[PR #488](https://github.com/vondele/nettest/pull/488) |
| 既定の10分の1の学習率で追加学習（×0.992/superbatch、400 superbatch） | tanuki-、追加学習の前後 | STC 14,616勝14,198敗1,196分（再計算+4.84）、LTC 5,908勝5,607敗1,005分（+8.35）で採用 | 著者は「学習率だけの効果を候補同士で直接測定したわけではなく、最適な値を特定した結果でもありません」と書く | [記事](https://nodchip.hatenablog.com/entry/goal-20260909-171439-3292c923) |
| さらに半分の学習率で追加学習 | tanuki-、追加学習の前後 | 10,867勝10,931敗1,054分（−0.97）、STCで不採用 | 同上 | [記事](https://nodchip.hatenablog.com/entry/goal-20260910-022356-3f339663) |
| 既定の80分の1の学習率で追加学習 | tanuki-、追加学習の前後 | 6,510勝6,623敗569分（−2.87）、STCで不採用 | データも異なる | [記事](https://nodchip.hatenablog.com/entry/goal-20260917-104914-10bb7c44) |
| 新規学習の学習率を4分の1 | tanuki-、同じ構成 | −5.73 Elo（95%区間−12.96〜+1.51）、判定不能。検証損失は0.008911から0.008669へ下がった | 初期重みの一致は未確認で各1回 | [記事](https://nodchip.hatenablog.com/entry/20261023-012541-2e82b3e3-459a-40c0-9427-5a207fccd7b3) |
| ReduceLROnPlateau（バッチごとの損失で判定） | tanuki-、基準のネット | R−30.6 ±9.7 | 重みのクリップの方式も違う。著者は「高い学習率で十分な学習が行えず、学習が不十分だったためだと思う」と書く | [記事](https://nodchip.hatenablog.com/entry/2023/12/04/000000) |
| SGD+Momentum（学習率0.5） | tanuki-、Ranger21の基準 | −188.75 ±38.45 Elo | 予定と教師も違う | [記事](https://nodchip.hatenablog.com/entry/2026/01/24/000000) |
| 学習率を一定（`--newbob-decay 1.0`） | 公開実験、水匠5に対して | −437 ±15 nElo。同じ教師と学習量のtatara公開実験は同じ相手に固定ノードで−83 ±15 nElo | 記録のlrは8.75e-4だが、コマンドに`--lr`がなく、当該コミットの既定は1.0のSGDである（[train.py 59行](https://github.com/nodchip/nnue-pytorch/blob/395cfbb37ad0e365ffcbe12dac08e5d173fe1727/train.py#L59)、[model.py 227行](https://github.com/nodchip/nnue-pytorch/blob/395cfbb37ad0e365ffcbe12dac08e5d173fe1727/model.py#L227)）。ノード数も300kと500kで違う | [実験](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTJDDQL7B52A6C6AF340522C) |
| 学習器の差（AdamW対Ranger） | 公開実験、固定ノード | BulletOuがtataraに−150 ±15 nElo | 損失、因子化の掛け方、および尺度も違う | [実験](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTJ1J6LRC9DEE09F3A48614B) |
| 初期学習率を10倍 | Viridithas、固定ノード | −180.47 ±27.29 Elo | なし（単独の変更と記載） | [411〜416行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L411-L416) |
| 10エポックの指数減衰 | Viridithas、STC | −15.84 ±7.13 Elo | 基準の学習の長さは未確認 | [256〜261行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L256-L261) |
| 別のエンジンの予定を移植 | Viridithas、STC | −7.27 ±2.55 Elo | なし | [1461〜1467行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1461-L1467) |
| 終わりの学習率を上げる | Viridithas、STC | −8.96 ±2.61 Elo | クリップも変えた | [1469〜1475行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1469-L1475) |
| 同じ学習の後のエポック | Viridithas `cyc`（4エポックごとに×0.3、15エポック） | エポック5は基準に−9.60、エポック11は基準に+10.80、エポック15はエポック11に+5.33（いずれもSTC） | なし | [334〜358行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L334-L358) |
| 学習率を半分にした追加の200 superbatch | Viridithas、固定ノード | +6.44 ±3.66 Elo | なし | [694〜700行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L694-L700) |
| 100 superbatchの追加と余弦 | Viridithas、固定ノード | +2.42 ±1.89 Elo | 数億局面の新しいデータも同時 | [622〜628行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L622-L628) |
| 第1層の初期値をfan-in | tatara、LayerStack | +31.17 ±13.79 nElo（1,220ペア） | なし | [PR #363](https://github.com/SH11235/tatara/pull/363) |
| 因子化を有効 | tatara | +8 ±10 nElo。40 superbatchの時点は+29 Elo | なし | [PR #303](https://github.com/SH11235/tatara/pull/303) |
| 重複の除去、静止探索の末端への置換 | tatara公開実験、固定ノード | HalfKPで+29 ±15、+111 ±15 nElo、幅1536で−2 ±15、−5 ±15 nElo | なし | [リリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607) |
| 同じデータの予定の比較（cosineと一定） | minase、STC | 281ペアで`H1`（候補の得点は[記録](../measurements/nnue-lr-schedule/nnue-lr-schedule-cosine-stc.md)のペンタノミアル度数[42, 2, 138, 3, 96]） | 幅128、先読みの教師の時代の比較である。現行の幅256、出力部、および教師では測っていない | [ADR](../adr/archive/anneal-the-nnue-learning-rate-with-a-cosine-schedule.md) |

### 慣習にすぎない設定

次の設定には、調べた範囲で値そのものを対局で比べた記録がない。

| 設定 | 由来 | 出典 |
|---|---|---|
| tataraの×0.992/superbatch | nnue-pytorchに揃えた。PRのテスト計画はSPRTによる検証を対象外とする。±10〜50 Eloという見積もりも経験則と書かれている | [PR #275](https://github.com/SH11235/tatara/pull/275) |
| 初期学習率8.75e-4 | 既定のままでよいと手引きに書かれている。比較の記録は見つからない | [quickstart 181〜182行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/docs/training-quickstart.ja.md?plain=1#L181-L182) |
| 16,384×6,104（約1億局面） | nnue-pytorchのエポックの規模に揃えた | [quickstart 184〜188行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/docs/training-quickstart.ja.md?plain=1#L184-L188) |
| tataraのRanger | 標準設定と書かれている。tataraの中でAdamWと比べた記録はない | [optimizer.rs 96〜99行](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/crates/nnue-train/src/optimizer.rs#L96-L99) |
| 160 superbatch（2周） | 公開実験の揃えた条件であり、記事は細部で最善のレシピではないと書く | [記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187) |
| variant-nnue-pytorchの1.5e-3、×0.987、出力層の10分の1 | コードのコメントは変更後の値と合っていない（「Drop learning rate after 75 epochs」） | [model.py 298〜312行](https://github.com/fairy-stockfish/variant-nnue-pytorch/blob/0536db20871d5d849402e1541c8a29722dc5a69d/model.py#L298-L312) |
| 2億局面に10エポックの目安 | データを1周する長さとして説明されている | [NNUE training 57行](https://github.com/fairy-stockfish/variant-nnue-pytorch/wiki/NNUE-training/668804196697bb157ae3f066382e60a8c46f0215) |
| bullet simpleの1e-3、×0.1/18 superbatch | 例の冒頭は「the training schedule is pretty sensible」と書くだけで、予定の調整に余地があると続ける | [simple.rs 4〜6行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/simple.rs#L4-L6) |
| bullet advancedの3段 | 配布例であり、特定のエンジンの採用設定ではない | [main.rs 195〜219行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/advanced/main.rs#L195-L219) |
| BulletOuの下限1e-5 | CLIの既定値。公開実験は2.4e-4を指定した | [4943行](https://github.com/yaneurao/BulletOu/blob/bc0a32348b06efa9cda67510322f5ae021f838f7/examples/bulletou.rs#L4943-L4944) |
| Stockfishの1周期の係数（warmup 5%、終端の除数1,000） | 導入は上の表の複合の変更に含まれ、係数を他の値と比べた採用の記録は見つからない | [threats.yaml](https://github.com/vondele/nettest/blob/d0bacd0a2ac2cf8d37ed0839a6720bca53968c1f/threats.yaml#L91-L92) |
| やねうら王の`eta` 1.0、`newbob_decay` 0.5 | 記事は推奨値の理由を十分に書いていない。`eta`は「発散する」値を避ける目安である | [記事](https://yaneuraou.yaneu.com/2018/12/30/nnue%E8%A9%95%E4%BE%A1%E9%96%A2%E6%95%B0%E3%81%AE%E5%AD%A6%E7%BF%92%E6%96%B9%E6%B3%95%E3%81%AB%E3%81%A4%E3%81%84%E3%81%A6/) |
| minaseの余弦の総長3周（T=3） | [設計書](../plans/archived/nnue-lr-schedule.md)の探索の範囲は事前に固定したものであり、最適性は確かめていない。T=3は当時の早期終了より前に下限へ届くように選んだ | [設計判断](../plans/archived/nnue-lr-schedule.md) |
| minaseの下限×0.1（f=0.1） | 一定の倍率の最小値に揃えた | 同上 |
| minaseの重み減衰1e-4 | 対局で比べた記録は見つからない | [options.rs](../../crates/minase-trainer/src/nnue/options.rs) |

### 検証損失が初期に最良になる現象についての記述

minaseの症状を直接論じた先行実装の開発者の記述は見つからなかった。
近い記述は次の4つの資料にある。

やねうら王の作者は、教師の少ない学習の報告に対して「eta 1.0が大きすぎ＆教師データが少ないので過学習になっているのでは…。」と答え（2019年11月14日）、別の報告には「500万局面が少なすぎて、たまたまlossが上がったところで判定されてrejectされてるだけのような…。」と答えた（2019年11月18日）。
同じ教師を複数回学ぶことについては「教師局面が少ないならやったほうがいいと思いますけど、あまり回し続けても過学習しますし、ほどほどに…。」と書いた（2019年12月2日）。
いずれも[2018年の記事](https://yaneuraou.yaneu.com/2018/12/30/nnue%E8%A9%95%E4%BE%A1%E9%96%A2%E6%95%B0%E3%81%AE%E5%AD%A6%E7%BF%92%E6%96%B9%E6%B3%95%E3%81%AB%E3%81%A4%E3%81%84%E3%81%A6/)のコメント欄である。

[nnue-pytorchのwiki](https://github.com/official-stockfish/nnue-pytorch/wiki/Basic-training-procedure-(train.py)/0e18032a596878725bd4d4772c24283bb605a4e2)は「Validation loss helps compare runs with the same data and loss settings; engine matches are needed to assess playing strength.」と書き、学習の揺らぎが関わる比較には複数のシードを使うよう勧めている。

nodchip氏は、ロスの低下が対局での強化を意味するわけではないと繰り返し書いている（[2026年10月19日](https://nodchip.hatenablog.com/entry/20261019-125745-44ba217c-0ac8-44d9-b248-905245c76bcc)、[2026年10月23日](https://nodchip.hatenablog.com/entry/20261023-012541-2e82b3e3-459a-40c0-9427-5a207fccd7b3)）。

tataraの作者は、公開実験の結果について「少なくとも本実験の範囲では accuracy の高低が棋力と一致していない」と書いた（[リリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)）。

対照として、tataraの公開実験の検証損失の最良はHalfKPで[157 superbatch](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTGQISAM5AD660DC041F20F7)、SFNN-1536で[159 superbatch](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTJ1J6LP022C3207CAC821EB)、bullet-shogiで[153 superbatch](https://nnue-lab.sh11235.com/t/sh11235/experiments/01JTGAVEA12B1E035D6A1BD06A)であり、いずれも学習の終わりの近くである。
これは同じ2周でも、外部の棋譜を検証に使い、更新回数がminaseの80倍以上ある条件での結果である。
minaseの過去の記録には、λ<1で[対局の勝敗を覚える](../lessons/game-constant-target-memorization.md)という別の機構も報告されているが、第2世代はλ=1.0なのでこの機構は当たらない。

## minaseの現行の設定との差分

| 項目 | minase | 先行の値 | 差の読み方 |
|---|---|---|---|
| 最適化手法 | AdamW | tataraとtanuki-はRanger、Stockfishは2026年8月からAdamW、bulletはAdamW | AdamWはStockfishの本番で実績がある。tataraの中の比較はない |
| 初期学習率 | 1e-3 | 8.75e-4、4e-4（バッチ131,072）、1.5e-3、1e-3 | 範囲の中にある |
| 学習率の群ごとの差 | 線形の経路だけ30倍の値 | nnue-pytorchは全群同じ、variantは出力層が10分の1 | 線形の経路は単位が違うので比べられない。線形の経路と非線形部分で別の大きさの学習率を与える先例は見つからない |
| 倍率の形 | 更新ごとの余弦 | Stockfishは更新ごとの1周期、tataraはsuperbatchごとの階段、BulletOuは自動 | 形の差の効果を測った記録はない |
| 半分までの周回数 | 1.61周 | tatara公開実験とtanuki-は1.08周、variantは届かない、Sapling 1.1億局面は14.5周 | 周回数ではtataraより緩い。局面の少ない先例は割れる |
| 半分までの更新回数 | 9,458 | 52.7万、84.4万、268万、9.8万（Sapling 1.1億局面） | 桁が違う。1億局面では全てを再現できない |
| 終わりの倍率 | ×0.1 | ×0.277、×0.040、×0.0016、×0.00004、×0.027 | 公開実験を除けば浅い |
| warmup | なし | tataraはなし、Stockfishは5%、tanuki- 2025年は1万バッチ | Viridithasの1,600バッチは読み飛ばしと同時の測定のみ |
| 総周回数 | 3周で下限、7周で停止 | tatara 2周、tanuki- 5周、Sapling 23.5〜240周、variant 1周 | 範囲の中にある |
| 総更新回数 | 約4万 | 1.2万（variant）、39万（Sapling 1.1億局面）、98万、244万、524万 | variantの次に少ない |
| バッチ | 16,384 | 16,384が多数。Stockfishの本番は131,072 | Stockfishの大きなバッチは処理速度のためと読める（推論） |
| 重み減衰 | 1e-4 | 0が多数。Stockfishは因子化の層だけ4e-5、bulletは0.01 | 第1層に減衰を掛ける先例はbulletの既定だけである |
| 勾配クリップ | なし | Stockfishはノルム2.0、tataraはなし | 効果の記録はない |
| 量子化を考慮した学習 | なし | Stockfishは既定で有効、tataraは任意 | StockfishにSTC +4.38、LTC +1.76 Eloの採用の記録があるが、量子化方式の変更と同時である。tataraの任意の機能は未測定である |
| λ | 1.0、学習後に線形の経路を補正 | 0.667、1.0→0.7→1.0、1.0など | [設計書](../plans/archived/nnue-generation-reset.md)の2026年11月5日の決定による。線形の経路を学習後に補正する先例は[tatara調査](tatara-nnue-transfer.md)でも見つかっていない |
| 損失 | 二値交差エントロピー | 差の2〜2.75乗 | [tatara調査](tatara-nnue-transfer.md)が扱う |
| 局面の間引き | 生成時だけ | Stockfishは1/4に間引き、variantも既定で1/4、tatara公開実験はなし | 周をまたいで別の局面を使う操作はminaseにない |
| 検証 | 対局単位の分割 | 局面単位か外部の棋譜 | minaseのほうが厳密である |
| 書き出すネット | 検証損失の最良 | 最後のネット、または対局で選ぶ | 標準とするのは調べた範囲でminaseだけである。やねうら王の旧学習器とBulletOuの`plateau`は採否を判定しながら学習率を下げる |
| 早期終了 | 改善なし5周 | tataraとStockfishにはない。旧学習器とnodchip氏の派生は学習率を下げてから止める | 学習率を下げる予定と組み合わせると、総長に届く前に止まる場合がある |
| 継続学習 | なし | 多くは既存のネットからの追加学習を併用 | 世代のやり直しの方針による |
| 第1層の初期値 | fan-inと同じ分散 | tatara、nnue-pytorch | 揃っている |

## minaseの標準学習レシピの候補

本節は開発者が決めるための候補であり、決定ではない。
候補の根拠はすべて前節までの資料であり、中将棋の教師で確かめたものではない（推論）。

### このまま採れる項目

次の項目は、先行実装と揃っているか、変える根拠が見つからない。

- バッチを16,384局面とする。tatara、variant-nnue-pytorch、およびnnue-pytorchの既定と同じであり、Stockfishの131,072局面は処理速度のための変更と読める（推論）。
- AdamWのβ1=0.9、β2=0.999、ε=1e-8とする。Stockfishの本番と同じ値である。
- 倍率を更新ごとに連続で変える。Stockfishの本番と同じ方式であり、階段にする根拠がない。
- 第1層の初期値をfan-in則と同じ分散とする。tataraの+31.17 ±13.79 nEloの記録があり、minaseも[採用済み](../measurements/nnue-architecture-revision/nnue-architecture-revision-init-training.md)である。
- 層ごとの重みの切り詰めを保つ。調べたすべての学習器が持つ。
- 検証を対局単位で分ける。先行実装より厳密である。
- 採否を段階ゲートで決め、検証損失を学習器の不具合と収束の点検に使う。tataraの[手引き](https://github.com/SH11235/tatara/blob/87f19551ceeffa072b8274d5bec88981ab33ffe1/docs/held-out-validation.ja.md?plain=1#L5-L9)とnnue-pytorchのwikiの考え方に一致する。
- warmupを入れない。不安定さは観測されておらず、単独の効果の記録もない。
- 学習の記録に周回数と更新回数と学習率の総和を書く。エポックの語の違いによる取り違えを防げる。

### 対局で測ってから決める項目

次の比較はいずれも同じ教師、初期値、およびシードで学び、`fit-linear`の後のネットをSTCのGSPRTで比べる案である。
費用が小さい順に並べる。
1周は作業機のRTX 3060 Tiで約3分（1更新約30 ms、[記録](../measurements/nnue-generation-reset/nnue-generation-reset-chain-trial.md)）、生成機のRTX 5090で1分未満（1更新3.2 ms）である。

| 項目 | 比較の案 | 根拠 | 読み方 |
|---|---|---|---|
| 予定の総長 | `cosine:3:0.1`に対して、`cosine:10:0.1`を`--epochs 10 --patience 50`で学ぶ。勝てば`cosine:25:0.03`を`--epochs 25 --patience 50`で学ぶ | tanuki-のPlateauとViridithasの`compact`と`cyc`は短い学習の不利を示す。Saplingは1.1億局面で約58周、×0.027である | 長い予定で検証損失が下がらないのに対局で勝てば、検証損失による選択を見直す材料になる |
| 下限 | 同じ総長で`cosine:10:0.1`と`cosine:10:0.01`を比べる | 先行はほとんど×0.1より深い。ただし第2世代の×0.1の区間で検証損失は改善していない | 差が出なければ下限は0.1のままでよい |
| 書き出すネット | 同じ学習の、検証損失が最良の周のネットと最後のネットを比べる。第2世代の学び直しなら2周と`--epochs 3 --patience 50`の3周 | 検証損失で選んで書き出す現行の先例が見つからない。第2世代の2周と3周の検証損失の差は0.000003である | 差がなければ規則を簡単にできる。最後のネットを書き出す手段の確認が要る |
| 重み減衰 | 第1層と出力部の重み減衰を1e-4と0で比べる | tatara公開実験、Stockfish（因子化の層以外）、およびSaplingの前提は0か未確認である。minaseの値の根拠は記録されていない | 予定を決めた後に測る |
| 基準の学習率 | 第1層と出力部を1e-3と5e-4で比べる。総長は上の結論に揃える | tanuki-の4分の1は判定不能、Viridithasの10倍は大敗。Stockfishの4e-4はバッチが大きい | 予定の総長と交絡するので順序を後にする |
| 追加学習の段 | 学んだネットに、`constant:0.1`を5周足した候補と比べる | tanuki-とStockfishとViridithasに採用の記録がある。同じ予定の下限の区間と重なる | 再開時に予定を変えられないので、学習器に重みだけを読む引数が要る。下限の結果を見てから決める |
| 局面の間引き | 周ごとに抽選する読み飛ばしを実装してから、4局面に1局面の予定を同じ更新回数で比べる。固定の部分集合での予備的な比較は`--train-subsample positions:4:<r>`でできる | StockfishとFairy-Stockfishは既定で使う。minaseには同じ局面数なら多くの対局から疎に採るほうが検証損失が低い記録がある（[記録](../measurements/nnue-early-best-epoch/nnue-train-subsample.md)） | [記録の間引きの設計書](../plans/sparse-teacher-recording.md)と重なるので、どちらで扱うかを先に決める |
| 損失の形 | 二値交差エントロピーと、sigmoid後の差の2乗を比べる | [tatara調査](tatara-nnue-transfer.md)の第5段階を参照 | 学習器の改修が要る |
| 最適化手法 | Rangerと比べる | 同じ調査の第5段階を参照。StockfishはRanger系からAdamWへ移り、tataraとBulletOuの差は交絡している | 実装の費用が大きいので後に置く |
| 量子化を考慮した学習 | 実数の順伝播へ丸めを入れて勾配を素通しする形で比べる | Stockfishは既定で有効で、導入時の採用の記録がある（[313ea4ab](https://github.com/official-stockfish/Stockfish/commit/313ea4ab0410872e99f0668c74edc7e49e9a0b6a)）。隠れ層を持つ現行の構成では対象がある（[tatara調査](tatara-nnue-transfer.md)の修正の節） | 学習器の改修が要るので予定の後に置く。量子化方式の変更と同時の記録なので、中将棋の構成での利得は分からない |
| 勾配クリップ | ノルム2.0を入れて比べる | Stockfishは固定で使うが効果の記録はない | 不安定さが観測されるまで測らない案もある |

暫定の標準として現行の`cosine:3:0.1`を残すか、`cosine:10:0.1`へ変えるかは、対局の結果が出るまで決められない。
どちらの結果でも、書き出しの規則と`--patience`の組み合わせは別に確かめる価値がある（推論）。

### minaseには当てはまらない項目

- 0.992という値は、1億局面単位の階段の値なので当てはまらない。minaseの1周に換算すると半減に86周かかり、前の[設計書](../plans/archived/nnue-lr-schedule.md)が見送った理由のとおりである。
- 固定局面数のエポックは当てはまらない。データが数千億局面ある学習の都合であり、minaseは全局面を毎周並べ替えるので周回数がそのまま読める。
- BulletOuのエポックごとに学習率を戻す形は当てはまらない。第2世代のように2周で頭打ちになる条件で学習率を戻す根拠が見つからない。
- λ<1とλの周期は扱わない。[設計書](../plans/archived/nnue-generation-reset.md)の2026年11月5日の決定により、第1世代のλ=0.75が第0世代に`H0`（得点率28.4%）だったことを受けて対象外とした。
- 入力の因子化はP型には意味を持たない（[tatara調査](tatara-nnue-transfer.md)）。HalfKP-hm-b8用の仮想行は実装済みである。
- Stockfishのlc0の評価による付け直しと、段ごとに別のデータを使う学習は、自分より強い評価器も別系統の教師データもないので当てはまらない（[調査](teacher-generation-strong-engines.md)）。
- 既存のネットから続けて学ぶ追加学習は、世代のやり直しの方針と衝突する。`--base`は探索用駒価値と尺度だけを読み、毎世代初期化する（[設計書](../plans/archived/nnue-generation-reset.md)）。
- Stockfishの本番の更新回数と総局面数は当てはまらない。同じ延べ局面数を1億局面で流すと約6,700周になる（推論）。

## 未確認の事項

- Stockfishの本番の一意な局面数は確認できなかった。公開文の1,000億超という記述と、`README`の約700 GB、wikiの800 GB超という容量だけがある。したがって周回数は未確定とした。
- 微調整段の学習率を10分の1にした学習で、再開時に1周期の学習率がどう適用されたかはコードで確かめていない。
- Sapling、Viridithas、およびBulletOuの公開実験以外の、各ネットのバッチと1 superbatchの更新回数は、既定値からの推論である。明示の記載があるのはSaplingの25.5億局面のネットだけである。
- Saplingの各ネットを乱数から学び直したか前のネットから続けたかは記載がない。これは[付録](teacher-generation-strong-engines/zero-chess-and-variants.md)と同じ状況である。
- Fairy-Stockfishの[一覧](https://github.com/fairy-stockfish/fairy-stockfish.github.io/blob/6c726ec48bbff68b09574082e432de78873c4bd1/nnue.markdown)の各ネットを作った局面数と設定は書かれていない。手引きの例と実際のネットが同じかは分からない。
- Zevra 2の学習器は非公開であり、学習率の予定は確認できなかった。
- BulletOu-minishogiは、手元の`cc202f3`のブランチとファイル名に小将棋に固有のものが見当たらなかった。小将棋のレシピは確認できなかった。
- nnue-pytorchの現行の第1層の重みの初期値の行は確かめていない。tataraのPRと[minaseの記録](../measurements/nnue-architecture-revision/nnue-architecture-revision-init-training.md)の記述に依る。
- tanuki-の2026年の新規学習のλは記事に書かれていない。
- やねうら王の旧学習器の`nn_batch_size`と更新回数の関係は、削除前の[evaluate_nnue_learner.cpp](https://github.com/yaneurao/YaneuraOu/blob/cc73ac44ad1433463df73fc6c97c06fd27c5d266/source/eval/nnue/evaluate_nnue_learner.cpp#L180-L186)の読み取りまでであり、実際の更新回数は記録がない。
- 検証損失がごく初期で最良になる現象について、Discordなどの非公開の場の議論は調べていない。

## ライセンス

本書は式、手順、および短い引用だけを扱う。
学習器へコードを写す場合の条件は[tatara調査](tatara-nnue-transfer.md)のライセンスの節に従う。

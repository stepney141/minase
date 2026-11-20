# 学習に使う局面の間引きに関する先行実装と論文の調査

本書のminaseに関する記述は2026年11月20日時点のものである。

本書は、将棋とチェスのエンジン、学習器、および論文が、対局の棋譜から学習の局面を採るときに局面を間引いているかを一次資料で調べた調査メモである。
調査の動機は、中将棋エンジンminaseの評価関数NNUE（差分更新可能なニューラルネットワーク）の教師データで、同じ対局の局面どうしの相関が強かったことにある。
教師データは、自己対局の局面に探索値と対局結果を付けた学習データであり、以下では単に教師と呼ぶ。
探索値を勝率に換算して対局内の平均を引いた値の相関係数は、1手離れた局面で0.989、32手離れた局面で0.816だった（[測定記録](../measurements/adhoc/tatara-p-generalization.md)）。
同じ記録では、訓練局面数と更新回数を揃えたとき、14,048局から疎に採ったモデルが3,425局から密に採ったモデルに対して+205.6 Eloだった。
本書は、この対策の候補である局面の間引きについて、先行例の方式、作者が述べる理由、および効果の測定値を集める。
調査日は2026年11月20日である。
出典のない値と設計上の含意には「推論」と付ける。

## 用語と調べ方

本書は、局面を減らす操作を、行う場所で4つに分ける。

- **生成時の間引き**は、教師を作る時点で1局の一部の局面だけを記録する操作である。1局から1局面または数局面だけを採る方式と、手ごとに抽選して深く探索した手だけを記録する方式を含む。
- **学習時の読み飛ばし**は、記録済みの局面を学習器が読み込むときに確率的に捨てる操作である。読むたびに抽選する実装では、訓練データをひととおり読む1周（1エポック）ごとに別の局面が使われる。
- **部分抽出**は、生成した全体から学習に使う部分集合を選ぶ操作である。リプレイバッファ（直近の自己対局のデータをためておく領域）からの抽出を含む。
- **重複除去とシャッフル**は、完全に同じ局面を除く操作と、対局をまたいで局面の順を混ぜる操作である。局面の数はほとんど減らないが、同じ対局の局面が1回の更新に固まることを防ぐ。

捕獲手、王手、序盤、および評価値の大きい局面を条件で除く操作は、相関への対策ではないので本書の対象から外し、必要な箇所で短く触れる。

調査は、将棋、チェスのNNUEとAlphaZero型、およびNNUE以前のチェスと論文の3つに分けて行い、ソースコードはコミットを固定したURLで、文章は作者本人の投稿、wiki、または論文で確かめた。
表の「再照合」の列に「済」とある項目は、調査の後に別の手順で原文を取得し直し、引用した字句と数値が原文にあることを確かめたものである。
「未」の項目は、調査の時点で原文を開いて確かめたが、取得し直していない。
既存の調査メモが扱った実装は、[学習レシピの比較](nnue-training-recipes.md)の「局面の間引き」の節と[AlphaZero型の学習の局面の選び方](alphazero-training-sample-selection.md)にあり、本書は新しく分かった事実と測定値を中心に書く。

## 結論

局面の間引きの先行例は、将棋、チェス、囲碁のいずれにもある。
同じ対局の局面の相関を理由に明記した例は、囲碁のAlphaGo、チェスのAlphaZeroを再学習したMcGrathらの論文、Leela Chess Zeroの旧い学習器、Texelの作者の投稿、Etherealの作者の論文、および将棋のやねうら王のwikiのページ「ふかうら王の学習」である。

将棋では、生成時に局面を無作為に間引く例が技巧にある。
技巧は、自己対局の各手を確率10%で記録する方式と、教師1件ごとに新しい手順を作って末端の1局面だけを探索する方式を持つ。
やねうら王、Apery、elmo、およびdlshogiは1局の通常の手をすべて記録し、相関には生成の後のシャッフルで対処する。

チェスのNNUEでは、Stockfishの学習器が学習時の読み飛ばしを本番で使い、現在は4局面に1局面を使う。
作者が述べた導入の理由は、1回のバッチに同じ対局の手を並べないことである。
読み飛ばしだけを変えて棋力を測った記録はStockfishには見つからなかった。
チェスエンジンのViridithasには、読み飛ばしの値を変えて−14.31 ±2.64 Eloになった記録と、全体をシャッフルして読み飛ばさない学習が読み飛ばす学習に対して+3.87 ±2.63 Eloだった記録がある。

間引きの効果を他の条件から切り分けて測った例は少ない。
対局数を増やす方向の効果を示した測定は、囲碁のKataGoの比較（手ごとに探索量を変えて深い探索の手だけを記録する方式を外すと1,329 Eloから1,242 Eloへ下がる）と、minase自身の測定である。
同じ対局の集合から局面を捨てるだけで強くなったことを示す測定は、調べた範囲で見つからなかった。
探索値を教師とするNNUEについて、少ないノード数の探索で指した手を記録せずに対局数を増やす方式の効果を測った例も見つからなかった。

## 将棋

表の技巧、やねうら王、tanuki-、Apery、elmo、dlshogi、rshogi、およびBonanzaは将棋エンジンである。
ふかうら王は深層学習のモデルを使うやねうら王の版、tataraはrshogiの作者によるNNUEの学習器、nnue-pytorchはStockfishの開発者によるNNUEの学習器である。
方策は指し手の確率分布の予測、価値は勝率の予測であり、一致率は検証用の棋譜の手または勝敗と予測が一致した割合である。
Rはレーティングの差、nEloは引き分けの多さによらず比べられるように正規化したEloである。

| 実装 | 分類 | 方式と値 | 作者の理由と効果の測定 | 再照合 |
|---|---|---|---|---|
| 技巧の勝敗の教師 | 生成時の間引き | 300万局の自己対局で、各手を確率10%で記録する。勝敗がついた対局だけを保存し、教師値は勝敗に置き換える（[teacher_data.cc 313〜347行](https://github.com/gikou-official/Gikou/blob/ca25edf02fe29f58d18890af604bd9cf7a455c01/src/teacher_data.cc#L313-L347)）。 | コメントは「一定の確率（現在は10%の確率）で、自己対戦中の局面をサンプリングする」とだけ書き、10%にした理由は書いていない。10%と全局面の比較はない。 | 済 |
| 技巧の探索値の教師 | 生成時の間引き | 教師1件ごとに、手数を乱数で決めた新しい手順を作り、末端の1局面だけを探索して保存する。件数は3,000万である（[62〜73行](https://github.com/gikou-official/Gikou/blob/ca25edf02fe29f58d18890af604bd9cf7a455c01/src/teacher_data.cc#L62-L73)、[171〜238行](https://github.com/gikou-official/Gikou/blob/ca25edf02fe29f58d18890af604bd9cf7a455c01/src/teacher_data.cc#L171-L238)）。 | 「実戦における出現可能性」と「局面の多様性」の両方を担保するためと書き、AlphaGoの論文を参考文献に挙げる。効果の測定はない。 | 済 |
| ふかうら王の学習 | 部分抽出 | 約2,600万パラメータのモデルで、3,000万局面を生成してシャッフルし、1,000万局面だけを抽出して学ぶ（[wiki「学習の基本その2」](https://github.com/yaneurao/YaneuraOu/wiki/ふかうら王の学習/87d9c8c22f4bddd75335f01f61b9fc0c4d511860)）。 | 「自己相関の強い局面が複数含まれている教師データを一度に学習すると、過学習気味になります」と書く。1,000万局面だけを生成するより正解率が良かったとするが、正解率の値と対局の結果は載っていない。 | 未 |
| やねうら王の旧`gensfen` | 重複除去とシャッフル | 間隔や確率で記録を減らす引数はない。16手未満を記録せず（`write_minply`）、既出の局面を約6,700万枠のハッシュ表で検出して、その対局でそれまでにためた局面ごと捨てる（[learner.cpp 655〜681行](https://github.com/yaneurao/YaneuraOu/blob/15c1cc6ae0ee87998af0251149dc6f1b175d95a3/source/learn/learner.cpp#L655-L681)）。 | 序盤を除く理由は「初期局面周辺はは類似局面ばかりなので学習に用いると過学習になりかねない」であり、続けて「比較実験すべき」と書く。測定はない。 | 済 |
| やねうら王のPython版`GenSfen` | 重複除去とシャッフル | 通常の手をすべて記録し、生成の後に別の道具でシャッフルする（[teacher/README.md 17〜32行](https://github.com/yaneurao/YaneuraOu-ScriptCollection/blob/3dc9ebb426c8584914150f5f4e12b96f7f7c96a6/teacher/README.md#L17-L32)）。 | 対局順のまま学習に流すとミニバッチごとの分布が偏ると説明する。測定はない。 | 未 |
| tanuki-（2025年） | 重複除去とシャッフル | 10億局面を8組生成し、シャッフルの工程で重複した局面を除く（[記事](https://nodchip.hatenablog.com/entry/2025/06/02/000000)）。 | 除かない場合との対局は5,000局で2,144勝758分2,098敗、R+3.2 ±9.6であり、有意な差はなかった。作者は「評価関数としての性質はあまり変わらない」と考察する。 | 済 |
| 将棋版のnnue-pytorch（nodchip） | 学習時の読み飛ばしは使われていない | `--random-fen-skipping`の既定は0である（[train.py 62〜63行](https://github.com/nodchip/nnue-pytorch/blob/39e504acec8518879e0d90da4126cb1b90c119cf/train.py#L62-L63)）。バッチの読み込み`fill`は局面をまとめて読み、読み飛ばしの判定を呼ばない（[nnue_training_data_stream.h 76〜143行](https://github.com/nodchip/nnue-pytorch/blob/39e504acec8518879e0d90da4126cb1b90c119cf/lib/nnue_training_data_stream.h#L76-L143)）。 | この版とこの経路では、0以外を渡しても読み飛ばしは働かない。 | 済 |
| Aperyとelmo | 間引きなし | 開始局面の集合から1つを選び、乱数の手を加えて対局を始め、以後の各手を記録して対局の単位で書き出す（[Apery usi.cpp 439〜499行](https://github.com/HiraokaTakuya/apery/blob/d14471fc879062bfabbd181eaa91e90c7cc28a71/src/usi.cpp#L439-L499)、[elmo usi.cpp 389〜446行](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp#L389-L446)）。elmoの手順書は、生成した教師をシャッフルしてから結合する（[README 58〜86行](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/README.md#L58-L86)）。 | 測定はない。 | 未 |
| dlshogi | 重複除去とシャッフル（任意） | 自己対局は通常の手をすべて記録する。学習器の`--use_average`は、同じ局面の方策、勝敗、および評価値を平均して1件にする。既定は無効である（[train.py 60行](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/1efd02ba94cad2749229c0f40f54b5bc41b484d5/dlshogi/train.py#L60)）。 | 重複が8.4%の教師500万局面では、平均化で価値の一致率が63.932%から64.498%へ上がり、方策の一致率は36.485%から36.266%へ下がった（[記事](https://tadaoyamaoka.hatenablog.com/entry/2021/04/17/182108)）。重複が0.14%の教師では傾向が見えなかった（[追試](https://tadaoyamaoka.hatenablog.com/entry/2021/05/09/181044)）。 | 未 |
| rshogiとtatara | 重複除去とシャッフル | rshogiの生成器は既出の局面を約6,700万枠の表で除く（[gensfen.rs 337〜345行](https://github.com/SH11235/rshogi/blob/e406dcb87b957a7898ff2ced4d5a37824466af38/crates/tools/src/bin/gensfen.rs#L337-L345)）。tataraの公開実験は学習時の読み飛ばしを使わない。 | 重複除去の対局での効果は、[tataraから移せる技法の調査](tatara-nnue-transfer.md)にある（HalfKPで+29 ±15 nElo、幅1536のLayerStackで−2 ±15 nElo）。 | 未 |
| Bonanza（2014年の論文） | 重複除去とシャッフル | 同じ局面と棋譜の手の組を、1回の反復で高々1回だけ使う（[Hoki and Kaneko, JAIR 2014](https://www.jair.org/index.php/jair/article/download/10871/25935/20279) 3.5.6節）。 | 序盤に同じ局面と手が重なることを理由に挙げる。測定はない。 | 未 |

将棋で重複除去について測られた値は、完全に同じ局面を除く効果であり、隣り合う別の局面を間引く効果ではない。
技巧の2つの方式は教師値が異なり、10%の記録は勝敗を、1局面の方式は探索値を教師にする。
将棋エンジンの水匠とKristallweizenが世界コンピュータ将棋選手権に出したアピール文書は、学習の前に教師をシャッフルすることを書くが、生成時に局面を間引いたとは書いていない（[水匠](https://www.apply.computer-shogi.org/wcsc29/appeal/Suishou/appeal.pdf)、[Kristallweizen](https://www.apply.computer-shogi.org/wcsc29/appeal/Kristallweizen/appeal2019_2.pdf)、再照合は未）。

## チェスのNNUEとAlphaZero型

表のnnue-pytorchとbulletはNNUEの学習器であり、McGrathらとZahavyらは論文である。
Leela Chess Zero（以下lc0）はAlphaZero型のチェスエンジンであり、ほかの行はNNUEを使うチェスエンジンとその教師生成のコマンドである。
表の「ネット」は、学習したニューラルネットを指す。

| 実装 | 分類 | 方式と値 | 作者の理由と効果の測定 | 再照合 |
|---|---|---|---|---|
| Stockfishのnnue-pytorch | 学習時の読み飛ばし | `random_fen_skipping`の値Nは、1局面を採るまでに平均で飛ばす局面の数であり、採る確率は1/(N+1)である。読むたびに抽選する。学習器の既定は0で、本番のレシピは2026年10月に10から3へ改めた（[学習レシピの比較](nnue-training-recipes.md)）。 | 導入したプルリクエストは「With very large binpacks allows for more diversity in a batch/epoch, not using all moves of a game in that batch」と書く（[#27](https://github.com/official-stockfish/nnue-pytorch/pull/27)、2020年12月）。作者は、シャッフルの変更と併せた試走で損失の曲線が「look better to the eye」と述べた。読み飛ばしだけを変えた対局の結果は見つからなかった。 | 済 |
| Viridithas | 学習時の読み飛ばしと全体シャッフルの比較 | ネット`fizz`は基準のネット`stalker`の読み飛ばしの値を0.75にした。`dagger`は`stalker`に、6局面に5局面の読み飛ばしと1,600バッチのウォームアップを加えた。`vega`は`dagger`と同じ条件で、全体をシャッフルした形式のデータを読み飛ばさずに学んだ（[networkhistory.txt 1381〜1419行](https://github.com/cosmobobak/viridithas/blob/7489e701bb07ff9a7223136bac7393f417aa588e/notes/networkhistory.txt#L1381-L1419)）。 | `fizz`は`stalker`に−14.31 ±2.64 Elo（20,022局）、`dagger`は`stalker`に+3.12 ±2.63 Elo（20,016局）、`vega`は`dagger`に+3.87 ±2.63 Elo（20,030局）だった。`stalker`の元の読み飛ばしの値はこの記録にない。`dagger`の差はウォームアップと分けられない。 | 済 |
| McGrathらによるAlphaZeroの再学習 | 部分抽出 | 直近100万局面のリプレイバッファから学び、1局から採る局面を平均で高々30にする（[arXiv:2111.09259](https://arxiv.org/pdf/2111.09259) 3.2節）。 | 「positions on subsequent moves are strongly correlated, and including all of them may lead to increased overfitting」と書く。30局面と全局面の比較はない。 | 済 |
| ZahavyらによるAlphaZeroの再学習 | 生成時の間引き | 標準の設定は、自己対局の局面の10%を無作為に選んでリプレイバッファへ入れ、残りを捨てる。速い設定は全局面を入れる（[arXiv:2308.09175](https://arxiv.org/pdf/2308.09175)）。 | 速い設定は弱かったが、同時にバッチの大きさなども変えているので、10%の効果は分からない。 | 済 |
| Leela Chess Zero | 学習時の読み飛ばし | 旧い学習器は各局面を確率1/32で採る（[学習の局面の選び方の調査](alphazero-training-sample-selection.md)）。2018年には、新しい局面1つあたり平均14.2回を学習に使っていた（[開発wiki](https://lczero.org/dev/wiki/large-elo-fluctuations-starting-from-id253/)）。 | 開発wikiは「we sample too many positions from the same game」と書き、強さの乱高下の最大の要因を過学習と結論する。読み飛ばしの率だけを変えた測定はない。 | 済 |
| Stockfishの`generate_training_data`とnodchip版の`gensfen` | 重複除去とシャッフル | 16手目から記録し、既出の局面を記録しない。間隔や確率で記録を減らす引数はない（[generate_training_data.md 45〜61行](https://github.com/official-stockfish/Stockfish/blob/9a4c7cf4e311f8d9526b79295b80c4d0464c07cf/docs/generate_training_data.md#L45-L61)）。 | 測定はない。 | 未 |
| Stockfishの`generate_training_data_nonpv` | 生成時の間引き（探索木から） | 探索の途中で評価した局面を既定で1%の確率で採り、後で浅い探索で値を付け直す（[仕様 15〜25行](https://github.com/official-stockfish/Stockfish/blob/9a4c7cf4e311f8d9526b79295b80c4d0464c07cf/docs/generate_training_data_nonpv.md#L15-L25)）。対局で指した手の100分の1を残す方式ではない。 | 測定はない。 | 未 |
| bullet | 学習時の読み飛ばし（呼び出す側が指定） | 読み込み器に渡すフィルタで指定する。既定の確率は0である。`examples/advanced`は確率0.15で局面を捨てる（[filter.rs 48〜76行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/examples/advanced/filter.rs#L48-L76)）。標準の前処理は、ファイルごとのシャッフルと複数ファイルの混ぜ合わせである（[3-data.md 3〜20行](https://github.com/jw1912/bullet/blob/5d70177680437b7c0ddf958bd4722d95afa78b0c/docs/3-data.md#L3-L20)）。 | 測定はない。 | 未 |
| Minic | 学習時の読み飛ばし | Stockfishと同じ式の読み飛ばしを持ち、作者の学習コマンドの例は10（11局面に1局面）を渡す（[Train/README.md 18〜24行](https://github.com/tryingsomestuff/Minic/blob/845b5267eb22e0e9bebf236dd56df480e097f00d/Train/README.md#L18-L24)）。 | 測定はない。 | 未 |
| Superpawn | 重複除去とシャッフル | 自己対局の局面を記録した後に全体をシャッフルする（[training.md 126〜141行](https://github.com/johnwbyrd/superpawn/blob/438def23b8ed233a3d36ace61cb38326725a0660/doc/training.md#L126-L141)）。 | 「Training data must be shuffled, since positions from one game are strongly correlated.」と書く。測定はない。 | 済 |
| Stormphrax、Monty、Halogen、Berserk | 間引きなし | 自己対局で探索した手をすべて記録する。条件による除外は持つが、間隔や確率で記録を減らす処理はない（[Stormphrax](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/src/datagen/datagen.cpp#L251-L300)、[Monty](https://github.com/jw1912/monty/blob/f8ea109330d914c392229408c6f4283462a2998e/datagen/src/thread.rs#L121-L160)）。 | 測定はない。 | 未 |

Stockfishの読み飛ばしは、対局の順に局面が並んだ巨大なファイル（binpack形式）を先頭から読む学習器で、1回のバッチに入る対局の種類を増やすための仕組みである。
読むたびに抽選するので、周を重ねれば全局面がいずれ使われ、教師の量を減らす操作ではない。
Viridithasの`fizz`の記録は、読み飛ばしの値が棋力を左右することを示すが、基準の`stalker`の値が記録にないので、読み飛ばしを増やしたのか減らしたのかは分からない。
`vega`の結果は、全体をシャッフルすれば読み飛ばしが要らない場合があることを示す。
手ごとに抽選して深く探索した手だけを記録する方式（次の節のKataGoのplayout cap randomization）は、調べた版のlc0の自己対局には見つからなかった。
変則チェスのcrazyhouseのエンジンCrazyAraの作者は、この方式を実装して短い期間だけ試したうえで、「both reduced the generation speed for training samples and likely increased the noise for the value target due to an increasing blunder rate」と書いて採らなかった（[Czechの修士論文](https://ml-research.github.io/papers/czech2019deep.pdf) 5.4節、再照合は済）。

## NNUE以前のチェスと論文

表のTexelからWinterまでの5つは、評価関数の係数を棋譜の局面と勝敗に合わせて調整したチェスエンジンであり、DeepChessから下は論文とその実装である。
`quiet-labeled.epd`はZurichessの作者が公開した局面の集合のファイルであり、静かな局面は駒の取り合いが続いていない局面を指す。

| 実装または論文 | 分類 | 方式と値 | 作者の理由と効果の測定 | 再照合 |
|---|---|---|---|---|
| Texel（Österlund、2014年） | 間引きなし | 64,000局の全局面（約880万）を使う（[作者の投稿](https://talkchess.com/viewtopic.php?p=555522#p555522)）。 | 誤差の和は「64000 independent events」の寄与であると認め、別々の880万局から1局面ずつ採るほうが「probably be better (or at least equally good)」だが、計算時間が足りないと説明する。比較の測定はない。 | 済 |
| Ethereal（Grant） | 生成時の間引き | 自己対局から局面を標本として採り、深く探索し直して主要変化の末端を保存する（[Tuning.pdf 2.2節](https://github.com/AndyGrant/Ethereal/blob/0e47e9b67f345c75eb965d9fb3e2493b6a11d09a/Tuning.pdf)）。1局あたりの数は書いていない。 | 「using a large number of positions from the same game has been shown to produce a lower quality dataset」と書く。値は載っていない。 | 済 |
| Blunder 8.0.0（Dean、2022年） | 生成時の間引き | 1局から無作為に最大10局面を採り、全体で重複を除く（[作者の投稿](https://talkchess.com/viewtopic.php?p=929286#p929286)）。 | 「Doing this seemed to help significantly with getting quality data and avoiding too many similar positions.」と書く。作者が報告する棋力の向上は再調整の全体の効果であり、上限だけの効果ではない。 | 済 |
| Zurichessの`quiet-labeled.epd` | 生成時の間引き | 75,000局のそれぞれで、対局中に探索が評価した局面から20局面を採る（[作者の投稿](https://talkchess.com/viewtopic.php?p=686204#p686204)）。棋譜の手から20手を採る方式ではない。 | 20にした理由は書いていない。測定はない。 | 未 |
| Winter | 生成時の間引き | 各棋譜から無作為に静かな局面を1つ採る（[data.cc 99〜128行](https://github.com/rosenthj/Winter/blob/0c46fa25d381f19fad062e04366f19c72255b9b0/src/data.cc#L99-L128)）。 | 理由は書いていない。測定はない。 | 未 |
| DeepChess（2016年） | 生成時の間引き | 1局から無作為に10局面を採る（[arXiv:1711.09667](https://arxiv.org/html/1711.09667v1#S3) 3節）。 | 上限の理由は書いていない。測定はない。 | 未 |
| Giraffe、KnightCap、Meep | 間引きなし | 連続する局面の探索値の差、または探索木の内部の局面まで学習に使う（[Giraffe](https://arxiv.org/pdf/1509.01549)、[KnightCap](https://arxiv.org/html/cs/9901002v1#S3)、[Meep](https://proceedings.neurips.cc/paper/2009/file/389bc7bb1e1c2a5e7e147703232a88f6-Paper.pdf)）。 | 局面を減らすのとは逆に、1回の探索から得る情報を増やす設計である。 | 未 |
| Ruossら（2024年） | 間引きなし | 1,000万局の全局面にStockfishの探索値を付け、付けた後にシャッフルする（[arXiv:2402.04494](https://arxiv.org/html/2402.04494v1#S2.SS1) 2.1節）。 | 探索値を教師にする学習で、全局面とシャッフルを選んだ例である。 | 未 |

囲碁の例は参考として次の2つを挙げる。

AlphaGo（2016年）は、価値ネットワークの学習で、同じ対局の局面の相関による過学習を報告した。
論文は「successive positions are strongly correlated, differing by just one stone, but the regression target is shared for the entire game」と書き、人間の棋譜の全局面で学ぶと平均二乗誤差が訓練0.19、試験0.37になったとする。
別々の対局から1局面ずつ採った3,000万局面に替えると、訓練0.226、試験0.234になった（[論文](https://storage.googleapis.com/deepmind-media/alphago/AlphaGoNaturePaper.pdf)、再照合は済）。
この改善は間引きだけの効果ではなく、教師の出所を人間の棋譜から自己対局へ替えた効果を含む。
1局から採る局面は1つだが、学習は32局面のバッチを5,000万回行うので、1局面は繰り返し使われる。

KataGoは、手ごとに抽選して約25%の手だけを多い探索回数で読み、その手だけを記録する（playout cap randomization）。
論文は理由を、価値の教師が「only one noisy binary result per entire game」しかなく、同じ計算で対局を増やすほうが価値の学習に有利だからと説明する。
この仕組みを外した学習は、同じ計算量で1,329 Eloから1,242 Eloへ下がり、同じ強さに達するまでの計算が1.37倍になった（[論文の表2](https://arxiv.org/html/1902.10565v5#S5.T2)、再照合は済）。

## 測定された効果のまとめ

間引きの効果を対局または損失で測った記録を次の表にまとめる。
ほかの変更と同時に測った記録は、分けられない変更を「同時に変えたもの」の列に書く。

| 例 | 比べたもの | 結果 | 同時に変えたもの |
|---|---|---|---|
| minase | 同じ局面数と更新回数で、3,425局から密に採る学習と14,048局から疎に採る学習 | 疎に採る側が+205.6 Elo［+157.0, +262.9］ | なし |
| KataGo | 深い探索の手だけを記録する方式の有無 | 方式を外すと1,329 Eloから1,242 Elo | 対局数、探索量の配分 |
| AlphaGo | 人間の棋譜の全局面と、自己対局の1局1局面 | 訓練と試験の誤差の差が0.18から0.008 | 教師の出所 |
| ふかうら王 | 1,000万局面の生成と、3,000万局面からの1,000万局面の抽出 | 抽出した側の正解率が良い（値はない） | なし |
| Viridithas | 読み飛ばしの値を0.75にした学習と基準 | −14.31 ±2.64 Elo | なし（基準の値が不明で、増減の向きは分からない） |
| Viridithas | 全体シャッフルで読み飛ばさない学習と、読み飛ばす学習 | +3.87 ±2.63 Elo | データの形式 |
| tanuki- | 重複した局面の除去の有無 | R+3.2 ±9.6で有意な差なし | なし |
| dlshogi | 同じ局面の平均化の有無（重複8.4%） | 価値の一致率が63.932%から64.498% | なし |

## minaseへの含意（推論）

先行例が支持するのは、同じ計算量で独立な対局を増やすことであり、同じ対局の集合から局面を捨てることではない。
KataGo、AlphaGo、Texelの作者の説明、およびminaseの測定は、どれも独立な対局の数が教師の価値を決めるという見方に立つ。
記録済みの局面を捨てるだけの操作で強くなった記録は見つからず、Viridithasでは全体をシャッフルして読み飛ばさない学習が読み飛ばす学習を上回った。
minaseの学習の標準の条件（標準レシピv3）が、全局面を学べる場合に学習器の引数`--train-subsample`で間引かないと定めていることは、この範囲の先行例と矛盾しない。

Stockfishの学習時の読み飛ばしの理由は、minaseではすでに満たされている。
Stockfishは対局の順に並んだファイルを順に読むので、読み飛ばしでバッチの中の対局の種類を増やす。
minaseの学習器は周ごとに訓練集合の全体を並べ替えるので（[設計判断の記録](../adr/shuffle-the-whole-training-set-per-epoch-with-rao-sandelius-buckets.md)）、1回のバッチに同じ対局の局面が固まらない。

[記録する局面の間引きの設計書](../plans/sparse-teacher-recording.md)の方式は、KataGoと技巧に先行例があるが、探索値を教師とするNNUEでの効果は誰も測っていない。
AlphaGoとKataGoが問題にしたのは、1局の全局面が同じ対局結果を教師として共有することである。
minaseの標準の教師は局面ごとの探索値だけであり（教師値のうち探索値が占める割合λが1.0）、教師値そのものは局面ごとに異なるので、この理由はそのままでは当てはまらない。
minaseの測定では、モデルの予測と教師の探索値との誤差の相関係数は1手離れた局面で0.678、32手離れた局面で0.030であり、探索値の相関（0.989と0.816）より速く減る。
したがって、対局数を増やす利益が探索値の教師でどれだけ残るかは、[先読みなしの教師での間引きの比較](../plans/nnue-subsample-no-lookahead.md)とこの設計書の測定で確かめる必要がある。
CrazyAraが同じ方式を試して採らなかった例は、少ないノード数の探索で指す手の質が教師の質を下げうることを示す。

## 確かめられなかった事項

Stockfishの`random_fen_skipping`だけを変えた対局の結果は見つからなかった。
本番のレシピが値を10から3へ改めた変更は、学習器の版の更新と同時である。

AlphaZero（2018年）の本番の学習で1局から採った局面の数は確かめられなかった。
Scienceの補足資料の擬似コードは出版社のサイトが403を返して取得できず、McGrathらの平均30局面とZahavyらの10%は、どちらも後年の別の実験の値である。

技巧の10%の記録と全局面の記録の比較、ふかうら王の抽出による正解率の値、およびZurichess、Winter、DeepChessが1局あたりの局面数を決めた理由は、資料に書かれていなかった。

NineDayFeverとelmoが世界コンピュータ将棋選手権に出したアピール文書、GPS将棋の2008年の論文、およびBonanzaの作者である保木の2006年の論文は、本文を取得できなかったか、文字を抽出できなかった。
チェスエンジンのEthereal、Koivisto、Seer、PlentyChess、およびRecklessの本番の教師で1局から採る局面の数は、公開の資料に見つからなかった。

# 教師生成で王手の局面の除外、開始局面の選別、および大差の対局の打ち切りを行う理由の先行調査

本書は、チェスと将棋のエンジンが教師データの生成と学習で行う3つの処理を一次資料で調べた調査メモである。
対象は、王手を受けている局面を教師から除く処理、ランダムな手で作った開始局面のうち探索値が大きく傾いたものを捨てる処理、および探索値の大差が続いた対局をその時点で優勢な側の勝ちとして打ち切る処理（一般には adjudication、将棋では「打ち切り」や「投了」と呼ばれる）である。
以下、この3つを「3つの処理」と書く。
各処理について、資料が理由として挙げていること、効果を測った記録、およびminaseに当てはめたときの前提の違いを分けて書く。
確認日はすべて2026年11月25日であり、ソースコードは各リポジトリのコミットに固定したリンクで示す。

中将棋エンジンminaseは2026年11月14日に、先行するチェスのエンジンに合わせる方針で、3つの処理を教師の生成器に入れた（設計判断の記録である[ADR](../adr/exclude-checks-screen-openings-and-adjudicate-decided-games-in-teacher-generation.md)）。
そのADRが根拠にした教師生成の手順の調査は[自己生成データで評価関数を育てたエンジンの教師生成](teacher-generation-strong-engines/zero-chess-and-variants.md)にあり、本書はそれを補って、各処理を入れた理由と効果の記録を調べたものである。
その後、外部の中将棋エンジンHaChuとの対局で、minaseが優勢を得てから勝ち切るまでの手数が以前の版より長いことが分かった（[調査報告](../audits/mating-speed-against-hachu-2026-11-25.md)）。
本書の調査は、3つの処理がその原因かを確かめるために、処理を外した教師から学んだ評価関数のネットを比べる[設計書](../plans/nnue-teacher-without-adjudication.md)を起案する準備として行った。

## 結論

3つの処理に共通する狙いは、教師生成の計算を節約することと、実戦の探索で評価関数が使われる局面で、教師の値が信頼できるものに学習を絞ることである。
ただし、理由を明記した一次資料は少なく、効果を対局で測った記録は打ち切りについてのHalogenの1件だけだった。

- 打ち切りについて資料が挙げる理由は、計算の節約である。打ち切りが勝ち切りを弱めたと読める報告は、チェスのHalogenと、将棋のdlshogi、elmo、tanuki-、およびやねうら王にあり、その一部は一部の対局だけを打ち切る方式や詰みまで指す方式へ移った。一方で、Viridithas、Integral、Stormphrax、およびSaplingは全対局を打ち切る。
- 開始局面の選別について理由として書かれているのは、「傾きすぎた局面は要らない」という短いコメントだけであり、効果の測定は見つからなかった。
- 王手の局面の除外について資料が挙げる理由は、探索が王手を受けた局面で評価関数を呼ばないことと、王手の局面は評価が不安定なことである。王手の除外だけを切り替えた効果の測定は見つからず、不安定な局面を除く処理一般については、効果の向きが資料ごとに違う。

minaseでは、王手の局面の除外の前提（探索が王手の局面で評価関数を呼ばない）が成り立たず、打ち切りで教師から欠ける勝勢の局面がチェスより多いと見られる（「minaseとの前提の違い」の節）。

## 打ち切り

### 資料が挙げる理由

AlphaGo Zeroの論文は、自己対局の投了の理由を計算の節約としている。
Methodsの節に「In order to save computation, clearly lost games are resigned.」とあり、投了の閾値は、投了しなければ勝てていた対局の割合が5%未満になるように自動で選ぶ。
その割合を測るために、自己対局の10%では投了せずに終局まで指す（[論文の著者版](https://discovery.ucl.ac.uk/id/eprint/10045895/1/agz_unformatted_nature.pdf)の24〜25ページ）。
Lc0のwikiは、投了で対局の長さが約30%縮むと見積もっているが、これはLc0側の推定値である（[training runs](https://lczero.org/dev/wiki/training-runs)）。

elmoのWCSC29（第29回世界コンピュータ将棋選手権）のアピール文書は、自己対局を終局まで指すのが現実的でないので、評価値3,000などで勝ちとみなして打ち切ると書き、その場合は宣言勝ちを学習しにくくなるとも述べている（[アピール文書](https://www.apply.computer-shogi.org/wcsc29/appeal/appeal_final_detailed_190519.pdf)の7〜8ページ）。

全対局を打ち切るチェスのエンジンは、コミットメッセージにもコードにも理由を書いていない。
Viridithasは打ち切りを入れたコミット（[3a00d18](https://github.com/cosmobobak/viridithas/commit/3a00d1801cc9d6225224d5fd94583c1ff2917348)、件名「game adjudication in datagen」）に説明がない。
Integralは閾値を1,500から2,500へ上げたコミット（[9e81ccf](https://github.com/aronpetko/integral/commit/9e81ccf42d9ed12a3af6e2c16d6704d65d901bdc)、件名「Make data generation adjudication less aggressive」）にも理由がない。
Saplingの作者はtalkchessのスレッドで、大差が続いたら勝ちと裁定するよう助言しているが、理由は述べていない（[t=84811](https://talkchess.com/viewtopic.php?t=84811)）。

### 各エンジンの条件

| エンジン | 打ち切りの条件 | 打ち切らずに指す対局 | ソース |
|---|---|---|---|
| Stockfishの学習データ生成器 | 手番側から見た探索値の絶対値が3,000以上で4手続いたら終える。10,000以上なら全対局で直ちに終える。 | 約20%（対局ごとに`prng.rand(10) > 1`で決める） | [training_data_generator.cpp](https://github.com/official-stockfish/Stockfish/blob/9a4c7cf4e311f8d9526b79295b80c4d0464c07cf/src/tools/training_data_generator.cpp)の53行、297〜298行、338〜348行 |
| Fairy-Stockfishのvariant-nnue-tools | 同上 | 約20% | [training_data_generator.cpp](https://github.com/fairy-stockfish/variant-nnue-tools/blob/1809b9debe20a560b17ce819a9685ff131c8f488/src/tools/training_data_generator.cpp)の53行、291〜292行、334〜344行 |
| Lc0 | 勝ち、引き分け、負けのどれかの見込みが閾値を超えたら終える（`resign-percentage`） | 実運用の設定で20%（`resign-playthrough=20`） | [tournament.cc](https://github.com/LeelaChessZero/lc0/blob/fd41ee6be74fb8715b60ac5cc34082d650cb856e/src/selfplay/tournament.cc)の81〜83行と401〜403行、[実運用の設定](https://training.lczero.org/training_runs) |
| Halogen | 白から見て±750を超える状態が6手続いたら終える | 約35% | [datagen.cpp](https://github.com/KierenP/Halogen/blob/1e16bd69f4ef9df378e3cd2899fd2724def9d742/src/datagen/datagen.cpp)の186〜201行 |
| Viridithas | 白から見た絶対値が2,500以上で4手続いたら終える | なし | [datagen.rs](https://github.com/cosmobobak/viridithas/blob/495d5d65267fc553bfc4a044886e8691fc7a1b54/src/datagen.rs)の579〜599行 |
| Integral | 白から見た値が2,500以上で5手（厳密な連続ではない） | なし | [data_gen.cc](https://github.com/aronpetko/integral/blob/7c8d1662d7691ea802f4250dc1fe725b4762fe7f/src/data_gen/data_gen.cc)の217〜221行と268〜273行 |
| Stormphrax | 駒量で正規化した値が±1,250を超える状態が5手続いたら終える | なし | [datagen.cpp](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/src/datagen/datagen.cpp)の82〜90行と227〜252行 |
| Sapling | 絶対値が2,500以上で5手続いたら終える | なし | [DataGenerator.cs](https://github.com/Timmoth/Sapling/blob/76fa659cbc7b1c7be1a6cb5fc722f3dd0ab68060/src/Sapling.Engine/DataGen/DataGenerator.cs)の314〜316行 |
| やねうら王のgensfen | 探索値の絶対値が3,000（既定）以上になった1手で終える | なし | [learner.cpp](https://github.com/yaneurao/YaneuraOu/blob/eb2856f91e6088e5b8c1216e3d84d0563f7cd85f/source/learn/learner.cpp)の824〜885行 |
| tanuki-と、やねうら王のgensfen2019 | 評価値では打ち切らず、詰みまで指す | すべて | [tanuki-の記事](https://nodchip.hatenablog.com/entry/2021/09/24/000000)、[learner.cpp](https://github.com/yaneurao/YaneuraOu/blob/eb2856f91e6088e5b8c1216e3d84d0563f7cd85f/source/learn/learner.cpp)の3397〜3413行 |
| dlshogi | 勝率が閾値（既定0.99）を超えたら終える。詰み探索を有効にすると、詰みを見つけた時点で終える | なし | [self_play.cpp](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/b9b090c5dc68f175a1eada5ede655d711e6fca6c/selfplay/self_play.cpp)の1431〜1446行と1245〜1300行 |

### 効果の記録

チェスのHalogenは、打ち切りの割合を変えた教師から学んだ評価関数のネットどうしを対局で比べた（2025年11月7日にマージされたプルリクエスト[#789](https://github.com/KierenP/Halogen/pull/789)）。
全対局を打ち切った教師から学んだネットは、打ち切りのない教師から学んだネットに対し、評価対局でも打ち切ると+9.38±4.76 Elo、評価対局を打ち切らずに指すと−24.17±4.70 Eloだった。
Halogenはこれを受けて、打ち切った対局の教師と打ち切らない対局の教師を混ぜることにし、駒数の分布が良かった65%対35%の割合を選んだ。この割合の教師から学んだネットは、評価対局を打ち切らない条件で+25.29±4.56 Eloだった。
このプルリクエストは差の原因を駒数の分布で説明しており、勝ち切りの能力とは結びつけていない。

将棋では、dlshogiの山岡忠夫が、閾値による自己対局の打ち切りが詰みの絡む局面の学習を妨げている可能性を挙げ、詰み探索で詰みを見つけるまで指す方式に変えた（[2019年2月4日の記事](https://tadaoyamaoka.hatenablog.com/entry/2019/02/04/215958)）。
変更の後、局面の勝率を予測する出力（value）が、対局サイトfloodgateでレーティング3500以上の強豪の棋譜の勝敗と一致する率が約1%上がったと報告している（[2019年3月2日の記事](https://tadaoyamaoka.hatenablog.com/entry/2019/03/02/104539)）。
dlshogiのWCSC35のアピール文書も、勝率の閾値による打ち切りはモデルの精度が低いうちは学習の精度に影響するとして、詰み探索の結果を報酬にした理由を述べている（[アピール文書](https://www.apply.computer-shogi.org/wcsc35/appeal/appeal_final_250504.pdf)の4.9節）。

tanuki-は教師を打ち切らずに詰みまで指して生成し、学習時に捨てる評価値の閾値`eval_limit`を724から32,000まで変えて比べた。閾値が高いほど勝率が高く、724で37.3%、32,000で65.3%だった（[記事](https://nodchip.hatenablog.com/entry/2021/09/24/000000)）。
これは学習側の閾値の比較であり、生成の打ち切りを直接比べたものではない。
やねうら王の更新履歴には、教師生成時の`eval_limit`は設定しない（上限値の32,000にする）ほうが良いと判明したとの記述があるが、根拠のデータは見つからなかった（[更新履歴](https://github.com/yaneurao/YaneuraOu/wiki/やねうら王の更新履歴2019まで)）。

囲碁のKataGoは投了せず、負け側の勝率が5%未満の状態が続くと探索回数を減らして記録の確率を下げる。論文は、終局まで指すことが「cleaner and less biased training targets」を与え、勝敗の決まった局面でも学習の価値が残ると書いている（Wu, "Accelerating Self-Play Learning in Go", arXiv:1902.10565v5, Appendix D）。

打ち切りが寄せの遅さに結びついたという記述は、チェスでも将棋でも見つからなかった。

## 開始局面の選別

### 資料が挙げる理由

理由として書かれているのは短いコメントだけである。
Viridithasのコメントは「if the position is too good or too bad, we don't want it」である（[datagen.rs](https://github.com/cosmobobak/viridithas/blob/495d5d65267fc553bfc4a044886e8691fc7a1b54/src/datagen.rs)の528〜531行）。
Berserkの設計文書は、閾値を「a shaping knob, not a correctness constraint」、つまり局面の分布を整える調整の値であって正しさのための制約ではないと書く（[設計文書](https://github.com/jhonnold/berserk/blob/1d2354d3250e5ed6fe832d0c6f60eb0b684d3e49/docs/plans/2026-08-29-datagen-genfens.md)）。
将棋のAperyは、開始局面を浅く探索して評価値の絶対値が4,000を超えたら別の局面にし、そのコメントは「最初から評価値が離れすぎていれば、学習に不適なので別の局面にする」である（[usi.cpp](https://github.com/HiraokaTakuya/apery/blob/d14471fc879062bfabbd181eaa91e90c7cc28a71/src/usi.cpp)の427〜464行）。

### 各エンジンの条件

| エンジン | 探索の量 | 捨てる閾値 | ソース |
|---|---|---|---|
| Viridithas | 5万ノード | 絶対値が1,000超 | [datagen.rs](https://github.com/cosmobobak/viridithas/blob/495d5d65267fc553bfc4a044886e8691fc7a1b54/src/datagen.rs)の515〜531行 |
| Integral | 深さ10、100万ノード | 絶対値が300以上 | [data_gen.cc](https://github.com/aronpetko/integral/blob/7c8d1662d7691ea802f4250dc1fe725b4762fe7f/src/data_gen/data_gen.cc)の217〜249行 |
| Stormphrax | 深さ10 | 正規化した値の絶対値が500超 | [datagen.cpp](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/src/datagen/datagen.cpp)の76〜92行と181〜193行 |
| Berserk | 深さ10、100万ノード | 内部値の絶対値が1,000超（表示の約6.3ポーン） | [datagen.c](https://github.com/jhonnold/berserk/blob/1d2354d3250e5ed6fe832d0c6f60eb0b684d3e49/src/datagen.c)の34〜37行と152〜153行 |
| Apery | 深さ4 | 絶対値が4,000超 | 上記の`usi.cpp` |

Stockfishの学習データ生成器とSaplingには、探索値による開始局面の選別がない。
効果を測った記録は見つからなかった。

## 王手の局面の除外

### 資料が挙げる理由

1つ目の理由は、チェスのエンジンの探索が、王手を受けた局面で評価関数を呼ばないことである。
nnue-pytorchの文書は「Since evaluation is not called in check, attacks to a king are also redundant」と書く（[nnue.md](https://github.com/official-stockfish/nnue-pytorch/blob/ef684e9aae2e894df0673b8cfd0062b123289fdd/docs/nnue.md)の2868行）。
Stockfishは評価関数の入口で王手でないことを表明（assert）している（[evaluate.cpp](https://github.com/official-stockfish/Stockfish/blob/3e8fde75412e7c47fc140cd82022a395bc681d92/src/evaluate.cpp)の53行）。
Viridithasの探索のコメントは「when we're in check, it could be checkmate, so it's unsound to use evaluate().」である（[search.rs](https://github.com/cosmobobak/viridithas/blob/495d5d65267fc553bfc4a044886e8691fc7a1b54/src/search.rs)の954〜958行）。
評価関数を呼ばない種類の局面を学んでも探索では使われない、という判断と読める。

2つ目の理由は、王手の局面の評価が不安定なことである。
nnue-pytorchは、指し手が捕獲か王手の局面を「positions that are bad training targets」として読み飛ばす選択肢を導入した（[導入のコミット](https://github.com/official-stockfish/nnue-pytorch/commit/7d91d6c6489975f0fcf3dbf969dcf24fa51e6f72)）。
シャンチー（中国の将棋）の評価関数NNUE（着手ごとに差分で更新できる小さなニューラルネット）の教師を調べた論文は「Because of the volatile nature of a check position, we defined it such that quiet positions cannot contain a check against the king.」と書く（Tan and Watkinson Medina, "Study of the Proper NNUE Dataset", [arXiv:2412.17948](https://arxiv.org/abs/2412.17948)）。

### 各実装の扱い

| 実装 | 除く段階 | ソース |
|---|---|---|
| nnue-pytorch（Stockfishの学習器） | 学習時に読み飛ばす（既定で有効） | [training_data_loader.cpp](https://github.com/official-stockfish/nnue-pytorch/blob/ef684e9aae2e894df0673b8cfd0062b123289fdd/data_loader/cpp/training_data_loader.cpp)の1182〜1183行 |
| Viridithas | 生成した教師を学習に使う形式へ書き出すときに除く（既定で有効） | [dataformat.rs](https://github.com/cosmobobak/viridithas/blob/495d5d65267fc553bfc4a044886e8691fc7a1b54/src/datagen/dataformat.rs)の44〜87行 |
| Stormphrax | 生成時に印を付け、出力形式marlinformatでは保存しない。出力形式viriformatでは印にかかわらず保存する | [datagen.cpp](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/src/datagen/datagen.cpp)の255行、[marlinformat.cpp](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/src/datagen/marlinformat.cpp)の31〜33行 |
| Fairy-Stockfishのvariant-nnue-tools | 生成時に常に除く | [training_data_generator.cpp](https://github.com/fairy-stockfish/variant-nnue-tools/blob/bad549a6b5555a8183bf10026f271cf980f94266/src/tools/training_data_generator.cpp)の361〜368行 |
| Stockfishの学習データ生成器、Integral | 生成時には除かない | 上記の各ソース |
| やねうら王（gensfen、learn） | 除かない（`learner.cpp`の全体で王手の判定を検索し、教師から除く処理がないことを確かめた） | [learner.cpp](https://github.com/yaneurao/YaneuraOu/blob/eb2856f91e6088e5b8c1216e3d84d0563f7cd85f/source/learn/learner.cpp) |
| elmo、Apery | 開始局面のランダムな手で王手を避けるだけで、教師から除く処理は見つからなかった | [elmo_for_learnのusi.cpp](https://github.com/mk-takizawa/elmo_for_learn/blob/f825ff057165826c421d2d7ca9a841bdac2b9088/src/usi.cpp)の409〜410行 |

探索側でも、Stockfish、Fairy-Stockfish、Viridithas、Stormphrax、およびBerserkは、王手を受けた局面で静的評価を呼ばない。
該当箇所は、Stockfishが[search.cpp](https://github.com/official-stockfish/Stockfish/blob/3e8fde75412e7c47fc140cd82022a395bc681d92/src/search.cpp)の844〜845行と1740〜1741行、Fairy-Stockfishが[search.cpp](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/52838835f7234e29fb48fd88eb39fb3c7e46fcfa/src/search.cpp)の872〜878行、Viridithasが[search.rs](https://github.com/cosmobobak/viridithas/blob/495d5d65267fc553bfc4a044886e8691fc7a1b54/src/search.rs)の587〜590行と954〜958行、Stormphraxが[search.cpp](https://github.com/Ciekce/Stormphrax/blob/d1468d99e3d3733100d20d682ac627998cce94b2/src/search.cpp)の777〜790行と1513〜1514行、Berserkが[search.c](https://github.com/jhonnold/berserk/blob/1d2354d3250e5ed6fe832d0c6f60eb0b684d3e49/src/search.c)の606〜607行である。

### 効果の記録

王手の局面の除外だけを切り替えた効果の測定は見つからなかった。
不安定な局面を除く処理一般については、効果の向きが資料ごとに違う。
Texel（チェスの手作り評価関数の調整）では、静止探索の値が探索値から大きくずれる局面の除外をやめると+39.4 Eloだった。作者は、そうした局面も「the q-search function has to deal with them all the time in real games」と説明している（[Chessprogramming wikiのTexel's Tuning Method](https://www.chessprogramming.org/Texel%27s_Tuning_Method)のResults節）。
一方、山岡忠夫は将棋のNNUEの学習で、静止探索の読み筋が1手以上ある局面を除くと+87.2のレーティング差で強くなったと報告している（[2022年6月28日の記事](https://tadaoyamaoka.hatenablog.com/entry/2022/06/28/222714)）。

## minaseとの前提の違い

以下は、上の事実を2026年11月25日の時点のminaseに当てはめた見方であり、測定で確かめたものではない。

| 処理 | 先行エンジンの前提 | minaseでの違い |
|---|---|---|
| 打ち切り | 計算を節約でき、チェスでは勝勢から勝ち切るまでの手数が短く、終盤は駒数の少ない局面の完全解析データ（tablebase）と探索で扱える。 | 中将棋は勝勢から勝ち切るまでが長い。HaChuとの対局では、駒得差+5,000から終局まで中央値で40〜63手かかった（[調査報告](../audits/mating-speed-against-hachu-2026-11-25.md)）。minaseの世代反復の第3世代の教師のうち番号0のファイルでは、7,500局のうち2,728局が打ち切りで終わった。 |
| 開始局面の選別 | 最初から勝敗が決まった対局で教師と計算を無駄にしない。 | 寄せの速さとの関係は薄いと見られる。 |
| 王手の局面の除外 | 探索が王手を受けた局面で評価関数を呼ばないので、学ばなくても使われない。 | minaseの静止探索は、王手を受けた局面でも静的評価で探索を打ち切る（`crates/minase/src/search/alphabeta/quiesce.rs`）。教師から除いた種類の局面の評価を、探索で使っている。 |

minaseの学習は、教師の探索値だけを目標にして対局の勝敗を混ぜない（混合比λ＝1）ので、打ち切りが学習に与える影響は、どの局面が教師に入るかを変えることに限られる。
そのため、minaseで打ち切りが持つ利点は主に生成の計算の節約である。設計書は、打ち切りをやめると生成時間が1.04〜1.24倍になると見積もっている（[設計書](../plans/nnue-teacher-without-adjudication.md)の「生成の見積もり」の節）。

## 見つからなかったもの

次の記録は、今回の調査では見つからなかった。

- 打ち切りが寄せや詰ませの遅さに結びついたという記述は見つからなかった。talkchessのスレッド（t=84811、t=83944、t=75477）、GitHubのissueとコミットの検索、nnue-pytorchのissue、およびLc0のwikiを調べた。Stockfish DiscordとEngine Programming Discordは公開の記録に到達できず、調べていない。
- 開始局面の選別の閾値について効果を測った記録は見つからなかった。
- 王手の局面の除外だけを切り替えて効果を測った記録は見つからなかった。
- 中将棋と大型将棋の教師生成に関する記述は見つからなかった。HaChuの公式ページにあるのも、手作りの評価関数の説明だけである。

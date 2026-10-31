# AlphaZero型の学習の局面の選び方と方策の損失の正規化の先行実装調査

本書は、自己対局で方策・価値ネットワークを学ぶ先行実装について、2つの点を一次資料で調べた調査メモである。
1つ目は、自己対局の棋譜から学習の局面をどう選び、同じ対局の局面どうしの相関と価値の過学習にどう対処しているかであり、2つ目は、方策の損失のsoftmaxを全出力にとるか、合法手だけに正規化するかである。
調査の動機は、[強化学習の実行](../plans/alphazero-rl.md)のサイクル1の後に、対局結果に対する価値の損失が、学習に使った対局の標本で0.509、学習に使っていない検証の対局で0.742となったことにある。
検証の値は、勝敗を常に五分と予測したときのln 2（約0.693）より大きく、方策の損失は訓練4.427と検証4.441で離れていなかった。
設計判断は強化学習の実行の設計書の「学習窓と局面の抽出」の節が定め、本書はその根拠を保持する。
調査日は2026年10月29日である。
出典のない値と設計上の含意には「推論」と付ける。

## 調べた版

各リポジトリは2026年10月29日に浅いクローンで取得して読み、コードは実行していない。
行番号はすべて次の版のものである。

| 対象 | リポジトリと版 | ライセンス |
|---|---|---|
| lczero-training | LeelaChessZero/lczero-training、ee626a97（2026年10月24日） | `tf/*.py`はGPL-3.0-or-later。新しいファイルの多くは表記なし |
| lc0（探索側） | LeelaChessZero/lc0、d91bd144（2026年10月24日） | GPL-3.0 |
| Leela Zero | leela-zero/leela-zero、3ee6d20d（2023年2月6日） | GPL-3.0 |
| Minigo | tensorflow/minigo、6d89c202（2021年3月4日） | Apache-2.0 |
| KataGo | lightvector/KataGo、d91ea855（2026年9月27日） | MIT型 |
| dlshogi | TadaoYamaoka/DeepLearningShogi、1efd02ba（2026年10月29日） | GPL-3.0 |
| Gumbel dlshogi | TadaoYamaoka/gumbel-dlshogi、1cae574a（2025年12月24日） | LICENSEなし |
| AobaZero | kobanium/aobazero、5eb94416（2026年7月20日） | 学習部は原則パブリックドメイン、`bin/aobaz`はGPLv3 |
| Polygames | facebookincubator/Polygames、eb5390e5（2021年1月19日） | MIT |

論文は、AlphaGo（Silver et al., Nature 529, 2016、DeepMindが配布するPDF）、AlphaGo Zero（Nature 550, 2017、UCL Discoveryの受理稿）、AlphaZero（arXiv:1712.01815）、およびKataGo（Wu, arXiv:1902.10565v5）を読んだ。
GPLのコードはminase（MIT）へ写さず、本書の用途は設計の参考に限る。

## 結論

価値の過学習を同じ対局の局面の相関に結びつけた明文は、AlphaGoの論文にある。
本文の「Reinforcement learning of value networks」の節（Nature誌p.486）は、全対局のデータで勝敗を回帰すると過学習するとし、その理由を "successive positions are strongly correlated, differing by just one stone, but the regression target is shared for the entire game" と書く。
KGSのデータでこの方法で学んだ価値ネットワークはMSEが訓練0.19、試験0.37となり、別々の対局から1局面ずつ取った3,000万局面に替えると0.226と0.234に縮まった（Methods「Value network: regression」、PDF p.8）。
Minigoの開発記録`RESULTS.md`にも、価値ネットワークが、学習に使わない保留の対局で、勝敗を五分と予測するより悪くなるまで過学習した事例がある（L229-240）。

調べた実装はどれも、1つの対局の局面が1回の学習にまとまって入らないように、学習窓を数万〜数十万局に広げるか、対局の局面を間引くか、その両方を行う。
1サイクルの全局面を直近の1サイクルだけで1回読む構成は、Gumbel dlshogiの既定と同じであり、minaseのサイクル1〜5の構成に最も近い。

| 実装 | 対局の局面の間引き | 学習窓 | 1局面の使用回数 |
|---|---|---|---|
| AlphaGo（価値ネット） | 1局から1局面 | 3,000万局 | 1回 |
| AlphaGo Zero | なし。窓の全局面から一様に抽出 | 直近50万局 | 推論で1対局あたり約220〜290回の抽出 |
| lczero-training（旧） | 各局面を確率1/32で採る（`tf/train.py` L29の定数） | 設定例10万局 | 間引きはバッチ内の対局の多様さを上げるためのもの |
| lczero-training（新） | `position_sampling_rate`か`position_count`の明示が必須（設定例0.03） | 設定例5万局 | 1/p回で全局面を1回ずつ使う抽出 |
| Leela Zero | 確率1/16（`--sample`の既定） | サーバー側で管理（未確認） | ― |
| Minigo | 世代ごとに各対局の約2%（ポアソン分布） | 約50万局 | 推論で平均約0.5回 |
| KataGo | 全探索の手番（約25%）だけを記録 | 行数のべき乗で25万行から約2,200万行 | 平均4回（手引き）または8回（簡易ループ）まで。「大きいと過学習」と注記 |
| AobaZero | なし。短い対局の重みを下げる | 直近150万局（重みの更新の約150倍） | 推論で平均約1回 |
| dlshogi | なし | コード外の運用 | 既定1エポック |
| Gumbel dlshogi | なし | 既定で直近1ファイル（100万局面）、スケジュールで2、4、8、16 | 1回 |

方策の損失を学習でも合法手だけに正規化するのは、lczero-trainingの選択肢とPolygamesだけであった。
lczero-trainingの`mask_legal_moves`（旧、`tf/tfprocess.py` L426-437）と`IllegalMoveHandling.MASK`（新、`proto/training_config.proto` L139-159）は、コード上の既定は無効で、新しいパイプラインの設定例`docs/example.textproto`では有効になっている。
この機能を入れたプルリクエスト#64（2019年4月）の本文は、作者の測定として「about 30 Elo on CCRL」を報告するが、条件と誤差は書いていない。
Polygamesは損失関数`pypolygames/model_zoo/loss.py` L28-37で非合法手のlogitを−400にしてからsoftmaxをとる。
AlphaGo Zero、Leela Zero、Minigo、KataGo、dlshogi、Gumbel dlshogi、およびAobaZeroは全出力のsoftmaxで学習し、非合法手の出力は方策の目標の確率が0であることだけで抑え、探索のときに合法手で正規化し直す。
AlphaZeroの論文は「Illegal moves are masked out by setting their probabilities to zero, and re-normalising the probabilities for remaining moves」とだけ書き、学習時のマスクの有無は書いていない（arXiv版p.14）。

## 実装ごとの根拠

**lczero-training。** 旧パイプラインの`tf/chunkparser.py`は、`sample_record()`が「skips most training records to avoid sampling over-correlated positions」と説明し（L34-35）、L424-427で局面を確率1/32で採る。
新パイプラインの`csrc/loader/stages/chunk_unpacker.cc`は、`position_sampling_rate`と`position_count`のどちらか一方の明示を求め（L140-143）、対局ごとに固定した乱数種で局面を互いに素な部分集合に分ける（L30-65）。
lc0の開発wikiの「Large Elo fluctuations starting from ID253」は、2018年の不調の最大の要因を過学習とし、同じ対局から多くの局面を抽出したことと、シャッフルのバッファが小さすぎたことを挙げる（WebFetchの要約を通して読んだので、字句は原文と照合していない）。

**Leela Zero。** `training/tf/parse.py` L44-46は`DOWN_SAMPLE = 16`を定め、「This helps improve the spread of games in the shuffle buffer.」と理由を書く。
8通りの対称変換を乱択で施す（`chunkparser.py` L325-327）。

**Minigo。** `rl_loop/example_buffer.py`は、1対局から取る局面数を`np.random.poisson(len(protos) * sampling_frac)`で決め、`sampling_frac`の既定は0.02である（L51-62、L80-84）。
1世代は2,097,152局面で（L43-45）、窓は500,000局まで広がる（L206-212）。

**KataGo。** 自己対局は、各手番を確率0.25で全探索、残りを速い探索とし、全探索の手番だけを記録する（`cpp/configs/training/selfplay1.cfg` L58-76、論文3.1節）。
`python/shuffle.py`は窓の大きさを全行数のべき乗で決め（L719-760）、`python/train.py`は新しいデータ1行が増えるごとに学習してよい行数の上限を設けて、各行の使用回数を抑える（L1400-1415）。
手引き`SelfplayTraining.md` L67は値4を勧め、`python/selfplay/synchronous_loop.sh`は値8に「Larger numbers may cause overfitting.」と注記する。
論文5.2節は、探索の上限を固定した比較の実験で、データ不足による過学習を避けるために窓を2倍にしたと書く。

**dlshogiとGumbel dlshogi。** dlshogiの`dlshogi/train.py`は渡された局面をすべて読み、価値の損失を(1−λ)・勝敗＋λ・探索値の2値交差エントロピーとし、λの既定は0.333である（L29、L61-63、L325-339）。
Gumbel dlshogiの訓練データは局面、方策の目標、および対局結果だけを持ち（`gumbel_dlshogi/common.py` L5-11）、価値は勝敗だけで学ぶ（`train.py` L39-45）。
したがって、minaseのλ=0.333は従来のdlshogiの既定値であり、Gumbel dlshogiの値ではない。
Gumbel dlshogiのブログ「その８」（2025年7月19日）では、訓練データに対する価値の損失が0.591から0.433へ下がる一方、評価データに対する価値の損失は0.763から0.981へ上がった。ただし評価データはfloodgateの棋譜であり、自己対局の対局を訓練用と検証用に分けた比較ではない。

**AobaZero。** `learn/yss_dcnn.cpp`は直近150万局を環状バッファに保持し（L2811-2848）、新しい1局ごとに128局面を学習する（L5419-5450）。
同梱の説明書`learn/learn.txt`は「過去100万棋譜からランダムに64局面を取り出して」と書き、現行のコードと値が異なる。

## ふかうら王の学習の記述

YaneuraOuのwikiの「ふかうら王の学習」（wikiのリポジトリのコミット4f1b5891、2026年10月15日版）は、dlshogiの学習器で将棋のネットを教師あり学習するときの経験則として、次の3点を書く。
教師局面はモデルのパラメータ数の10倍以上を目指し、計算資源が十分なら、パラメータ数の100倍程度の局面数までは棋力が伸び続けるとする（「学習の基本その1」）。
棋譜は自己相関が強いので、2,600万パラメータのモデルでは、1,000万局面だけを生成するより、3,000万局面を生成してシャッフルし1,000万局面を抽出したほうが正解率が良かったとする（「学習の基本その2」）。
また、教師データの一部を分けた検証用の局面では過学習に気づかないことがあるので、検証用データは教師の生成と無関係の棋譜から取るとする（「学習の基本その4」）。

## minaseへの含意（推論）

minaseのサイクル1〜5は、1サイクル約100万局面の全局面を直近1サイクルの窓で1回学ぶ構成であり、価値ヘッド（value head）が見る独立な勝敗は1サイクルの対局数（9,100局から2,716局）に限られていた。
局面を間引くだけでは1回の学習のバッチ内の相関は減るが、独立な勝敗の数は増えないので、学習窓を複数のサイクルに広げて抽出する方式が、AobaZero、Minigo、およびlczero-trainingの構成に当たる。
方策の損失の正規化は、探索が合法手のlogitだけを読んで正規化し直す点でlc0、Leela Zero、およびMinigoと同じであり、minaseの合法手に乗る確率質量はマスクなしで0.957から0.994へ上がった。

## 確かめられなかった事項

lc0の本番の学習の`position_sampling_rate`、`illegal_moves`、および窓の大きさは、リポジトリにも参照した頁にもなかった。
Leela Zeroの窓の大きさは、二次情報しか見つからなかった。
AlphaZeroの補足資料の擬似コードは、science.orgが403を返したので取得できず、第三者の書き写しを要約器を通して読んだだけである。
dlshogiのλ=0.333の由来、従来のdlshogiの強化学習の窓の大きさ、およびKataGoの1周期のデータ量に対する窓の比は確かめていない。
方策の損失を合法手に限るかどうかの比較の報告は、lczero-trainingのプルリクエスト#64の作者の報告のほかに見つからなかった。

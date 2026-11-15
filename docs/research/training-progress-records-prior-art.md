# 学習の進み具合と強さの推移の記録と可視化に関する先行例の調査

本書のminaseに関する記述は2026年11月16日時点のものである。
minaseは、本プロジェクトが開発している中将棋エンジンである。

## 結論

評価関数やニューラルネットワーク（以下、ネット）を学習するゲームAIの開発で、学習の進み具合と強さの推移をどう記録し、どう図にしているかを、チェスのNNUE系、AlphaZero型、および将棋のNNUE系の3系統について調べた。
NNUEは、着手ごとに差分で更新できる小さなニューラルネットワークの評価関数である。
AlphaZero型は、ネットで導く探索の自己対局から教師を作り、ネットを学び直すことを繰り返す方式である。
調査の動機は、minaseのNNUEの世代反復（前の世代のネットで教師の局面を作り、次の世代を学ぶ繰り返し）とAlphaZero型の強化学習の記録を、どの単位で、どの形式で残すかを決めることである。

結論は6点ある。

1. 調べた範囲で、強さの推移の図を自動で更新して公開しているのは、分散学習の学習サイトを持つ、チェスのLeela Chess Zero（以下、lc0）、囲碁のLeela Zero、および囲碁のKataGoの3つである。3つとも、ネットの一覧と対局の結果をサーバーのデータベースに持ち、ページを開くたびに図を描く。
2. 強さの物差しは2通りに分かれる。lc0とLeela Zeroは、新しいネットと現行の最良ネットとの対戦から求めたEloの差を順に足し上げる。KataGoは、自己対局とは別に評価用の対局を行い、全ネットのEloを同時に推定して、標準偏差の約2倍の幅の誤差棒を付ける。連続するネットの対から求めた自己対局のEloが膨張することは、ELF OpenGoの論文が述べている。lc0の公式ブログは、足し上げた自己対局のEloが、より正確な推定の動きに従わないことが多いと述べている。
3. チェスと将棋のNNUE系では、ネットの一覧を持つfishtestとOpenBenchに、ネットごとのEloも強さの推移の図もない。強さは、現行の既定ネットとの対局の結果として、文字で残す。対局は、逐次確率比検定（以下、SPRT）か固定の局数で行う。図があるのは、1回の学習の中のエポックごとのネットのEloを手で描くnnue-pytorchのスクリプトである。
4. 学習の指標（損失や一致率）は、どの系統でも学習器ごとの簡単な記録に任されている。記録先は、TensorBoard、CSV、JSON、または標準出力のテキストである。lc0とKataGoの公開サイトは、損失の図を載せていない。
5. 1回の学習を単位にして、学習曲線、系譜、強さ、およびパラメータを1か所にまとめる例は、将棋のNNUE学習器tataraとその作者の実験管理サイトnnue-labである。tataraは学習1回につき1つのJSONファイルを書き、nnue-labがそれを実験ごとのページに表示する。
6. 図の形の先例は次のとおりである。損失と強さを1枚の図に縦に並べる形は、Stockfishの学習器nnue-pytorchの`do_plots.py`にある。ただし、損失の横軸は更新のステップ、Eloの横軸はエポックであり、横軸は揃えていない。誤差棒つきの点で強さを描く形はKataGoに、横軸に累積の学習量を取る形はlc0、KataGo、およびAlphaZeroの論文にある。学習の設定を変えた時点を図に示す例はlc0にあり、学習率を下げた時点に点を置いている。自己対局の性質（1局の手数や引き分けの割合）の推移を図にする例は、調べた範囲にはなかった。

## 調べた版と確認の方法

手元に複製がある実装は、次の版のソースを読んだ。

| 実装 | コミット | 複製の場所 |
|---|---|---|
| Stockfish | コミット`c1fff716`のメッセージ | `../stockfish` |
| variant-nnue-pytorch（nnue-pytorchをチェスの変種向けに改めた派生） | `749db94` | `../variant-nnue-pytorch` |
| DeepLearningShogi（dlshogi） | `a9e7e5f`（2026年11月10日） | `../DeepLearningShogi` |
| やねうら王 | `3b8d53fa`。旧学習器は`0a6dd2cb`の直前の版 | `../YaneuraOu` |
| tatara | `87f1955`（2026年11月5日） | `../tatara` |
| BulletOu-minishogi（BulletOuを5五将棋向けに改めた派生） | `cc202f3` | `../BulletOu-minishogi` |

Web上の資料は、2026年11月16日にページまたはGitHub上のソースを取得して読んだ。
GitHub上のソースの版は、fishtestが`f7af98f8`、nnue-pytorchが`8d30ed74`、nettestが`6b75d796`、Bulletが`5d701776`、チェスエンジンViridithasが`a23af048`、OpenBenchが`230ed46e`である。
lczero-server、lczero-training、katago-server、およびKataGoは、取得した日のmasterを読んだ。lczero-serverの最後の更新は2021年10月21日、katago-serverの最後の更新は2023年3月5日である。
nnue-labは、同じ日にブラウザで表示して確かめた。

取得できなかった資料は次のとおりである。
fishtestのサイト（tests.stockfishchess.org）はボットの検査画面を返したので、ネットの一覧の構成はテンプレートのソースで確かめた。
AlphaGo Zeroの論文（Nature 2017）の本体、Minigoの進捗の公開サイト（cloudygo.com）、およびQhapaq氏のレーティングのサイトは取得できず、内容を確かめていない。
lc0の評価用の対局の探索ノード数と、lc0でEloの再推定を誰がいつ実行しているかは、資料から読み取れなかった。

## チェスのNNUE系

### Stockfishとfishtest

fishtestは、Stockfishの変更を分散して対局で検定するサーバーである。
ネットは`nn-<SHA-256の先頭12桁>.nnue`という名前で区別し、サーバーはアップロードした利用者、名前、ダウンロード数、既定のネットになったかどうか、およびそのネットを使った最初と最後の検定だけを記録する（fishtest [`nns_content_fragment.html.j2`](https://github.com/official-stockfish/fishtest/blob/f7af98f8/server/fishtest/templates/nns_content_fragment.html.j2)、[`rundb.py`](https://github.com/official-stockfish/fishtest/blob/f7af98f8/server/fishtest/rundb.py)）。
ネットごとのElo、学習データ、および親のネットは記録せず、ネットの一覧のページに図はない。
系譜と学習の手順は、Stockfish本体のコミットメッセージに文章で書かれる。
たとえば`c1fff716`（2023年5月12日）は、最初の学習と3回の再学習からなる4段階の学習の内容とコマンドを本文に書いている。

強さの推移の図は、ネットの単位ではなく、エンジンの版の単位にある。
[StockfishのwikiのRegression Testsのページ](https://github.com/official-stockfish/Stockfish/wiki/Regression-Tests)は、開発中の版を前回のリリースと60秒＋0.6秒で60,000局対局させ、Eloを日付または前回のリリースからの経過日数に対して、点つきの折れ線で描く。
図は、Googleスプレッドシートのグラフを画像として埋め込んだものである。

### nnue-pytorchとnettest

Stockfishの学習器nnue-pytorchは、TensorBoardとCSVに、訓練損失、検証損失、および学習率を書く（[`train.py`](https://github.com/official-stockfish/nnue-pytorch/blob/8d30ed74/train.py)、[`trainer/loggers.py`](https://github.com/official-stockfish/nnue-pytorch/blob/8d30ed74/trainer/loggers.py)）。
同じリポジトリの`run_games.py`は、学習の途中のネットを順に対局させ、レーティングを計算するプログラムordoで、masterのネットを0に固定したEloを求める。
[`do_plots.py`](https://github.com/official-stockfish/nnue-pytorch/blob/8d30ed74/do_plots.py)は、訓練損失（横軸はステップ）、検証損失（横軸はステップ）、およびエポックごとのElo（横軸はエポック、誤差棒つき）を、縦3段の1枚の画像にまとめる。
損失とEloの横軸は別の量であり、共通の軸への換算はしていない。
複数の実験は凡例で重ねる。
手元のvariant-nnue-pytorchでも同じ構成を確かめた（`do_plots.py` 98〜110行と166〜171行、`run_games.py` 46〜68行）。
このスクリプトは手で実行するもので、学習の一部として自動では動かない。

公式の学習手順は、2026年の時点でnettestへ移っている。
nettestは、学習の各段に、直前の段のハッシュとその段の設定から作る12桁のハッシュを付け、最終のネットを参照のStockfishとSPRTで対局させて、結果をYAMLファイルと継続的インテグレーションのログに残す（[`generate_pipeline.py`](https://github.com/vondele/nettest/blob/6b75d796/nettest/generate_pipeline.py)、[`test.py`](https://github.com/vondele/nettest/blob/6b75d796/nettest/test.py)）。
作図のコードは持たない。

### BulletとOpenBench

BulletはRustで書かれたNNUEの学習器である。
損失を標準出力と、チェックポイントごとの`log.txt`（superbatch、バッチ、損失の3列）に書くだけで、作図の機能はない（[`logger.rs`](https://github.com/jw1912/bullet/blob/5d701776/crates/trainer/src/run/logger.rs)）。
superbatchは、学習を区切る単位で、決まった数のバッチからなる。

OpenBenchは、エンジンの変更を分散して検定するサーバーである。
ネットの一覧は、既定の印、エンジン、作者、名前、およびSHA-256の先頭8桁だけを持ち、Eloも図も検定へのリンクも持たない（[`models.py`](https://github.com/AndyGrant/OpenBench/blob/230ed46e/OpenBench/models.py)、[`networks.html`](https://github.com/AndyGrant/OpenBench/blob/230ed46e/Templates/OpenBench/networks.html)）。

Bulletを使うエンジンは、強さをOpenBenchのSPRTで測り、結果の文字列をプルリクエストの本文やリリースの説明に貼る。
記録が最も厚いViridithasは、ネットの来歴を1,851行の手書きのテキストの表（[`notes/networkhistory.txt`](https://github.com/cosmobobak/viridithas/blob/a23af048/notes/networkhistory.txt)）で管理し、各行に、ネットの名前、変更の内容、および直前のネットとのSPRTの結果を書いている。

## AlphaZero型

### lc0とLeela Zero

lc0の学習サーバーは、ネット（番号、SHA、ブロック数、フィルタ数、対局数、Elo）と、ネット同士の対戦（勝ち、負け、引き分け、合否）をデータベースに記録する（lczero-server [`src/db/models.go`](https://github.com/LeelaChessZero/lczero-server/blob/master/src/db/models.go)）。
[学習サイト](https://training.lczero.org/)の図は、横軸が累積の対局数、縦軸がEloで、最良ネットを結ぶ折れ線と、全対戦の点を重ねる。
Eloは、合格した対戦のEloの差を順に足し上げた値であり、図に不確かさは出ない（[`main.go`](https://github.com/LeelaChessZero/lczero-server/blob/master/main.go)）。
ネットの一覧の表は、対戦ごとのEloの差と95%の誤差の幅を載せる。
Leela Zeroのサイトも同じ構成で、横軸が累積の対局数、縦軸が乱択を0としたEloである。

lc0の[公式ブログ](https://lczero.org/blog/2018/10/lc0-training)は、足し上げたこの値を自己対局のEloと呼んで粗い推定とし、より正確な推定とは一致しないことが多いと述べている。
より正確な推定として、Eloが既知のエンジンとの対局と、固定の条件のStockfishとの対局を挙げる。
同じ記事は、これらの推定を重ねた図に「LR」と書いた点があり、学習率を下げた時点を表すと説明している。
サーバーには、全対戦の棋譜からordoで再推定した値をネットに書き込む道具もある（`cmd/prepare_match_pgns`、`cmd/populate_elo`）。
学習の指標は、学習器がTensorBoardに書く（lczero-training [`tf/tfprocess.py`](https://github.com/LeelaChessZero/lczero-training/blob/master/tf/tfprocess.py)）。
項目は、方策の損失、価値の損失、正則化の項、勾配のノルム、および検証データでの方策と価値の一致率である。

### KataGo

KataGoのサーバーは、ネットごとに、親のネット、学習したサンプル数、データの行数、SHA-256、およびレーティングの値とその不確かさを記録する（katago-server [`network.py`](https://github.com/katago/katago-server/blob/master/src/apps/trainings/models/network.py)）。
学習用の自己対局と、評価用の対局は、データベースの別々の表に記録する。
評価用の対局の相手は、不確かさが大きいネットや対局の少ないネットを優先して選び、レーティングは、親のネットとの仮想の引き分けを事前分布として、全ネットで同時に推定する（[`bayesian_elo.py`](https://github.com/katago/katago-server/blob/master/src/apps/trainings/services/bayesian_elo.py)）。

[学習サイト](https://katagotraining.org/)の図は、ネットごとの点と、約95%の信頼区間を表す縦線で描く。
横軸は、アップロードの時刻、データの行数（線形）、およびデータの行数（対数）から選べる。
ページは、KataGoのネット同士でしか対局しないので値が少し大きく出ている可能性があると注記している。
ネットの名前は`<名前>-s<学習したサンプル数>-d<データの行数>`の形で、学習量が名前から分かる。

学習器は、損失と一致率を、1行に1つのJSONを書くファイル（`metrics_train.json`、`metrics_val.json`）とログに書くだけで、TensorBoardも作図の道具も持たない（[`python/train.py`](https://github.com/lightvector/KataGo/blob/master/python/train.py)）。
[論文](https://arxiv.org/abs/1902.10565)の図は、横軸が自己対局の費用（推論の回数に換算した値）の対数、縦軸が相対のEloの折れ線である。

### 論文

[AlphaZeroの論文](https://arxiv.org/abs/1712.01815)のFigure 1は、横軸が学習のステップ数、縦軸がEloの折れ線であり、StockfishやElmoなどの基準を水平線で示す。
Eloは、学習の途中の各時点のネットと基準のプレイヤーを1手1秒で総当たりさせ、レーティングを推定するプログラムBayesEloで求めている。
図に誤差棒はない。

[ELF OpenGoの論文](https://arxiv.org/abs/1902.04522)は、4.2節で、連続するモデルの対から求めた自己対局のEloが膨張すると述べ、理由として、直前のモデルに勝つだけで値が上がることを挙げる。
同じ論文は、損失の推移、固定のモデルに対する勝率、およびプロの棋譜の手との一致率を図にしている。

### dlshogi

dlshogiの学習器`dlshogi/train.py`は、訓練損失、テスト損失、およびテストデータでの方策と価値の一致率を、テキストのログに書く（359〜376行）。
`dlshogi/utils/plot_log_policy_value.py`は、このログを読み、横軸がステップ数、左の縦軸が損失、右の縦軸が一致率の折れ線を描くが、画像は保存しない。
PyTorch Lightningを使う別の学習器`dlshogi/ptl.py`については、開発者のブログが、ログをTensorBoardへ出力すると述べている（[2024年2月28日の記事](https://tadaoyamaoka.hatenablog.com/entry/2024/02/28/220912)）。手元の版の設定ファイルに、ログの出力先の指定はない。
開発者のブログは、世代ごとのfloodgate（コンピュータ将棋の対局サーバー）の棋譜との一致率を表で載せ、強さを、固定の相手（教師あり学習で作ったネットと、将棋ソフトGPSFish）との100局ずつの勝率で示す（[2018年8月26日の記事](https://tadaoyamaoka.hatenablog.com/entry/2018/08/26/221248)）。
世代に対するEloの推移の図はない。

## 将棋のNNUE系

### やねうら王とBulletOu

やねうら王の旧学習器（2026年5月13日に削除）は、100万局面ごとに、検証と学習の交差エントロピー、評価値のノルム、および指し手の一致率を標準出力へ1行で書いた（`0a6dd2cb`の直前の`source/learn/learner.cpp`）。
保存は連番のディレクトリで行い、対局のスクリプトがその連番を順に対局させて、勝敗と、引き分けを除いた勝率から求めたレーティングの差を1行で出した。
作図の仕組みはない。
[2023年7月7日のブログの記事](https://yaneuraou.yaneu.com/)は、少なくとも3,000局ほど前の版やほかのソフトと対局させることと、レーティングの推定にordoを使うことを勧めている。

現在のやねうら王の学習器BulletOuは、superbatchごとに、検証の一致率、検証損失、訓練損失、学習率、および局面数をCSVに書く（BulletOu-minishogi `docs/spec/04-checkpoint-layout.md`）。
作図は、チュートリアルにmatplotlibの例があるだけである。

### tataraとnnue-lab

tataraは、学習1回につき1つの`experiment.json`を書く（`crates/nnue-train/src/experiment.rs`）。
superbatchごとに、一時ファイルへ全体を書いてから名前を付け替えるので、中断しても妥当なJSONが残る。
主な項目は、識別子、名前、日付、状態、コミット、コマンド、系譜（親の識別子、再開したチェックポイント、同じレシピの続きか設定を変えた派生かの別）、全パラメータ、データの局面数、結果（最良の損失、最良のテスト損失、学習の時間）、およびsuperbatchごとの履歴（損失、テスト損失、テストの一致率）である。

[nnue-lab](https://nnue-lab.sh11235.com/t/sh11235)は、この`experiment.json`を取り込む実験管理のWebアプリである。
一覧のページは、実験1件を1行とする表で、名前、損失の小さな折れ線、日付、構造、最適化器、学習率、バッチの大きさ、最良の損失、一致率、superbatchの数、および学習の時間を載せ、構造とタグで絞り込める。
実験のページは、横軸がsuperbatch、縦軸が損失の折れ線に、訓練とテストの2本を重ね、タブでテストの一致率の図に切り替える。
その下に、系譜、対局成績、ノート、全パラメータ、データセットの局面数、および生成元の版が並ぶ。
対局成績は、対局サーバーでのレーティング（初期値1500、K=32の逐次の更新）と勝敗数であり、棋譜の一覧へリンクする。
世代をまたぐ強さの推移の図は、見た範囲にはなかった。

tatara本体には、作図も対局の実行器もない。
強さは、同じ作者の将棋エンジンrshogiの道具`tournament`で、先後を入れ替えた2局の組の結果を5通りに分類して行うSPRTによって測り、結果を、1行に1つのJSONを書くファイルに残す。

### nodchip氏の記録

tanuki-の開発者nodchip氏は、実験1件をブログの記事1本にまとめる。
[2026年10月21日の記事](https://nodchip.hatenablog.com/entry/20261021-084554-9c2f8e9b-9494-42e9-995f-c79443b830fa)は、横軸がsuperbatch、縦軸が訓練損失の折れ線を1枚載せ、検証損失と対局の結果は表と本文で示す。
対局は、前のネットを相手に、先後を入れ替えた8,192局を行い、2局を1組とした95%信頼区間を求めている。
採否の規則は、測定の前に決めている。

## 比較

| 対象 | 学習の指標の記録 | 強さの図 | 強さの物差し | ネットの区別と系譜 |
|---|---|---|---|---|
| Stockfishとfishtest | 学習は別のリポジトリの学習器で行う | ネットの単位にはない。版の単位で、前回のリリースに対するEloの折れ線 | 現行の既定ネットとのSPRT | SHA-256の先頭12桁。系譜はコミットメッセージ |
| nnue-pytorch | TensorBoardとCSV | `do_plots.py`が損失2段とエポックごとのEloを縦3段にする。手で実行する | masterとの対局をordoで集計 | エポックの番号 |
| Bulletとその利用エンジン | 標準出力と`log.txt` | なし | OpenBenchのSPRT | 名前で区別する。系譜は手書きの表やリリースの説明に書く |
| lc0、Leela Zero | TensorBoard（lc0） | 折れ線と点。横軸は累積の対局数。自動で更新する | 最良ネットとの対戦のEloの差の足し上げ | 通しの番号とSHA |
| KataGo | 1行に1つのJSONを書くファイルとログ | 誤差棒つきの点。横軸は時刻またはデータの行数。自動で更新する | 評価用の対局を全ネットで同時推定 | 学習量を含む名前と親のネット |
| AlphaZeroの論文 | 記載なし | 折れ線。横軸は学習のステップ数。基準を水平線で示す | 1手1秒の総当たりをBayesEloで集計 | 学習のステップ数 |
| dlshogi | テキストのログ | なし（一致率の表） | 固定の相手との100局の勝率 | 世代の番号 |
| やねうら王、BulletOu | 標準出力またはCSV | なし | 前の版やほかのソフトとの対局 | 連番のディレクトリ |
| tataraとnnue-lab | 学習1回に1つの`experiment.json` | 実験ごとのページに対局成績の表 | 対局サーバーのレーティングとSPRT | 識別子と系譜の欄 |

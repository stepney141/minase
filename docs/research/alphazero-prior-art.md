# AlphaZero型の深層強化学習の先行実装調査

本書は、minaseへMCTS（モンテカルロ木探索）と方策・価値ネットワークによるAlphaZero型の探索と学習を導入するために、学習アルゴリズム、探索の実装、推論ランタイム、および複数の探索方式の共存方法を一次資料で調べた調査メモである。
設計判断は [plans/alphazero.md](../plans/alphazero.md) が定め、本書はその根拠を保持する。
調査日は2026年9月24日で、参照したソースの版は次のとおりである。
lc0は1227b4c（2026年9月19日）、DeepLearningShogi（以下dlshogi）は559d636（2026年9月23日）、ortはmasterのe8c3ce0である。
出典のない見積もりと設計上の含意には「推論」と付ける。

## 結論

第1に、1枚の消費者向けGPUでゼロから学習してminaseのαβ探索を超えることは、計算量の面で現実的でない。
AlphaZeroの将棋は自己対局に第1世代TPUを5,000基使い、将棋でのAlphaZero追試であるAobaZeroは同じ規模の学習をデスクトップ1台で100年近いと見積もっている。
第2に、将棋での先行2例が現実的な道筋を示している。
dlshogiはαβエンジンの生成局面による教師あり学習で初期化してから強化学習へ進み、技巧とGumbel dlshogiは1手16〜64シミュレーションのGumbel AlphaZeroで学習を安定させた。
第3に、探索の実装はlc0とdlshogiの構成がそのまま参考になり、Rustからの推論はortのload-dynamic機能を使えばビルド時にGPUライブラリを必要としない。
第4に、複数の探索方式の共存は、lc0が2025年に導入した「起動時に探索を1つ選び、選んだ探索のオプションだけを公開する」方式が、minaseの規模では閉じた列挙型で再現できる。
中将棋にAlphaZero型やNNUEを適用した公開例は見つからなかった。

## 学習アルゴリズム

### AlphaZero

AlphaZeroは、学習中に1手800シミュレーションの探索を行い、訪問回数の分布を方策の目標、対局結果を価値の目標として、19個の残差ブロックと256チャネルのネットワークを学習した（[arXiv:1712.01815](https://arxiv.org/abs/1712.01815) Table S3）。
将棋では2,400万局の自己対局、ミニバッチ4,096で70万ステップの更新を行い、約2時間でelmoを上回った。
方策出力は9×9×139の平面で、平坦な分布でも最終結果はほぼ同じで学習がやや遅いだけだったと報告されている。
ルートの事前確率にはディリクレ雑音を混合率0.25で加え、αは典型的な合法手数に反比例させて将棋では0.15とした。
PUCTの係数は C(s) = log((1+N(s)+c_base)/c_base) + c_init で、擬似コードの値はc_base=19652、c_init=1.25である（[Science版プレプリント](https://discovery.ucl.ac.uk/id/eprint/10069050/1/alphazero_preprint.pdf)）。
AlphaGo Zeroの55%ゲーティングは廃止され、単一のネットワークを連続的に更新した。

### Leela Chess Zero

lc0は価値の出力を勝ち・引き分け・負けの3分類（WDL）にし、残り手数を予測するヘッド（moves-left head、MLH）を加えた（[Project History](https://lczero.org/dev/wiki/project-history/)、[v0.25](https://lczero.org/blog/2020/05/lc0-v0.25-has-been-released/)）。
MLHは、勝っている側に早く勝つ手を、負けている側に長引く手を選ばせ、自己対局での引き延ばしを減らす。
学習側では、相関の強い同一対局の局面を避けるために旧TensorFlow版が記録の31/32を捨て、価値目標を探索値qと対局結果zの混合 q·q_ratio + z·(1−q_ratio) にできる（[train.py](https://github.com/LeelaChessZero/lczero-training/blob/master/tf/train.py)、[tfprocess.py](https://github.com/LeelaChessZero/lczero-training/blob/master/tf/tfprocess.py)）。
rescorerの「deblunder」は、温度サンプリングで指した悪手より前の局面の勝敗目標を最善手の値で置き換え、悪手が価値目標を汚すのを防ぐ（[rescorer.cc](https://github.com/LeelaChessZero/lc0/blob/master/src/trainingdata/rescorer.cc)）。
T80は前世代のデータで部分的に事前学習してから開始しており、ゼロからの学習を毎回繰り返してはいない（[training_runs](https://training.lczero.org/training_runs)）。

### KataGo

KataGoは、ELF OpenGoの約74 GPU年に対し約1.4 GPU年でELFの最終モデルを上回り、ドメインに依存しない効率化を定量化した（[arXiv:1902.10565](https://arxiv.org/abs/1902.10565)）。
2日間のアブレーションでの加速率は、プレイアウト上限のランダム化が1.37倍、強制プレイアウトと方策目標の刈り込みが1.25倍、大域プーリングが1.60倍、相手の次の手の予測が1.30倍であった。
プレイアウト上限のランダム化（playout cap randomization、PCR）は、25%の手番だけ上限の大きい完全探索を行って学習に記録し、残りは少ない探索で対局を進める方式である。
ネットワークは6ブロック96チャネルから始め、学習の進行に合わせて20ブロック256チャネルまで段階的に拡大した。
論文後の改良として、方策目標を1/4乗したソフト方策の補助目標と、数手先の探索値の指数平均を予測する短期価値目標が追加されている（[KataGoMethods.md](https://github.com/lightvector/KataGo/blob/master/docs/KataGoMethods.md)）。
所有権とスコアの目標は囲碁に固有であり、中将棋へはそのまま移せない（推論）。

### Gumbel AlphaZero

Gumbel AlphaZeroは、ルートでGumbel-Top-k法により重複なしにm個の手を抽出し、Sequential Halvingで予算を配分する（[Danihelka博士論文 第5章](https://discovery.ucl.ac.uk/id/eprint/10167022/2/ivo_danihelka_thesis.pdf)、[OpenReview](https://openreview.net/forum?id=bERaNdoegnO)）。
方策の目標は、未訪問手のQを価値の推定で補完した「completed Q」を使い、softmax(logits + σ(completedQ)) とする。
σ(q) = (c_visit + max_b N(b))·c_scale·q で、c_visit=50、c_scale=1.0ならシミュレーション数を変えても同じハイパーパラメータで済む。
9路盤囲碁では、通常のMuZeroが学習時16シミュレーション以下で学習に失敗したのに対し、Gumbel MuZeroは2シミュレーションでも安定して学習した。
公式実装は [google-deepmind/mctx](https://github.com/deepmind/mctx) である。

将棋では2つの適用例がある。
技巧（WCSC34）は、既存の棋譜を使わずGumbel AlphaZeroで学習し、1手16〜64シミュレーションでも比較的安定して学習できたと述べている（[アピール文書](https://www.apply.computer-shogi.org/wcsc34/appeal/Gikou/gikou_appeal_detail.pdf)）。
技巧の実装はRustの探索とPythonの学習（PyTorch Lightning、Torch-TensorRT）の組合せである。
Gumbel dlshogi（2025年7月）は、1サイクル100万局面の生成と学習を交互に行い、シミュレーション数を16、32、64、学習窓を1、2、4サイクルと段階的に増やして、106サイクルを約3日22時間で回した（[途中経過](https://tadaoyamaoka.hatenablog.com/entry/2025/07/25/230204)、[スケジューラ](https://tadaoyamaoka.hatenablog.com/entry/2025/07/22/212555)）。
学習初期には最大手数で終わる入玉宣言勝ちが66.1%を占める退化戦略が生じ、最大手数で終わった対局を学習から除いて対処した。

中将棋の合法手は1局面で100手を超えることが多く、800訪問の通常のPUCTでもルートの子を訪問し尽くせない。
これはGumbel論文が少数シミュレーションでのAlphaZeroの失敗条件として挙げる状況であり、Gumbel方式を自己対局に採る理由になる（推論）。

### dlshogiの初期化と強化学習

dlshogiは、elmoが深さ8で生成した4.9億局面で教師あり学習してから強化学習へ進み、資源不足のためスクラッチからは学習しないと明言している（[WCSC29アピール](https://www.apply.computer-shogi.org/wcsc29/appeal/dlshogi/dlshogi_appeal_wcsc29.pdf)）。
強化学習は1サイクル250万〜500万局面を生成して直近10サイクル分で学習する方式で、elmoのデータだけで収束させたモデルより有意に強くなった。
2020年には既存の将棋プログラムを1/8の割合で対局相手に混ぜるリーグ方式を採った（[WCSC31アピール](https://www.apply.computer-shogi.org/wcsc31/appeal/dlshogi_with_GCT/dlshogi_with_GCT_appeal_wcsc31.pdf)）。
損失は `方策の交差エントロピー + (1−λ)·BCE(価値, 勝敗) + λ·BCE(価値, 探索値の勝率)` で、λの既定は0.333である（[train.py](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/master/dlshogi/train.py) 46行、329〜339行）。
ネットワークは10ブロック192チャネルから始め、大会ごとに15ブロック224チャネル、20ブロック、30ブロック384チャネルへ拡大した。
入力には駒の配置に加えて駒の利きと利き数を入れており、利きの特徴が学習時間の短縮に有効だったと報告している。
方策目標は当初は指し手だけ（one-hot）で、後にルートの訪問分布へ移った（[WCSC33アピール](https://www.apply.computer-shogi.org/wcsc33/appeal/dlshogi_with_HEROZ/dlshogi_with_HEROZ_appeal_wcsc33.pdf)）。
ふかうら王のwikiは、2025年12月時点ではDL系で教師を生成するのはNNUE系より費用対効果が悪いかもしれないと述べている（[ふかうら王の学習](https://github.com/yaneurao/YaneuraOu/wiki/%E3%81%B5%E3%81%8B%E3%81%86%E3%82%89%E7%8E%8B%E3%81%AE%E5%AD%A6%E7%BF%92)）。

### 教師あり初期化の効果と限界

教師あり初期化は計算量を大きく節約するが、教師の弱点を引き継ぐ。
AlphaGo Zeroでは人間棋譜だけで学習したネットワークが初期性能で上回ったが、自己学習版が24時間以内に上回った（[AGZプレプリント](https://discovery.ucl.ac.uk/id/eprint/10045895/1/agz_unformatted_nature.pdf)）。
この比較は教師あり学習だけの版とゼロからの強化学習の比較であり、教師あり学習で初期化してから強化学習へ進む構成の比較ではない。
CrazyAraFishはStockfishの自己対局12万局で学習したが、Stockfish 10には3勝1分6敗であった（[CrazyAra](https://pmc.ncbi.nlm.nih.gov/articles/PMC7861260/)）。
dlshogiはWCSC31で特定の開始局面集に偏った学習のために振り飛車を弱点とした（[結果報告](https://tadaoyamaoka.hatenablog.com/entry/2021/05/04/225928)）。
したがって、教師あり学習は強化学習の出発点として使い、教師あり学習だけの版の強さを到達目標にしない（推論）。

## 探索の実装

### lc0

lc0の辺は4バイトで、16ビットの指し手と11ビット仮数・5ビット指数に圧縮した事前確率を持ち、子ノードは訪問した時点で生成する（`src/search/classic/node.h` 85〜112行、288〜354行）。
仮想損失は評価待ちの訪問数 `n_in_flight_` で表し、PUCTの分母に加える（`node.h` 167行）。
`PickNodesToExtendTask` は、最良子が次点に逆転されるまでの推定訪問数を一度に割り当てて木の下へ配り、キャッシュに当たったノードと終端ノードはNNの計算を待たずに逆伝播する（`search.cc` 1573〜1919行）。
`TwoFoldDraws` は探索木の内側で閉じる2回目の出現を引き分けの終端とし、木を再利用して周期が根をまたいだ場合は終端を解除する（`search.cc` 1532〜1571行、1958〜1972行）。
DAG版は局面を表す `LowNode` を複数の辺が共有し、経路に依存する反復の情報を辺の側に持たせる（`dag_classic/node.h` 958〜959行）。
NNキャッシュは200万局面で、合法手の数が一致しなければ衝突とみなす（`memcache.cc` 117〜127行）。

### dlshogi

dlshogiの探索木には置換表がなく、NN推論の結果を256シャードのLRUキャッシュに保存する（`usi/UctSearch.cpp` 1505行、`PolicyValueCache.h` 42〜134行）。
GPUごとに探索スレッドを2本置き、各スレッドが128件をまとめて推論することで、CPUの木の操作とGPUの計算を重ねる（`usi.cpp` 95行、111行、`UctSearch.cpp` 1326〜1383行）。
TensorRTはスレッドごとに最適化プロファイルとCUDAストリームを持ち、入力をビット単位にパックして転送する（`nn_tensorrt.cpp` 242〜324行、`usi/unpack.cu` 14〜50行）。
千日手は新しい子ノードを作った直後に直近16手の中の初回の再現で判定し、引き分けには先手用と後手用で別の値を使う（`UctSearch.cpp` 1487〜1525行）。
葉ではNN評価の前に奇数手詰めを調べ、根ではUCTと並行してdf-pnの詰み探索を走らせる（`UctSearch.cpp` 1527〜1554行、`main.cpp` 541〜588行）。
自己対局の生成器は、GPUあたり「スレッド数×バッチ幅」局を同時に進め、1局の内部では仮想損失を使わない（`selfplay/self_play.cpp` 499〜638行）。
データ形式hcpe3は対局単位の可変長で、手ごとに指した手、評価値、および候補ごとの訪問数を持ち、Python側の拡張モジュールが開始局面から手を再生して局面を復元する（`cppshogi.h` 79〜157行、`python_module.cpp` 379〜530行）。

### 方策の符号化

AlphaZeroの将棋は移動元の升ごとに139種類の移動を並べた9×9×139平面、dlshogiは移動方向20種と駒打ち7種を移動先の81升に掛けた2,187ラベルである（`cppshogi.h` 46〜53行）。
dlshogiの「方向だけ」の方式は、獅子、麒麟、鳳凰、角鷹、および飛鷲が距離2へ跳ぶので中将棋では単射にならない（推論）。
HaChuは獅子の2歩手を「第1歩方向×第2歩方向」の64通りで符号化する（`hachu-debian/hachu.c` 1090〜1096行、1487〜1514行）。
minaseの `Move { from, mid, to, promote }` は1つの結果に1つの表現を対応させており（[plans/move-canonicalization.md](../plans/move-canonicalization.md)）、方策のラベルを着手の値だけから単射に計算できる。

移動元基準の平面で数えると、8方向×距離1〜11の直進88面、その成り88面、獅子の桂馬型の跳び8面、第1段で相手駒を取る2歩56面、居喰い8面、およびじっと1面の計249面になる。
249面×144升＝35,856出力のうち、盤上で有効な要素は16,108である（調査用スクリプトで数え、手計算とも一致した）。
2歩系の64面はHaChuの64通りと同型である。
移動先基準で距離3以上をまとめる案は121面（17,424出力）に縮むが、ラベルから着手を復元するのに盤面が要る。
第1段と第2段を分けて出力する分解案は、中間節点で付け喰い（RULES.md第16条）を含む合法性を保証する必要があり、実装の負担が大きい（推論）。

## 推論ランタイム

ortはONNX Runtimeのバインディングで、最新は2.0.0-rc.13（2026年7月28日）、ライセンスはMIT OR Apache-2.0である（[リリース](https://github.com/pykeio/ort/releases/tag/v2.0.0-rc.13)）。
rc.13はONNX Runtime 1.28に対応し、公式文書はCUDA 13.2以上とcuDNN 9.23以上を要件とする（`docs/content/perf/execution-providers.mdx` 212行）。
`load-dynamic` 機能はリンクを無効にし、実行時に `ORT_DYLIB_PATH` の共有ライブラリを読み込むので、ビルド時にCUDA、cuDNN、TensorRT、ONNX Runtimeのいずれも要らない（`Cargo.toml` 70行、`docs/content/setup/linking.mdx` 24〜35行）。
TensorRT実行プロバイダはFP16、エンジンキャッシュ、可変バッチの最適化プロファイルを設定できる（`src/ep/tensorrt.rs`）。
ortのmasterのMSRVは1.92だが（`Cargo.toml` 29行）、固定する候補のrc.13のタグでは `ort` と `ort-sys` がどちらも `rust-version = "1.88"` を宣言しており（[ort](https://github.com/pykeio/ort/blob/v2.0.0-rc.13/Cargo.toml#L27)、[ort-sys](https://github.com/pykeio/ort/blob/v2.0.0-rc.13/ort-sys/Cargo.toml#L6)）、minaseの `rust-version = "1.88"` と一致する。
Cargoの `[lints]` は現在のパッケージにだけ適用されるので、ortの安全なAPIだけを使えばminaseの `unsafe_code = "forbid"` と両立する（[Cargo reference](https://doc.rust-lang.org/cargo/reference/manifest.html#the-lints-section)）。

他の候補は、ビルドの負担か成熟度で劣る。
tch 0.26はlibtorch 2.13とC++のビルドが要り、candle 0.11はビルドにnvccが要り、burn 0.21は畳み込みの性能の根拠が見つからずAPIの破壊的変更が続いている。
TensorRTの直接バインディングには保守されている安全な候補がない。
KataGoのREADMEはNVIDIA GPUではTensorRTが通常最速でCUDAとcuDNNは僅差と述べ、RustのQuoridorエンジンClaustrophobiaの計測ではTensorRTのFP16がtchのFP32の7.0〜7.5倍であった（[Claustrophobia](https://github.com/Plaaasma/Claustrophobia) `docs/trt_probe.md`）。
いずれの計測もRTX 3060 Ti上のものではない。

学習側はPyTorchで学習してONNXへ出力する経路が、dlshogiと技巧で実運用されている。
lczero-trainingはTensorFlowでlc0固有の形式に結びついており、移植の元には向かない（推論）。

## 探索方式の共存

lc0はv0.32.0（2025年7月）で、複数の探索アルゴリズムを共存させる探索APIを導入した（`changelog.txt` 83〜105行）。
`SearchFactory` が名前、オプションの登録、および探索本体の生成を担い、探索本体 `SearchBase` は `SetPosition`、非ブロッキングの `StartSearch`、`StopSearch` などを持つ（`src/search/search.h` 46〜102行）。
登録済みの探索は `classic`、`dag-preview`、`policyhead`、および `valuehead` の4つで、静的初期化時にマクロで登録される（`search/register.h` 40〜67行）。
探索はUCIオプションでは切り替えられず、コマンドラインのサブコマンド、ビルド時の既定、実行ファイル名の順に起動時に1回だけ決まる（`main.cc` 45〜76行）。
選ばれたファクトリのオプションだけを公開するので、選ばれなかった探索のオプションは表に出ない（`engine_loop.cc` 47〜58行）。
自己対局は登録機構を通らず、`classic::Search` を直接使う（`selfplay/game.h` 34〜35行、104〜110行）。
NNの実行系は `Backend` という抽象の背後にあり、cuda、onnx、blasのほかに、試験用のmockやrandomのバックエンドを持つ（`neural/backend.h` 43〜125行）。

Rustには静的初期化による登録がないので、minaseでは探索の集合を閉じた列挙型にし、起動引数で1つを選べば同じ構造になる（推論）。
lc0の `policyhead` 探索（方策の最大の手を探索なしで返す）は、学習したネットワークの素の強さを測る診断として有用である（推論）。

## 計算量の見積もり

以下は推論による試算であり、実測で置き換える必要がある。
10ブロック128チャネルのネットワークは12×12盤で1評価あたり約0.9 GFLOP、20ブロック256チャネルでは約6.8 GFLOPである。
RTX 3060 TiのTensorRT FP16の実効性能を10〜20 TFLOPSと置くと、10ブロック128チャネルで毎秒1万〜2万評価になる。
1局400手、毎秒1.5万評価とすると、1手32シミュレーションのGumbel方式で1日約10万局（学習局面約4,000万）、固定800シミュレーションで1日約4,000局（学習局面約160万）となる。
32〜64シミュレーションではGPUよりCPU側の木の操作と合法手生成が律速になり得る。
比較として、minaseの既存の自己対局生成器は10万ノードのαβ探索で毎秒219局面を探索した（`data/gen1-s500000.log`、合法手生成の高速化の前の値）。

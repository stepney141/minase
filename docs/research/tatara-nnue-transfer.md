# 将棋のNNUE学習器tataraからminaseへ移せる技法

本書のminaseに関する記述は2026年11月3日時点のものである。
「試す順序」の節のうち、第1段階のPairwise、学習だけで済む第2段階の3件のうち第1層の初期値のfan-in則への縮小、および第3段階の幅256と隠れ層は、2026年11月4日に[NNUEに隠れ層を加えて第1層を広げ、整数化の精度を上げる設計書](../plans/archived/nnue-architecture-revision.md)で、幅128の基準に`H1`を取らないまま世代反復で使う構成として確定した。
第2段階の残る2件のうち、一定のλ=0.75は、[NNUEの世代反復を手書きの評価関数v0からやり直す設計書](../plans/archived/nnue-generation-reset.md)の第1世代が第0世代に`H0`となったので、2026年11月5日に対象から外れた。
非線形の経路の倍率の調整、第4段階の進行度bucket、および第5段階のRangerとMSE型の損失には、まだ設計書がない。

本書は、将棋のNNUE学習器tataraが採用しているアーキテクチャと学習の技法のうち、中将棋エンジンminaseへ移すと棋力を上げ、駒の配置の偏りを減らせる見込みがあるものを、ソース、コミット、PR、および公開された対局記録から調べた調査メモである。
資料は2026年11月3日に参照した。

NNUE（efficiently updatable neural network）は、第1層の出力を着手ごとに差分で更新できる小さなニューラルネット評価関数である。
tataraは、RustのホストとCUDAのカーネルで将棋のNNUEを学習するツールであり、学習したネットは同じ作者の将棋エンジンrshogiの評価関数として使う。
tataraの用語のFT（feature transformer）はminaseの第1層にあたり、L1、L2、L3はその後段の全結合層である。
minaseのNNUEより前の評価関数は、駒と升ごとの点数の表PST（piece-square table）と、2駒の組の関係を補う補正項FM（Factorization Machine）の和であった。
λは教師値に占める探索値の割合であり、minaseの現行は1.0で対局結果を混ぜない。
STCとLTCは[測定の手引き](../guides/sprt.md)の短時間条件（持ち時間10秒、1手0.1秒加算）と長時間条件（持ち時間60秒、1手0.2秒加算）であり、この順に判定する採否の手順を段階ゲートと呼ぶ。
SPSAは、対局の結果から少数の係数を確率的に調整する手法である。
配置の偏りとは、現行のNNUEで指すと20〜99手目の奔王が85.8%の手数で自陣の最下段から2段以内にいるなど、人間の上位者、既存のエンジンHaChu、および強化学習でゼロから学習したAlphaZero型のネットc0032の3者がそろって示さない配置が複数の駒に現れることである（[集計](../measurements/adhoc/piece-placement-census.md)）。
nEloは先後を入れ替えた2局の組の結果から求めた正規化Eloであり、tataraとrshogiの記録はこの単位である。
Eloはチェス由来のロジスティックなEloであり、tanuki-の開発者nodchip氏の記事とStockfishの再計算はこの単位である。
2つの単位は換算しない。
出典のない見積もりと設計上の含意には「推論」と付ける。
数値の出典の一覧は[付録](tatara-nnue-transfer/sources.md)にある。

## 結論

第1に、tataraの周辺で対局による効果の記録がある技法は少ない。
記録があるのは、活性化のPairwise（SCReLUに対して+29.5 ±13.2 nElo）、第1層の初期値の縮小（+31.17 ±13.79 nElo）、局面進行度によるbucket（王の位置によるbucketに対して+30.72 ±12.10 Elo）、および特徴分解（+8 ±10 nElo）である。
norm loss、勝率モデル損失の一般化、L2とL3のshared項、dense QAT、チェックポイントの平均、および学習率の減衰率は、PRやissueに対局の結果が記録されていない。

第2に、tataraと将棋の構造の利得は、教師の規模を確認できたものはいずれも約80億局面以上の上で測られている。
minaseは約8,000万局面と幅128で学習しており、幅16の隠れ層を足すと訓練損失だけが下がって検証損失は下がらなかった（[ADR](../adr/archive/keep-the-direct-screlu-head-after-the-hidden-layer-candidates-did-not-lower-validation-loss.md)）。
したがって転移の見込みは、データの規模に依存しない技法かどうかで分かれる（推論）。

第3に、tataraにも周辺の記録にも、駒の配置の偏りを測った記録はない。
偏りへの見込みはすべて仕組みからの推論であり、偏りに働きかける仕組みを持つのは対局結果を教師へ混ぜるλ、非線形の経路の倍率、および進行度bucketの3つだけである。
活性化、初期値、最適化器、および幅には、偏りを減らす仕組みが見当たらない（推論）。

第4に、最初に試す候補はPairwiseである。
出典に対局の記録があり、規模に依存せず、推論費用が現行以下で済む候補はこれだけである（推論）。
隠れ層とbucketの候補は検証損失で見送られたまま対局で測られていないので、測る意味があるが、隠れ層の費用は速度の低下で約50 Eloに相当する（[速度の記録](../measurements/nnue-hidden-layer/nnue-hidden-layer-bench.md)と[持ち時間の感度](../measurements/evaluation-gen1/sensitivity-time2x.md)からの換算。線形性は確かめていない）。

次の表は、試す順に候補を並べたものである。
見込みの2列はすべて推論である。

| 順 | 候補 | 出典の対局記録 | minaseでの費用 | 棋力の見込み | 偏りの見込み |
|---|---|---|---|---|---|
| 1 | Pairwise（第1層の出力の要素積）を直接内積へ入れる | SCReLUに+29.5 ±13.2 nElo（隠れ層つき、2,677局） | 推論側と学習器を合わせて1〜2日。推論費用は現行以下 | 正。隠れ層なしでの効果は未知 | 見込まない |
| 2 | 対局結果を教師へ混ぜる（λ<1） | 単独の効果の記録なし。Stockfishの現行の設定は1→0.7→1 | 一定値なら変更なし。学習約30分 | 不明。対局結果を丸暗記する危険がある | 効く可能性あり。向きは駒によって割れる |
| 3 | 非線形の経路の倍率（FV_SCALEにあたる）をSPSAで調整する | tataraに記録なし。minaseのFMでは倍率の調整で−8 Eloから約+120 Eloへ | SPSA約8時間 | 正 | 選好を一様に弱める可能性あり |
| 4 | 第1層の初期値をfan-in則まで小さくする | +31.17 ±13.79 nElo（1,220ペア） | 定数1つ。学習約30分 | 不明。SCReLUでは勾配が消える懸念がある | 見込まない |
| 5 | 既存の隠れ層と幅256を固定ノードで診断し、対局で測る | Stockfishの第2層の拡大は長時間で+2.97 Elo、将棋の幅2048は−39.99 ±16.97 Elo | コード変更はほぼなし。速度の損失は−27〜−54 Elo相当 | データの規模に左右される | 見込まない |
| 6 | 局面進行度でbucketを切り替える | 王の位置のbucketに+30.72 ±12.10 Elo | 3〜5日。直接内積なら推論費用はほぼ増えない | 正の可能性 | 序盤の重みを分けることで効く可能性あり |
| 7 | RangerとMSE型の損失 | 単独の比較なし。SGDで−188.75 Elo、交差エントロピー型で−56.52 Elo | 学習器だけで各1〜2日 | 不明 | 見込まない |

この順序の根拠と、試さない候補の理由は「試す順序」の節に書く。

## 本書の順序の見直し

2026年11月3日に開発者は、教師の生成の前にアーキテクチャを見直し、隠れ層を追加する前提で計画すると決めた。
本書の順序は[構成の見直しの設計書](../plans/archived/nnue-architecture-revision.md)で置き換えられた。
同日の再調査で、本書の次の記述に修正が要ると分かった。

- 「容量より教師の対局数が先に尽きる」という読みの根拠（S1の検証損失、間引きの比較、およびHalfKP-hm-b8の2回の敗北）は、いずれも翌日に廃止された先読み教師での結果である。先読みを外すと最良エポックは2から17へ移るので、現行の教師での容量の限界を示さない。また、HalfKP-hm-b8の敗北は入力の細分化の結果であり、幅の拡大の反証にならない（推論）。
- 「累算値はi16の範囲に収まらない」は、切り詰めの±8から導いた形式上の上限である。実際の重みの最大は実数で1.37〜1.85であり（[集計](../measurements/adhoc/nnue-first-layer-quantization.md)）、値域を±220/255へ狭めればi16に収まる。
- minaseのSCReLUは尺度127と`c²>>7`であるが、tataraのSCReLUは尺度255と`x²>>9`である（`crates/nnue-format/src/simple_weights.rs` 36〜39行）。この差は本書の表に入れていなかった。量子化誤差の主因は第1層の丸めであり、尺度を上げると減る（同じ集計）。
- Pairwiseの+29.5 nEloは活性化だけの効果ではない。tataraの`simple`ではSCReLUとPairwiseで後段の活性化と第1層の尺度も異なる。また、推論費用が「現行以下」という見込みは、第1層の更新の費用を含まない。
- 速度の低下をEloへ換算した値は、単体の時間と別世代の持ち時間感度からの目安であり、採否の根拠には使えない。
- 学習の所要時間は「約30分」ではない。先読みなしの幅128の学習の実績は約2時間である（[記録](../measurements/nnue-material-linear-init/nnue-material-linear-init-training.md)）。
- 量子化を考慮した学習の節の「対象がない」は、隠れ層を持つ構成では当たらない。

線形の経路について補足する。
tataraの`--psqt`は隠れ層の経路と同じ1つの損失から勾配を受け、線形の経路だけの教師や学習後の補正を持たない（`crates/gpu-kernels/src/layerstack/psqt.rs` 11〜30行）。
[結果回帰](../plans/archived/nnue-outcome-regression.md)のように線形の経路を学習後に対局結果へ回帰する方式は、tatara、Stockfish、およびnnue-pytorchには見当たらない。

## 調べた版

本文のtataraのパスは、特に断らない限り次の表の版である。
短く書いたパスの正式な位置は、`cli.rs`と`eval_sfen/src/main.rs`が`bins/`の下（`bins/nnue_train/src/cli.rs`と`bins/eval_sfen/src/main.rs`）、`layerstack_weights.rs`が`crates/nnue-format/src/`、`optimizer.rs`と`init.rs`が`crates/nnue-train/src/`、`progress_kpabs.rs`、`kingrank9.rs`、`effect_bucket.rs`、および`threat.rs`が`crates/shogi-features/src/`、`quickstart`と`*.ja.md`と`dense-qat.md`と`arch-string.md`が`docs/`（`README.ja.md`は最上位）である。
Stockfishの`nnue_architecture.h`、`nnue_accumulator.h`、および`network.cpp`は`src/nnue/`、やねうら王の`evaluate_nnue.cpp`は`source/eval/nnue/`にある。
tataraは2026年10月22日のコミット`1046cb7`で設計記録`docs/decisions/`を削除したので、記録はその親`b76973c`の版で引く。

| 対象 | 版 | 確認の方法 |
|---|---|---|
| [tatara](https://github.com/SH11235/tatara/tree/875f39dccf3c09061af93337dce7fbd21178dc5b) | `875f39d`（2026年11月2日） | 手元の複製、PR、issue、およびリリースノート |
| [tataraの設計記録](https://github.com/SH11235/tatara/tree/b76973c/docs/decisions) | `b76973c` | `git show 1046cb7^:docs/decisions/...` |
| rshogi | [v1.7.0](https://github.com/SH11235/rshogi/releases/tag/v1.7.0)と[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607) | `gh`の読み取りだけ |
| bullet-shogi | [リポジトリ](https://github.com/SH11235/bullet-shogi)の`docs/` | `gh`の読み取りだけ。実験記録の`docs/experiments/`は見つからなかった |
| Stockfish | `5c4cef0b`（2026年10月13日、SFNNv18） | 手元の複製で件名と勝敗数を照合し、Eloを再計算した |
| やねうら王 | `c1b80eaa`（2026年9月14日） | 手元の複製 |
| nodchip氏の記事6本 | 2026年1月21日から4月5日 | 取得した本文。URLは付録にある |

[将棋の教師生成と学習ツールの調査](teacher-generation-prior-art.md)は、版`536beb6`のtataraの教師形式、付け直し、損失、および検証を扱っている。
本書はそれらを繰り返さず、対局の記録と、それ以降に変わった点だけを書く。
たとえば同書が引く`docs/decisions/`は現在の版にはなく、教師のシャッフルに使う窓（メモリへ読み込む区間）は2026年8月の[v0.7.1](https://github.com/SH11235/tatara/releases/tag/v0.7.1)で既定の設定から外れた。

## tataraとminaseの前提の差

転移の見込みは次の差に左右されるので、項目ごとの検討の前に示す。

| 観点 | tataraの標準 | minaseの現行 |
|---|---|---|
| 教師 | 公開データの約80億局面を2周（160億局面）。[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)の条件 | 訓練79,458,082局面、210,788局（[記録](../measurements/nnue-early-best-epoch/nnue-train-subsample.md)） |
| 第1層 | 入力73,305、幅1536で約1.1億個の重み（arch-string.md 95行） | 入力13,680、幅128、鏡映の共有で独立重み889,200個（[考察](../proposals/archived/nnue-architecture-discussion.md)） |
| 1重みあたりの局面数 | 約71局面（推論の計算） | 約89局面（同上） |
| 後段 | 16と32の隠れ層をbucket別に持つ（layerstack_weights.rs 5〜20行） | 隠れ層なしの直接内積（[ADR](../adr/archive/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-a-direct-screlu-head.md)） |
| 活性化 | CReLUの後に前半と後半の要素積（同上） | SCReLU（[head.rs](../../crates/minase/src/eval/nnue/head.rs) 61〜77行） |
| 累算値の型 | tataraは学習専用で推論部がない。Stockfishはi16（nnue_accumulator.h 44行） | i32（[accumulator.rs](../../crates/minase/src/eval/nnue/accumulator.rs) 18行） |
| 最適化器 | Ranger（β1=0.99、k=6、α=0.5。optimizer.rs 96〜129行） | AdamW（[options.rs](../../crates/minase-trainer/src/nnue/options.rs) 26〜42行） |
| 教師の混合 | 既定は探索値だけ、公開の標準レシピは対局結果を1/3（[記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187)） | 探索値だけ（[ADR](../adr/train-the-nnue-on-search-value-teachers-without-lookahead.md)） |
| 損失 | 勝率空間の二乗誤差。素のシグモイドかWRM（cli.rs 535〜541行、wrm-loss-tuning.ja.md 15〜33行） | 二値交差エントロピー（同ADR） |

1重みあたりの局面数は両者で近いが、独立な単位である対局の数はおそらく大きく違う。
minaseは1局あたり約377局面を持ち、同じ約2,000万局面でも多数の対局から疎に採るほうが検証損失が約0.0009低かった（[記録](../measurements/nnue-early-best-epoch/nnue-train-subsample.md)）。
tataraの教師の対局数は確認できなかったが、1局の局面数が将棋の手数の範囲に収まるなら数千万局の桁になり、minaseの100倍以上になる（推論）。
このため、tataraで容量を増やして得た利得は、minaseでは過学習として現れる可能性がある。

累算値の型の差は、幅を増やす費用に効く。
minaseの第1層の重みは±8へ切り詰めて量子化値を±1,016に収めており（[model/mod.rs](../../crates/minase-trainer/src/nnue/model/mod.rs) 542〜553行）、発火が最大145個なので累算値は最大約±147,000になり、i16の範囲（±32,767）に収まらない。
1命令で処理できる成分の数がi16の半分になるので、幅を増やす費用はStockfishのようなi16の累算値を使うエンジンの経験則より重いと見込まれる（推論）。

## アーキテクチャ

### 活性化（CReLU、SCReLU、Pairwise）

(a) Pairwiseは、第1層の出力の前半と後半を成分ごとに掛けて次元を半分にする活性化である（cli.rs 1383〜1388行、整数の式はeval_sfen/src/main.rs 383〜391行）。
式は`clamp(a,0,127) * clamp(b,0,127) >> 7`であり、値域はminaseのSCReLUの`(c*c) >> 7`と同じ0〜126である。

(b) 効果の記録は[tataraのv0.1.0](https://github.com/SH11235/tatara/releases/tag/v0.1.0)と[issue #165](https://github.com/SH11235/tatara/issues/165)にある。
入力halfka-hm-merged、構成`512x2-8-64`、60 superbatchの3本を比べた結果、PairwiseはSCReLUに+29.5 ±13.2 nElo（H1=+5 nEloのSPRTで採択、2,677局）だった。
1秒秒読みの総当たりでは、Pairwise、SCReLU、およびCReLUの勝率が53.5%、49.5%、および45.5%であり、1秒あたりの探索局面数は372.0K、361.0K、および359.7Kだった。
総当たりの各組の誤差は±48 nEloあり、確定しているのはPairwiseとSCReLUの差だけである。
これは隠れ層2段のネットの結果であり、隠れ層のない直接内積での記録はない。

(c) minaseのADRは、第1層の後の要素積を試していないと記している（[ADR](../adr/archive/keep-the-direct-screlu-head-after-the-hidden-layer-candidates-did-not-lower-validation-loss.md) 30行）。
実装は`Head`トレイトの新しい実装（[head.rs](../../crates/minase/src/eval/nnue/head.rs) 9〜11行）、学習器のカーネルと参照実装、および構造記述文字列の3か所である。
本書を書いた時点の層積みの設計書（後に取り下げた。`git show e2539378:docs/plans/nnue-layer-stack.md`の49行）は要素積を対象外としていたので、固定の出力部として先に試せる。
第1層の幅をHとすると、積の回数はHで現行の2Hの半分であり、内積の長さも半分になるので、推論費用は現行の17.2 ns以下に収まる（推論。[速度の記録](../measurements/nnue-hidden-layer/nnue-hidden-layer-bench.md)）。

minaseには、tataraにない注意点が1つある。
P型の第1層は成分jとj+H/2が鏡映の対であり、出力重みも対で共有している（[評価関数の手引き](../guides/evaluation-nnue.md) 230〜243行）。
jとj+H/2をそのまま掛けると、積は盤の左右の鏡映で不変になるので出力重みの対の共有が不要になる一方、掛け合わせる2本の行は独立でなくなる（代数の確認であり、測定ではない）。
別の組み方として、jとj+H/4、およびその鏡映であるj+H/2とj+3H/4を掛けると、独立な2本の行の積になるのでtataraの形に近く、鏡映の不変性は出力重みの共有で保てる（同上）。
どちらが良いかは測るまで分からないので、2本とも学習してSTCまでに1つへ絞る。

(d) 棋力への見込みは正である（推論）。
費用が下がるので棋力が下がる経路は表現の変化だけであり、STCで判定が付く大きさの差が出れば、tataraの記録と同じ向きかどうかを読める。
教師を変えない変更なので、偏りが減ると見込む理由はない。

### SimpleとLayerStack、第1層の幅と後段の次元

(a) tataraの`simple`はbucketなしの`256x2-32-32`の4層であり、`layerstack`は幅1536の第1層の後に16と32の層をbucket別に持つ（cli.rs 1160〜1234行、1365〜1388行）。
LayerStackのL1は出力の1成分をskip接続（最終出力へ直接足す経路）に使い、残りをSCReLUとCReLUの両方に通してL2へ渡す（layerstack_weights.rs 10〜20行）。

(b) [実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)では、同じ約80億局面と2周の学習で、水匠5に対してHalfKPの`256x2-32-32`が固定ノードで−83 ±15 nElo、幅1536のLayerStackが1秒秒読みで+61 ±22 nEloだった。
対局条件が違うので2つの数は直接比べられないが、[公開記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187)によると検証局面での符号一致率は0.68060から0.69854へ上がり、LayerStackは探索速度が水匠5の約0.81倍でも勝ち越した。
一方で幅を増やせば強くなるわけではなく、nodchip氏の[2026年4月5日の記事](https://nodchip.hatenablog.com/entry/2026/04/05/000000)では、幅2048は幅1536に−39.99 ±16.97 Elo（8秒＋0.08秒、1,632局）で負け、著者は探索速度の低下を理由に挙げている。
Stockfishは利き特徴を入れた後に第1層を縮め、現行は幅1024である（nnue_architecture.h 45行）。
第2層を16から32へ広げた[SFNNv13](https://github.com/official-stockfish/Stockfish/commit/a6d055d7)は短時間でほぼ0 Elo（+0.03、189,184局でSPRT棄却）、長時間で+2.97 Eloであり、SFNNv16も短時間で棄却されて長時間で採択された（[付録](tatara-nnue-transfer/sources.md)）ので、費用の重い変更は短時間の判定に落ちることがある。

(c) minaseでは、幅16の隠れ層の候補S1、S1に位相補間を加えた候補S2、および序盤用と終盤用の出力を盤上総駒数で補間する候補D2のコードが残っており（[ADR](../adr/archive/keep-the-direct-screlu-head-after-the-hidden-layer-candidates-did-not-lower-validation-loss.md)）、`ActiveHead`の変更と学び直しだけで対局にかけられる。
ただし当時の計画は検証損失で基準を下回った候補だけを段階ゲートへ出す規則だったので、これらは対局で測られていない。
出力部の費用は、S1とS2で探索時間の+18%と+19%であり、持ち時間の感度（2倍で+216 Elo。PSTの時代の値で、線形性は未確認）で換算すると約−51 Eloと約−54 Eloに相当する（推論）。
幅256は形式上は読めて（[format.rs](../../crates/minase/src/eval/nnue/format.rs) 183行）、重みあたりの局面数は約45へ半減する。
幅128のネットはPST＋FMより探索速度が約9%低く（[記録](../measurements/nnue-replacement/nnue-replacement-training.md) 164〜173行）、幅を倍にする費用はこれを上回る見込みなので、−12〜−15%（−40〜−50 Elo相当）を計画の値とする（推論）。

(d) 棋力の見込みはデータの規模次第である。
minaseでHalfKP-hm-b8の検証損失がエポック1で最良になったこと（[記録](../measurements/nnue-early-best-epoch/halfkp-retrain-stc.md)）と、S1の訓練損失だけが下がったことは、容量より教師の対局数が先に尽きることを示唆する（推論）。
ただし検証損失は棋力を予測しない。
静止探索の末端への置換は符号一致率を下げてHalfKPで+111 ±15 nEloを得ており（[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)）、minaseでも検証の対局の結果に対する損失で負けた先読みなしのネットが対局で勝っている（[ADR](../adr/train-the-nnue-on-search-value-teachers-without-lookahead.md) 26行）。
そこで固定ノードの対局で「速度を除いた評価の質」を先に診断し、速度の損失を上回る利得が見えた構成だけを段階ゲートへ出す。
容量を増やしても教師の癖を同じように学ぶので、偏りが減ると見込む理由はない。

### 局面に応じて後段を切り替えるbucket

(a) tataraのbucketは、局面ごとにL1、L2、L3の重みの組を1つ選ぶ（cli.rs 1184〜1195行、1221〜1234行）。
`kingrank9`は両者の王の段を3段刻みにした9区分である（kingrank9.rs 9〜23行）。
`progresskpabs`は、王の升と駒の組を特徴とするロジスティック回帰で局面が1局のどこまで進んだかを推定し、等分した区間へ振り分ける（progress_kpabs.rs 1〜13行）。
回帰の教師は対局の中の相対位置なので、学習データは対局順のまま渡す（progress-bin.ja.md 18〜25行）。
`progresskpabsq16`は同じ方式を整数で計算し、[PR #428](https://github.com/SH11235/tatara/pull/428)の確認では20,000局面のうち実数版とbucketが違ったのは1局面である。

(b) nodchip氏の[2026年2月4日の記事](https://nodchip.hatenablog.com/entry/2026/02/04/000000)では、幅1536で進行度の8区分は王の位置のbucketに+30.72 ±12.10 Elo（8秒＋0.08秒、3,028局、SPRT）であり、検証損失の差は0.0123と0.0122だけだった。
これは「進行度対王の位置」の比較であり、「bucketあり対なし」の比較ではない。
Stockfishの[SFNNv18](https://github.com/official-stockfish/Stockfish/commit/5c4cef0b)はbucketを8から32へ増やし、駒数と女王の有無で選ぶ（network.cpp 154〜156行）。
同コミットの入力の変更と合わせた効果は長時間で+3.60 Elo（再計算）であり、作者は出力bucketの単独の追加が最も有望だったと記している。

(c) 王の段のbucketでは、minaseのほとんどの局面が同じ区分に入る。
自玉は最下段中央の2升に全観測の63.0%があり、盤上の駒が50枚以上の局面では観測の90%が2升に集まる（[調査](../proposals/archived/nnue-input-features.md) 65〜69行）。

駒数によるbucketも、偏りが出る手数帯では働きにくい。
現行の対局で最初に駒を取るまでの手数の中央値は52手であり（[集計](../measurements/adhoc/piece-placement-census.md) 110行）、それまで駒数は変わらない。
D2の補間の係数も同じ駒数から作るので、D2が検証損失を下げなかった事実は、進行度bucketの結果を予測しない（推論）。

進行度bucketの費用は、minaseでは次の形になる（推論）。
P型の絶対位置の特徴だけで回帰すれば、進行度は線形の経路と同じ形の和を1つ増やすだけで差分更新でき、やねうら王も同じ考え方で和をキャッシュしている（evaluate_nnue.cpp 188〜280行、[コミット`9a0df6a5`](https://github.com/yaneurao/YaneuraOu/commit/9a0df6a5)）。
隠れ層のない直接内積なら、bucketで切り替わるのは出力重みの組だけなので積和の回数は変わらない。
学習器の改修は回帰の学習ツール、bucket別の出力重み、および構造記述の3点で、3〜5日を見込む。
本書を書いた時点の層積みの設計書（同じ版の49行）は、境界で重みを切り替えるbucketと進行度の推定を対象外としていた。

過去の議論は、境界で重みを切り替えるbucketを着手間の評価の不連続を理由に見送った（[考察](../proposals/archived/nnue-architecture-discussion.md) 161〜163行）。
この不連続が問題になるかは対局で測らないと分からない。
ただし直接内積をbucketに分けても第1層は共有のままなので、将棋の記録のような利得はbucket別の隠れ層から出ている可能性があり、直接内積だけの結果が負けても方式全体の否定にはならない（推論）。

(d) 棋力の見込みは正の可能性がある（推論）。
偏りにも効く可能性がある。
教師の隣接局面の70.6%は150手目以降にあるので（[調査](../proposals/archived/nnue-input-features.md) 91〜92行）、1組の出力重みは終盤の局面に強く引かれ、20〜99手目の配置の評価がその副作用である可能性を、序盤のbucketを分けることで調べられる（推論）。

### L1とL2、L3のshared項

(a) tataraのL1は、bucket共通の項とbucket別の項の和として学習し、書き出すときに畳み込むので、推論側の形式は変わらない（layerstack_weights.rs 13行、38〜39行）。
`--init-l1-shared`はその共通項の初期値を指定し、`--stack-shared-delta`は同じ分解をL2とL3へ広げる（cli.rs 803〜806行、1236〜1240行）。
どのbucketの局面でも共通項へ勾配が流れるので、bucketの数だけデータが薄まる問題を和らげる狙いである（推論）。

(b) [PR #391](https://github.com/SH11235/tatara/pull/391)と[設計記録](https://github.com/SH11235/tatara/blob/b76973c/docs/decisions/2026-07-24-layerstack-shared-delta.md)に対局の記録はなく、効果は測られていない。

(c) 共通項は学習器の中だけの変更であり、推論費用は増えない。

(d) bucketを入れる場合だけ意味を持つので、進行度bucketと同時に入れて分けて測らない（推論）。
共通項は学習の進み方を変えるだけなので、偏りへの影響は見込まない。

### PSQTのshortcut

(a) PSQT（piece-square table）のshortcutは、特徴ごとbucketごとのスカラーを後段の出力に直接足す線形の経路であり、tataraでは`--psqt`で有効にする（cli.rs 1273〜1303行）。
`--psqt-init material`は駒価値で初期化する。

(b) tataraの作者は最良のレシピではPSQTを使っていないと書いているが（[PR #303](https://github.com/SH11235/tatara/pull/303)）、比較の数値はない。
Stockfishは[SFNNv17](https://github.com/official-stockfish/Stockfish/commit/a17c0ce2)でPSQTを廃止し、非劣性の検定を短時間で+1.82 Elo、長時間で+1.91 Elo（再計算）で通した。

(c) minaseの線形の経路はこれにあたり、駒価値の初期化も既に同じである。
開発者は線形の経路を残すと決めており（[ADR](../adr/archive/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-a-direct-screlu-head.md) 39行）、本書は再提案しない。

(d) 線形の経路は[結果回帰の設計書](../plans/archived/nnue-outcome-regression.md)の補正先でもあるので、偏りの対策の面でも残す意味がある（推論）。

## 入力特徴

### HalfKP、HalfKA系、および特徴分解

(a) tataraの5種の入力は、王の升と駒の組に王自身を含めるかと、左右の鏡映で王の升を減らすかで分かれる（README.ja.md 97〜108行）。
特徴分解（factorizer）は、学習の間だけ王の升によらない仮想の行を足し、書き出しのときに畳み込む技法である（cli.rs 816〜848行）。

(b) [v0.1.0](https://github.com/SH11235/tatara/releases/tag/v0.1.0)の1,000局ずつの総当たりでは、上位3種の入力の勝率は50.8%、51.1%、および51.8%で区別できなかった。
特徴分解は[PR #303](https://github.com/SH11235/tatara/pull/303)で既定になり、棋力は+8 ±10 nEloで劣化なし、40 superbatchの時点では+29 Eloと学習が早まった。

(c) 仮想の行は「王の位置を外した駒と升の組」であり、これはminaseのP型そのものである。
したがってP型には特徴分解は意味を持たない。
HalfKP-hm-b8用の仮想行は、minaseの学習器に既に実装されている（[feature_set.rs](../../crates/minase/src/eval/nnue/feature_set.rs) 44行、[model/mod.rs](../../crates/minase-trainer/src/nnue/model/mod.rs) 398行）。
HalfKP-hm-b8はSTCでP型に2回大敗しており（[ADR](../adr/use-p-type-input-features-for-the-nnue.md)）、ADRは教師の量か幅が大きく変わったときだけ測り直すとしている。

(d) 本書は再提案しない。
2回の対局の棋譜で配置を集計していないので、偏りへの影響は分からない。

### threatとeffect

(a) threatは「ある駒が別の駒に利いている」関係を1つの特徴にする（threat.rs 3〜15行）。
effect bucketは、各駒の升の被攻撃数と被防御数を0、1、2以上へ切り詰め、基底の特徴の番号を4倍または9倍に細分する（effect_bucket.rs 82〜88行）。

(b) tataraの[設計記録](https://github.com/SH11235/tatara/blob/b76973c/docs/decisions/2026-06-19-threat-feature-set.md)は、threatが深さ固定では基準を上回り、秒読みでは探索速度の低下で負け越したと記している。
数値の出典である`bullet-shogi`の実験記録は公開のリポジトリで見つからなかった。
effect bucketは[PR #359](https://github.com/SH11235/tatara/pull/359)が等深さのSPRTを進行中と記すだけで、結果は見つからなかった。
Stockfishの[SFNNv10](https://github.com/official-stockfish/Stockfish/commit/8e5392d7)のthreatは長時間で+3.83 Elo（再計算）、同じコミットの別の測定で+13.9 ±1.9 Eloであり、半年の開発を経た結果である。

(c) 中将棋では利きを使う特徴の計算が大きく重くなる（推論）。
利き数の表の差分更新は探索速度を0.816倍にし、1手で利き数が変わる升の組は平均21.9である（[測定](../measurements/strength-stage9/strength-stage9-table-bench.md) 67〜86行）。
effect bucketはこの表を前提にするので約−63 Elo相当の速度低下を引き継ぎ、入力も13,680の4倍から9倍で、学習で負けたHalfKP-hm-b8の109,440に近い規模になる。
threatの次元は、将棋の216,720が「9駒種×2陣営の2乗×81升×平均の利き先」でほぼ再現できるので、同じ式で中将棋を見積もると、29駒種のままなら約1,000万、12駒種へまとめても約170万になる。
tataraはこの216,720でも幅1536で12 GBのGPUに収めるために半精度の状態を使っており（[PR #306](https://github.com/SH11235/tatara/pull/306)）、作業機は8 GBである。

(d) 候補から外す。
棋力の利得は速度の損失を埋める必要があり、[第1層の幅を縮めた先例](https://github.com/official-stockfish/Stockfish/commit/a6d055d7)のような予算の振り替えは、幅128のminaseでは余地が小さい（推論）。
利きの情報は配置の損得の手がかりになりうるが、探索値を教師にする限り偏りの出所は変わらない（推論）。

## 学習

### 最適化器（Ranger、RAdam、lookahead）

(a) RangerはRAdamの更新を6回ごとに低速の重みへ半分戻すlookaheadと組み合わせた最適化器であり、tataraの既定は`β1=0.99`である（optimizer.rs 96〜129行、cli.rs 591〜598行）。
RAdamの分母の補正とlookaheadの2つを持ち、低速の重みの分だけAdamWより状態が1本多い（[設計記録](https://github.com/SH11235/tatara/blob/b76973c/docs/decisions/2026-06-19-threat-feature-set.md)のGPUメモリの節）。

(b) tataraの中でRangerとAdamWを比べた記録はない。
[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)の同じ構成のtatara（Ranger）対BulletOu（AdamW）は+150 ±15 nEloだが、[記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187)自身が特徴分解の掛け方も違うと注記している。
nodchip氏の[2026年1月24日の記事](https://nodchip.hatenablog.com/entry/2026/01/24/000000)はSGD+Momentumで−188.75 ±38.45 Eloだが、学習率0.5と別の学習率スケジュールが混ざる。

(c) minaseの学習器はAdamWで、GPUのカーネルと参照実装を増やす必要がある。
tataraはMITなので式とカーネルの設計を参照でき、作業は1〜2日と見込む（推論）。
メモリは幅128で8 GBの16%しか使っていないので余裕がある（[記録](../measurements/nnue-replacement/nnue-replacement-training.md) 57行）。

(d) 棋力の見込みは不明である。
学習が数エポックで頭打ちになるminaseで、lookaheadの平均の効果が出るなら検証損失の最良エポックが遅れる形で観察できる（推論）。
最適化器を変えても教師は同じなので、偏りが減ると見込む理由はない。

### norm loss

(a) norm lossは、出力ニューロンごとの重みのノルムを1へ緩く引き寄せる正則化である（cli.rs 641〜657行、[設計記録](https://github.com/SH11235/tatara/blob/b76973c/docs/decisions/2026-06-02-norm-loss.md)）。

(b) 効果は測られていない。
[PR #284](https://github.com/SH11235/tatara/pull/284)の棋力検証の項目は未実施のままであり、[issue #283](https://github.com/SH11235/tatara/issues/283)にも結果はない。

(c) 実装は小さいが、minaseは第1層に重み減衰1e-4をかけており（[options.rs](../../crates/minase-trainer/src/nnue/options.rs) 36行）、役割が重なる。

(d) 過学習の抑制に効く可能性はあるものの、根拠がないので順序に入れない（推論）。
選好の大きさが変わる可能性はあるが、向きが変わると見込む理由はない。

### 損失の形（WRM、pow_exp、weight boost）

(a) WRM（win-rate model）は、教師とネットの出力を勝率へ換算してその二乗誤差を最小にする損失である（wrm-loss-tuning.ja.md 15〜33行）。
`--loss-pow-exp`、`--loss-qp-asymmetry`、および`--loss-weight-boost-w1`は誤差の冪、過大評価への罰、および決着寄りの局面の重みを変える（同69〜87行）。

(b) tataraの一般化の項目には対局の結果がなく、[PR #277](https://github.com/SH11235/tatara/pull/277)はSPRTが要ると書くだけである。
既定のオフセットと尺度はチェス向けで、tataraの`simple`の例はオフセット0で素のシグモイドへ退化させている（quickstart 36〜45行）。
対局の記録として最も近いのはnodchip氏の[2026年1月29日の記事](https://nodchip.hatenablog.com/entry/2026/01/29/000000)で、交差エントロピー型の勝率モデルは二乗誤差型に−56.52 ±19.45 Elo（1,172局）だった。
著者は理由を不明としている。

(c) minaseは二値交差エントロピーであり、以前のADRはオフセットと尺度の根拠がないのでWRMを採らなかった（[旧ADR](../adr/archive/train-the-nnue-on-search-value-lookahead-teachers.md)）。
オフセット0の素のシグモイドの二乗誤差ならこの反対理由は当たらず、新しい係数も要らない。
教師の36%は探索値2,000 cp以上の決着局面であり（[記録](../measurements/nnue-early-best-epoch/nnue-teacher-decided-positions.md)）、二乗誤差ではこの局面の勾配が小さくなる（推論）。

(d) 棋力の見込みは不明だが、学習器だけで済むので試す価値がある（推論）。
決着局面の寄与は変わるが、配置の偏りとの関係は分からない。

### WDL（対局結果）を混ぜる比率のスケジュール

(a) tataraの`--wdl`はWDL（win、draw、loss、対局結果）を教師へ混ぜる比率であり（minaseのλの逆で、`--wdl 1/3`はλ≈0.667にあたる）、線形の変化`--start-wdl`と`--end-wdl`、および余弦の周期`--wdl-cycle-delta`を選べる（training-schedule.ja.md 52〜115行、cli.rs 343〜391行）。
`--wdl-ignore-draws`は引き分けの行に探索値だけを使わせ、`--validation-wdl`は検証側のλを固定する。

(b) tataraの中に効果の比較はない。
公開の標準レシピは対局結果を1/3混ぜ（[記事](https://note.com/ramu_shogi_dev/n/ndb0d9f2f2187)）、nodchip氏の2026年2月の記事は探索値だけで学ぶ。
Stockfishの2026年9月の設定はλを1から0.7へ下げて1へ戻すが、単独の効果は分離されていない（[チェス評価関数の教師と混合比](chess-evaluation-targets.md) 59〜78行）。

(c) minaseはλ<1を、同じ対局の全局面で同じ値を取る対局結果をネットが対局ごとに覚えてしまう過学習（対局の記憶）を理由に、対局で測らずに退けた（[ADR](../adr/train-the-nnue-on-search-value-teachers-without-lookahead.md) 27行、[教訓](../lessons/game-constant-target-memorization.md)）。
旧ADRは、混合の比を変える提案は対局で測って決めると書いている（[旧ADR](../adr/archive/train-the-nnue-on-search-value-lookahead-teachers.md)の結果の節）。
`--lambda`は既存の引数なので、一定値ならコード変更は要らない。
余弦の周期には学習率と同じ形の倍率が要り、Stockfishの形なら学習の終わりにλ=1へ戻るので、対局の記憶を最後に薄める狙いが立つ（推論）。

(d) 偏りに働きかける手段としては、調べた技法の中でこの節のものが最も直接である（推論）。
他の技法は教師の作り方を変えないが、λは探索値の外の情報である対局結果を教師へ入れる。
[回帰の記録](../measurements/adhoc/nnue-placement-outcome-regression.md)では、評価値と枚数の差で条件付けても配置が対局結果を予測しており（損失の差は−0.0033、95%区間−0.0052〜−0.0015）、対局結果には探索値に現れない配置の情報がある。
ただしこれは観察データの予測であり、配置の因果的な損得を示すものではない。
ただし向きは一様でない。
同じ記録の係数は、醉象の配置の過大評価（−45.4 cp）と龍馬の配置の過小評価（+77.3 cp）を示すので、λを下げれば龍馬の偏りはむしろ強まる可能性がある。
自己対局の両者が同じ癖を持つ局面の外の情報は得られないので、共有された癖の解消までは期待できない（[先行事例の調査](self-play-bias-prior-art.md)の第3の結論）。
棋力は不明であり、汎化差（検証と訓練の結果の損失の差）を見ながら測る。

### 学習率のスケジュールと初期値

(a) tataraの学習率は既定が`8.75e-4`で1 superbatchごとに0.992倍であり、一定、線形、余弦、および1周期の形も選べる（cli.rs 269〜341行）。
第1層の初期値は既定が`uniform:fanin`（半値幅√(1/入力次元)）である（cli.rs 782〜797行、init.rs 32〜34行）。

(b) 第1層の初期値は[PR #363](https://github.com/SH11235/tatara/pull/363)に記録があり、半値幅0.01から0.0037への変更で+31.17 ±13.79 nElo（400 superbatch、1,220ペア、H1採択）だった。
同じPRは、L1からL3の初期値の変更は+5.86 ±12.43 nEloで未決着と記す。
学習率の減衰率は[PR #275](https://github.com/SH11235/tatara/pull/275)に見積もりがあるだけで、測られていない。

(c) minaseの学習率は`cosine:3:0.1`で既に検証損失から選ばれており（[ADR](../adr/anneal-the-nnue-learning-rate-with-a-cosine-schedule.md)）、tataraから足す根拠はない。
初期値は標準偏差0.05で（[model/mod.rs](../../crates/minase-trainer/src/nnue/model/mod.rs) 399〜401行）、fan-in則の0.00494の約10倍である（推論の計算）。
fan-in則では初期局面の入力和の標準偏差が約0.05になり、[教訓](../lessons/nnue-first-layer-init-saturation.md)の下限0.2を下回る。

活性化の違いが転移を難しくする。
tataraの記録はCReLUとPairwiseのネットであり、SCReLUの微分は2xなので、極端に小さい初期値では勾配も小さくなる（推論）。
そこでPairwiseの結論が出てから、1点だけ試す。

(d) 棋力の見込みは不明である（推論）。
線形の経路の初期値を変えても麒麟の選好は弱まらなかった（[ADR](../adr/archive/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-a-direct-screlu-head.md) 21行）ので、第1層の初期値にも偏りへの効果は見込まない。

### 非線形の経路の倍率（FV_SCALE）

(a) tataraのFV_SCALEは、ネットの出力を評価値（センチポーン、cp）へ換算する係数である。
READMEは、学習の尺度から導いた値は対局での最適値とは限らないので、対局で測って焼き込むよう書いている（README.ja.md 73〜74行）。
`net_fv_scale`は重みを変えずにこの値を書き換える。

(b) tataraの範囲に測定の結果はない。
実験のリリースも、28は対局で調整していない値であり伸びる余地があると書くだけである。
rshogiの[PR #1028](https://github.com/SH11235/rshogi/pull/1028)と[#1029](https://github.com/SH11235/rshogi/pull/1029)はネットの出力重みなどをSPSAで調整するツールを持つが、結果の数値はない。

(c) minaseには強い先例がある。
FMの補正は、縮小前のものが1/4倍のmasterに約67 Elo負け、同時学習で得た補正も倍率1のままでは約8 Elo負けたが、その補正の倍率をSPSAで0.597へ下げて焼き込んだ候補はLTCでmasterに約+120 Eloだった（[設計書](../plans/archived/fm-scale-spsa.md) 19行、27〜31行）。
NNUEの非線形の経路の倍率を対局で調整した記録は、確かめた範囲では見つからなかった。
手順はFMと同じで、学習も形式の変更も要らず、SPSAは約8時間である（[考察](../proposals/archived/nnue-architecture-discussion.md) 133行）。
倍率が1未満なら出力重みとバイアスへ焼き込める。

(d) 棋力の見込みは正である（推論）。
偏りにも効く可能性がある。
升ごとの評価の差は大部分を非線形の経路が担うので（[ADR](../adr/archive/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-a-direct-screlu-head.md) 45行）、倍率を下げれば配置の選好の強さそのものが弱まる（推論）。
ただし評価の順序は変わらないので、選ぶ配置が変わるかどうかは測らないと分からない。

### 教師のリスコア

(a) tataraの`--rescore-input`は、学習済みのネットの静的評価で全局面の値を付け直し、行が対応する別ファイルへ書く（cli.rs 453〜479行、[PR #411](https://github.com/SH11235/tatara/pull/411)）。

(b) tataraに効果の記録はない。
PR #411は、自分より強い大きなネットで付け直すStockfishの方式を目的に挙げている。
tanuki-の深層学習系の評価器による付け直しのR+104.3は別の評価器の結果であり、[強豪エンジンの調査](teacher-generation-strong-engines.md)が扱っている。

(c) minaseには自分より強い評価器がなく、c0032は当面の教師に使えない。
同じエンジンの大きなネットでの付け直しは、速度の制約を外して精度を上げる余地があるが、自己対局の教師の偏りをそのまま引き継ぐ（推論）。

(d) 転移の根拠がないので候補から外す。

### 局面の間引き、重複の除去、静止探索の末端、シャッフル

(a) 実験のリリースは、tataraの学習器と同じレシピで教師の加工だけを変えた6本を比べている。
重複の除去（dedup）は完全に同じ局面を除き、静止探索の置換（qsearch）は各局面を静止探索の末端へ置き換えて元の探索値を保つ。

(b) 固定300kノードの約2,000局で、HalfKPのdedupは+29 ±15 nElo、qsearchは+111 ±15 nEloであり、幅1536のLayerStackではそれぞれ−2 ±15 nEloと−5 ±15 nEloだった（[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)）。
qsearchは2つの構造でともに符号一致率を下げている。
tataraのシャッフルの窓は、高速な記憶装置では0.43〜6.41%遅くなるだけなので、v0.7.1で既定を無効にした。

(c) minaseの全体シャッフルと対局単位の検証分割はtataraより厳密である（[ADR](../adr/shuffle-the-whole-training-set-per-epoch-with-rao-sandelius-buckets.md)）。
対局の間の重複の割合は確認していない。
minaseは静止探索の末端への置換を、PSTの学習の不採用と元の局面と末端の値の差（静かな局面でも平均185.8 cp）を理由に退けているが（[旧ADR](../adr/archive/train-the-nnue-on-search-value-lookahead-teachers.md)）、この理由は成り立たなかった。その後のP型の幅128での[比較](../measurements/nnue-qsearch-leaf/nnue-qsearch-leaf-stc.md)は`H0`だった（[ADR](../adr/do-not-replace-nnue-teacher-positions-with-qsearch-leaves.md)）。
minaseの生成器は最善手が捕獲または成りの局面を既に除いており、役割が一部重なる（[調査](teacher-generation-prior-art.md)の表）。

(d) minaseのネットは隠れ層がないので、効果の大きかったHalfKPに近い可能性があるものの、隠れ層の有無が効果を分けたとは確かめられていない（推論）。
再生成か再生の工程を要するので、順序では後に置く。
局面の加工は教師の値の出所を変えないので、偏りへの効果は見込まない。

### チェックポイントの平均

(a) `ckpt_average`は、複数の学習チェックポイントの重みを一様またはEMAで平均して書き出す、CPUで動くツールである（[PR #431](https://github.com/SH11235/tatara/pull/431)）。
SWA（stochastic weight averaging）は[issue #268](https://github.com/SH11235/tatara/issues/268)で未実装のまま研究扱いである。

(b) 効果は測られていない。

(c) minaseはエポックごとのチェックポイントを残さないので、学習器の保存の変更が少し要る（[手引き](../guides/nnue-training.md) 167行）。

(d) 棋力の根拠はなく、重みの平均で偏りが減ると見込む理由もない。
上位の候補が尽きた後の低費用の試行として残す（推論）。

### 量子化の扱いとdense QAT

(a) tataraの`--qat dense`は、既存のチェックポイントからの追加学習で、L1からL3の順伝播を整数で行い、丸めの勾配を素通しする部分的な量子化考慮学習である（dense-qat.md 30〜51行）。
別に、tataraは実数の順伝播の中でPairwiseの出力へ127/128を掛け、整数推論の`>> 7`と目盛りをそろえている（layerstack_weights.rs 10行）。

(b) dense QATの棋力は測られていない（dense-qat.md 56〜57行、[PR #413](https://github.com/SH11235/tatara/pull/413)）。

(c) minaseの直接内積には量子化する隠れ層がないのでdense QATの対象がない。
一方で、実数の式は`clamp(x,0,1)²`、整数の式は`(c×c) >> 7`で目盛りが約0.8%ずれ（本書を書いた時点の層積みの設計書、`git show e2539378:docs/plans/nnue-layer-stack.md`の83行）、隠れ層の比較の基準D0の量子化誤差は平均13.61 cp、最大99.7 cpである（[記録](../measurements/nnue-hidden-layer/nnue-hidden-layer-training.md) 65行）。
127/128の係数を実数の式へ入れる変更は小さく、誤差の減少は既存の検査で確かめられる（推論）。

(d) 棋力への寄与は不明であり、非線形の経路の倍率の調整と重なる部分がある（推論）。
目盛りの補正なので、偏りへの効果は見込まない。

### held-out検証

tataraの検証の方法は[既存の調査](teacher-generation-prior-art.md)が扱っており、対局単位に分けるminaseのほうが厳密である。
1つだけ追加すると、公開の標準実験は学習と別の出所の強豪の対局856,923局面で符号一致率を測っている（[実験のリリース](https://github.com/SH11235/rshogi/releases/tag/nnue-experiments-202607)）。
minaseなら人間の棋譜をこの診断の物差しに使える（強くする手段には使わない）。
ただし符号一致率は棋力を予測しないので、採否には使わない。

## 推論の費用

出力部の費用は実測があり、他は実測からの見積もりである。
基準は深さ9のbench（評価6,400,021回、4.23秒）であり、Elo相当は持ち時間2倍で+216 Eloからの換算である（[速度の記録](../measurements/nnue-hidden-layer/nnue-hidden-layer-bench.md)、[感度の記録](../measurements/evaluation-gen1/sensitivity-time2x.md)）。
感度の値はPSTの時代の測定であり、線形性を仮定した目安にすぎないので、採否には使わない。

| 構成 | 根拠 | 出力部1回 | 探索時間 | Elo相当 |
|---|---|---:|---:|---:|
| 現行の直接内積D0（幅128） | 実測 | 17.2 ns | 基準 | 0 |
| Pairwiseの直接内積 | 見積もり。積と内積が半分 | 17.2 ns以下 | 減る | 0以上 |
| D2（2端点の補間） | 実測 | 28.4 ns | +1.7% | 約−5 |
| 進行度8区分の直接内積 | 見積もり。積和は不変で和が1つ増える | 約17 ns | 1%程度 | 約−3 |
| Pairwiseと幅16の隠れ層 | 見積もり。S1の積和の半分 | 約78 ns | 約+9% | 約−27 |
| S1（幅16の隠れ層） | 実測 | 136.5 ns | +18% | 約−51 |
| S2（S1と補間） | 実測 | 145.1 ns | +19% | 約−54 |
| 幅256 | 見積もり | 未測定 | +14〜+18% | −40〜−50 |
| 利き数の表（effect bucketの前提） | 実測（0.816倍） | 別の費用 | +23% | −63 |

`unsafe`を使わない現在の方針の下で、活性化の出力を`u8`の配列に置いて16ビットの積和を取る形にすれば`vpmaddwd`へ自動ベクトル化されることは確かめられている（[速度の記録](../measurements/nnue-hidden-layer/nnue-hidden-layer-bench.md) 28〜32行）。
Pairwiseも同じ形で書けるので同様にベクトル化される見込みであり、実装後に`objdump`で確かめる。
差分更新のスパースな経路とbucketの費用は、隠れ層を採らない限り問題にならない（推論）。
第1層の幅は形式上512まで受けるが、512では重みあたりの局面数が約22になる。

## 試す順序

本節全体が推論である。
順序は、出典に対局の測定があるか、データの規模に依存しないか、推論費用が現行以下か、実装がどれだけ小さいか、偏りに働きかける仕組みがあるかの5つで決めた。
採否はすべて[段階ゲート](../guides/sprt.md)のコミット対コミットで決め、検証損失や符号一致率だけでは見送らない。

1回の所要時間の目安は、学習が約30分（1エポック71〜78秒、[採用したネットは22エポック](../adr/train-the-nnue-on-search-value-teachers-without-lookahead.md)）、STCが上限3,000ペアで約7時間（[記録](../measurements/nnue-material-linear-init/nnue-material-linear-init-noninferiority-stc.md)の8.2秒/ペア）、LTCが1,500ペアで約11時間（[記録](../measurements/nnue-material-linear-init/nnue-material-linear-init-noninferiority-ltc.md)の26秒/ペア）である。
大きな差が出れば早く止まり、先読みを外したネットのLTCは237ペア、55分で決着した（[記録](../measurements/nnue-early-best-epoch/nnue-no-lookahead-ltc.md)）。

各候補のLTCの棋譜から、[集計](../measurements/adhoc/piece-placement-census.md)の5配置と[結果回帰の設計書](../plans/archived/nnue-outcome-regression.md)の距離を計算して記録する。
追加の対局は要らない。
偏りが減らなかったことを理由に棋力の採用を止めるかどうかは、利用者の判断である。

第1段階はPairwiseである。
成分の組み方を2通りで学習し、STCまでに1つへ絞ってLTCへ進める。
負けた場合は、隠れ層のない形では活性化の利得が出ないと読み、後段を増やしてからもう一度だけ試す。

第2段階は、学習だけで済む3件である。
まず一定のλ=0.75で学習する。
PSTで使っていた値であり、Stockfishとtataraの標準レシピの比率にも近い。
汎化差が学習前のネットより広がるかを同時に記録し、対局の記憶が見えたら余弦の周期へ切り替える。
次に、採用したネットの非線形の経路の倍率をSPSAで調整して焼き込む。
活性化と教師が決まってから調整するほうが、調整のやり直しが減るので、この順に置く。
続いて第1層の初期値をfan-in則へ下げた1点を試す。
学習の初めに飽和率と勾配を確かめ、エポック0から動かなければ中止する。

第3段階は、容量の診断である。
S1と幅256を学習し直し、固定ノードの対局で現行と比べる。
固定ノードは速度を除いた診断であり、採否の根拠には使わない（[手引き](../guides/sprt.md)）。
固定ノードで表の速度の損失を明らかに上回る利得が見えた構成だけを段階ゲートへ出す。
Stockfishの第2層の拡大は短時間で棄却されて長時間で採択されたので、費用の重い候補でSTCを省いてLTCへ直行するかどうかは、手引きの規則どおり測定の前に設計書へ記録して決める。
利得が見えなければ、容量ではなく教師が律速であると読み、[教師の間引きの計画](../plans/sparse-teacher-recording.md)と対局数を増やす計画を優先する。

第4段階は進行度bucketである。
第3段階の結果を見て、直接内積のbucketか隠れ層つきのbucketかを選び、構造記述の出力部の名前に区分を加える。
区分は8を出発点とし、共通項を同時に入れる。
王の段のbucketは、自玉の分布から採らない。
1局の前半の区分を分けることで配置の偏りが変わるかが、この段階の偏りに関する問いである。

第5段階は、RangerとMSE型の損失である。
どちらも学習器だけで済み、エンジンの費用を増やさないが、対局の根拠が間接的なので後に置く。
1本ずつ別に測り、同時に変えない。

第6段階以降の候補は、対局の間の重複の除去、127/128の係数、およびチェックポイントの平均である。
いずれも上の段階で頭打ちが見えた後に、低費用の順で試す。

試さない候補は、threatとeffect（速度の費用と入力の規模）、HalfKP系（2回の大敗とADR）、リスコア（教師にできる強い評価器がない）、norm lossとdense QAT（根拠がなく、後者は対象がない）、およびWRMの既定の係数（チェス向けの値で、中将棋での根拠がない）である。

[結果回帰の設計書](../plans/archived/nnue-outcome-regression.md)との関係も順序に影響する。
結果回帰は学習済みの非線形部分を固定して線形の経路を補正するので、第1段階か第2段階でネットが変わると測り直しになる。
結果回帰を先に行う場合は、この測り直しの費用を見込む。

## ライセンス

tatara、bullet-shogi、bullet、およびBulletOuはMITである。
rshogi、Stockfish、やねうら王、およびnnue-pytorchはGPL-3.0である。
minaseのエンジンはGPL-3.0-or-later、minase-coreとminase-trainerはMITなので、学習器へコードを写せるのはMITの資料からだけであり、GPL-3.0の資料は式と手順の記述にとどめる（[引用の教訓](../lessons/cite-quotes-with-pinned-source-and-license.md)）。
本書は第三者のコードと文章を長く引用していない。

## 未確認の事項

- nnue-lab（<https://nnue-lab.sh11235.com/t/sh11235>）の実験ページは、JavaScriptで描画されるので本文を読めなかった。学習曲線、幅とbucketの掃引、threat、factorizerの単独の結果があるとすれば、そこにある可能性がある。
- bullet-shogiの`docs/experiments/`は公開のリポジトリで見つからず、threatが深さ固定で勝って秒読みで負けたという記述の数値を確認できなかった。
- effect bucketの等深さSPRTの結果は見つからなかった。
- PairwiseのSPRTの時間条件と教師の規模は、リリースノートとissueに書かれていない。総当たりは1秒秒読みである。
- tataraの主流のレシピでの進行度bucketと`kingrank9`の比較は見つからなかった。nodchip氏の記事は別の学習器（nnue-pytorchの派生）の結果である。
- 公開の教師の対局数は確認していない。対局数の比較は推論である。
- minaseの対局の間の重複局面の割合と、幅256の探索速度は測っていない。本書の作成では学習、対局、および長時間の計算を行っていない。
- Pairwiseの組み方、一定のλ、倍率、および初期値について、minaseの偏りへの影響を測った記録はない。

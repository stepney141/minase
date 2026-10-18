# 評価値を勝率へ換算する尺度Kの決め方に関する既存実装の調査

2026年10月15日に、チェスと将棋のエンジンおよび学習器の公開ソースと開発者の説明を確認した。
対象は、評価値（センチポーン）を `sigmoid(v / K)` で勝率へ換算するときの尺度Kが、手で決めた定数か、データから推定した値か、最適化によって更新される学習対象のパラメータかという点である。
minaseの学習器は、教師の探索値を勝率へ換算する尺度（教師K）を学習のたびに訓練集合から推定している。この現状に対して「Kは再推定するものではなく、ハイパーパラメータとして与えるべきではないか」という疑問が出たので、既存実装でのKの扱いを調べた。

## 結論

調べた範囲では、ネットの重みと一緒にKを学習する実装は、チェスにも将棋にも見つからなかった。
どの実装も、学習の間はKを固定している。
決め方は次の3通りに分かれる。

1. 手で決めた定数。将棋のαβ系（やねうら王、elmo、Apery、tanuki-、技巧）は600を直書きし、チェスのNNUE学習器（bullet、CudAD、Grapheus）は設定で与える。
2. 対局データから一度推定して定数化した値。Texel法の原著者の実践、Stockfishのnnue-pytorch（340と380）、Stockfish本体のWDLモデル、dlshogiの756.09、Lc0の表示用の換算式がこれに当たる。Texel法の手順はKの推定を含むが、原著者は最初に求めた値を以後変えていない。
3. 実行ごとにデータから推定する値。Etherealのチューナーと、dlshogiの `--use_evalfix` がこれに当たる。いずれも学習の前に1回推定し、学習中は固定する。

minaseの教師Kは3番目の型であり、分類ごとに推定する点でdlshogiの `--use_evalfix`（教師ファイルごとに推定）に最も近い。
一方、ネットの出力をセンチポーンへ換算する出力Kは、[ADR](../adr/archive/fix-the-output-k-of-trained-weights.md)により2026年9月14日から1072.65の定数である。
主流の慣行は定数であり、実行ごとに推定すると、教師値の基準が訓練集合の分割や間引きの違いに左右される。
一方、minaseの教師データは世代や探索ノード数の異なる複数の生成器に由来し、記録された教師Kは分類間で758から1218まで開きがあるので、単一の定数で一律に換算すると一部の分類の教師値の尺度が歪む。
両方の要請を両立させる運用は「分類を作った時点で1回推定し、その値をデータの由来とともに記録し、以後は定数として学習器に与える」ことであり、これはTexel法の原著者の実践とdlshogiの既定と同じ型である。
分類ごとの較正を保つか単一の定数へ寄せるかは、本調査では決めない。

## minaseの現状

minaseのKは2つある。
将棋の600系統のponanza定数は、この2つの役割を1つの定数で兼ねたものに当たる（後述）。

- 出力Kは、ネットの出力（勝率のロジット）をセンチポーンへ換算する尺度である。学習の初期値として与えた重みファイルのヘッダの値を使い、データから推定し直さない（[ADR](../adr/archive/fix-the-output-k-of-trained-weights.md)）。
- 教師Kは、教師の探索値を勝率へ換算する尺度である。教師の分類ごとに、訓練集合の探索値から終局結果を予測する二値交差エントロピーを黄金分割探索で最小化して求める（`crates/minase-trainer/src/data/teacher.rs` の `estimate_k`、分類ごとの呼び出しは `crates/minase-trainer/src/data/mnsd.rs` の `estimate_teacher_ks`）。推定は学習の前に1回行い、勾配計算の対象には含まれない。

記録された教師Kは次のとおりである（単位はセンチポーン/ロジット、[先読み教師の診断](../measurements/lookahead-teacher/lookahead-teacher-diag.md)、[世代3の学習](../measurements/pst-gen3/pst-gen3-training.md)、[形式統一の照合](../measurements/file-format-consolidation/file-format-consolidation-equivalence.md)）。

| 教師の分類 | 探索値のK | 先読み教師のK |
|---|---:|---:|
| 世代0 | 804.0 | 768.5 |
| 世代1 | 1099.7 | 1068.2 |
| 世代2 | 777.6 | 758.4 |
| 世代3 | 765.95 | |
| NNUE代替計画の2万ノードの教師（2ファイル） | 1217.6 | |

世代0から世代3は1局面10万ノードの教師である。
世代1だけが大きいのは、当時は出力Kも学習ごとに再推定しており、世代1のネットの評価値の目盛りが広がっていたためである。
2万ノードの教師のKが10万ノードの教師より大きいのは、浅い探索値ほど終局結果との対応が弱いことを示し、dlshogiの作者が述べる「読みの深さによって変わる」と合う。
分類ごとに推定する仕組みは、この目盛りの違いを教師側で吸収するために導入された（[教師データの分類のADR](../adr/archive/classify-teacher-data-by-provenance-files.md)）。
学習ごとの再推定に伴う制約は、他の設計書にも現れている。
[学習率スケジュールの設計書](../plans/nnue-lr-schedule.md)は、間引きの影響だけを比較できるようにするため、間引く前の訓練集合全体から教師Kを推定する規定を置き、[out-of-coreシャッフルの設計書](../plans/out-of-core-shuffle.md)は、bucket数によらず教師Kをビット単位で一致させる条件を課している。

## 比較表

Kの表記は実装ごとに異なるので、1ロジットあたりのセンチポーンへ換算した値を併記する。
Texel法の `1 / (1 + 10^(-K s / 400))` は `400 / (K ln 10)`、Etherealの `1 / (1 + exp(-K E / 400))` は `400 / K` で換算した。

| 実装 | 適用対象 | 決め方 | K（cp/ロジット） | 備考 |
|---|---|---|---:|---|
| やねうら王 learner.cpp、elmo、Apery、tanuki-、技巧 | 教師側と出力側で同じ | 手で決めた定数 | 600 | ponanza定数。CLIに出さない |
| nnue-pytorch（Stockfish） | ネット側と教師側で別 | 一度最適化して定数化 | ネット340、教師380、オフセット270 | 2022年6月以来不変。設定ファイルに数値として直書きされ、CLIで上書き可、学習時に再推定しない |
| Stockfish本体のWDLモデル | 表示と正規化 | 一度推定して定数化、年に1回から数回再フィット | 駒数依存 | WDL_modelリポジトリが対局データから係数を求め、uci.cppへコミットする |
| bullet | 教師側と推論で同じ | 手で決めた定数 | 400が通例 | `eval_scale` を利用者が設定する。推定コードはない |
| CudAD、Grapheus（Koivisto、Berserk） | 教師側と出力側で同じ | 手で決めた定数 | 160 | ソースに直書き |
| Texel法（Österlund、2014年） | 教師側 | 一度推定して定数化 | 約154 | 手順はKの推定を含むが、原著者はK=1.13を求めて以来変えていない |
| Ethereal の tuner | 教師側 | 実行ごとに推定 | データ依存 | `computeOptimalK` をエポックの前に1回呼ぶ |
| Weiss の tuner | 教師側 | 手で決めた定数 | 約178 | 推定コードはあるがコメントアウトし、2.25に固定 |
| dlshogi（既定） | 教師側 | 一度推定して定数化 | 756.09 | 2017年にelmoの教師100万局から回帰 |
| dlshogi（`--use_evalfix`） | 教師側 | ファイルごとに推定 | ファイル依存 | 教師ファイルごとに係数aを推定し、756.09/aで評価値を補正する。既定は無効 |
| dlshogi、ふかうら王（表示） | 出力側 | 手で決め直した定数 | 756、285 | USIオプション `Eval_Coef` |
| Lc0（表示） | 出力側 | 一度推定して定数化 | 換算式は `tan` 型 | Stockfishの評価値に合わせて係数を求めた |

## 将棋の実装

### ponanza定数600の系統

やねうら王の旧学習部は、評価値を勝率へ変換する関数を次のように書く（[learner.cpp V4.89](https://github.com/yaneurao/YaneuraOu/blob/V4.89/source/learn/learner.cpp)。ローカルのクローンではコミット `0a6dd2cb^` の1006行から1012行で照合した）。

```cpp
// 評価値を勝率[0,1]に変換する関数
double winning_percentage(double value)
{
	// この600.0という定数は、ponanza定数。(ponanzaがそうしているらしいという意味で)
	// ゲームの進行度に合わせたものにしたほうがいいかも知れないけども、その効果のほどは不明。
	return sigmoid(value / 600.0);
}
```

同じ関数を深い探索値と浅い探索値の両方に使い、elmo式の混合比 `ELMO_LAMBDA = 0.33` もソースの定数である。
学習部は2026年5月のコミット `0a6dd2c` で本体から削除された。
elmo（[elmo_for_learn の usi.cpp](https://github.com/mk-takizawa/elmo_for_learn/blob/master/src/usi.cpp)）とApery（[usi.cpp](https://github.com/HiraokaTakuya/apery/blob/master/src/usi.cpp)）は「係数 600 は Ponanza で採用しているらしい値」と注記して同じ式を使う。
tanuki-のNNUE学習器は `constexpr double kPonanzaConstant = 600.0;` をネット出力のセンチポーンへの換算と探索値の勝率変換の両方に使う。つまり、minaseの出力Kと教師Kに当たる2つの役割を、この1つの定数が兼ねている（[trainer.h](https://github.com/nodchip/tanuki-/blob/master/source/eval/nnue/trainer/trainer.h)）。
技巧は `const float kGain = 1.0f / 600.0f;` の出典として山本一成氏の2015年のツイートを挙げる（[learning.cc](https://github.com/gikou-official/Gikou/blob/master/src/learning.cc)）。
いずれの学習器も、Kをコマンドライン引数や設定ファイルから指定できるようにしておらず、コード内の定数として固定している。
やねうら王の作者は、600が「ある評価値をとった局面から、同じ程度の棋力のAIプレイヤー同士で終局まで対局させて統計的に調べる」ことで得られた値だと説明し、実際はもう少し小さい値とも言われていると付記している（[機械学習エンジニアのための将棋AI開発入門その1](https://yaneuraou.yaneu.com/2020/05/03/機械学習エンジニアのための将棋ai開発入門その1/)）。
Bonanzaの学習にあるシグモイドは指し手一致損失の幅であり、勝率変換ではない。

### dlshogiの756.09と `--use_evalfix`

山岡忠夫氏は2017年5月に、elmo_for_learnの自己対局教師（深さ6の探索値と勝敗）10万局面のデータを回帰分析し、追記で100万局の再計測値 `756.0864962951762` を記した（[評価値と勝率の関係](https://tadaoyamaoka.hatenablog.com/entry/2017/05/24/220807)）。
この値はdlshogiの内部基準として `cppshogi/python_module.cpp` に直書きされ、USIオプション `Eval_Coef` の既定値756にもなっている。
2021年5月の記事は、floodgateの棋譜からレート帯ごとに係数を推定すると237から399、自己対局教師では488になることを示し、「評価値を実際の対局結果から求めた係数で補正することで、価値の精度が向上することが確かめられた」と述べる（[自己対局の評価値の補正](https://tadaoyamaoka.hatenablog.com/entry/2021/05/06/213506)）。
同年10月の記事は「評価値と勝率の関係は、使用するエンジン、読みの深さ、評価関数の精度によって、変わるものである」として、表示用の係数をUSIオプションにした（[dlshogiの評価値のスケール調整](https://tadaoyamaoka.hatenablog.com/entry/2021/10/19/230601)）。

学習側の `--use_evalfix` は、教師ファイルごとに係数を推定して基準へ換算する（[data_loader.py](https://github.com/TadaoYamaoka/DeepLearningShogi/blob/master/dlshogi/data_loader.py)）。

```python
# 評価値から勝率への変換
def score_to_value(score, a):
    return 1.0 / (1.0 + np.exp(-score / a))
...
                if use_evalfix:
                    eval, result = cppshogi.hcpe3_prepare_evalfix(path)
                    if (eval == 0).all():
                        a = 0
                        logger.info("{}, skip evalfix".format(path))
                    else:
                        popt, _ = curve_fit(score_to_value, eval, result, p0=[300.0])
                        a = popt[0]
                        logger.info("{}, a={}".format(path, a))
```

`use_evalfix` の既定は `False` であり、その場合は補正せずに読み込む。
やねうら王のwikiは「現代の将棋AIは当時の将棋AIよりずっと強いので、a = 1/100～1/300 程度となります。(教師生成時の1局面あたりの探索局面数で変わってきます。)」と説明し、`--use_evalfix` がファイルごとに係数を決めて補正すると記す。
教師ファイルごとに結果から係数を推定して換算するこの方式は、minaseが教師の分類ごとに教師Kを推定する方式と数学的に同じである。

### ふかうら王

ふかうら王は学習にdlshogiの学習器を使い、表示用の係数 `Eval_Coef` の既定値を285にしている（`source/engine/dlshogi-engine/SearchOptions.cpp`）。
wikiは、285にすると評価値の膨らみ方が水匠とだいたい同じになると説明する。
これは手で決め直した定数である。

## チェスの実装

### Texel法とその系統

Texel法の原法は、初期の評価関数に対して誤差を最小にするKを求め、以後は変えない。
作者は「K=1.13が当時の評価関数に最もよく合った。それ以来Kを変えておらず、今後も変えるつもりはない。単なる任意の尺度定数だからである」と説明している（[talkchessの説明](https://talkchess.com/viewtopic.php?t=50823&start=20)、[Chess Programming Wiki](https://www.chessprogramming.org/Texel%27s_Tuning_Method)）。
Etherealのチューナーは `K = computeOptimalK(entries);` をエポックの前に1回呼び、−10から10を刻み幅1で走査してから刻み幅を10分の1ずつ狭めて、結果と `sigmoid(K, seval)` の二乗誤差を最小にするKを求める（[tuner.c](https://github.com/AndyGrant/Ethereal/blob/master/src/tuner.c)）。
Weissのチューナーは同じ推定関数を持つがコメントアウトし、`K = 2.25;` に固定している（[tuner.c](https://github.com/TerjeKir/weiss/blob/master/src/tuner/tuner.c)）。

### Stockfishのnnue-pytorchとWDLモデル

nnue-pytorchの損失は、ネット出力と教師の探索値を別々の尺度とオフセットで勝ち確率と負け確率へ換算し、得点期待値の形で比べる（[model/nnue.py](https://github.com/official-stockfish/nnue-pytorch/blob/master/model/nnue.py)）。

```python
    # convert the network and search scores to an estimate match result
    # based on the win_rate_model, with scalings and offsets optimized
    q = (scorenet - loss_params.in_offset) / loss_params.in_scaling
    qm = (-scorenet - loss_params.in_offset) / loss_params.in_scaling
    qf = 0.5 * (1.0 + q.sigmoid() - qm.sigmoid())

    s = (score - loss_params.out_offset) / loss_params.out_scaling
    sm = (-score - loss_params.out_offset) / loss_params.out_scaling
    pf = 0.5 * (1.0 + s.sigmoid() - sm.sigmoid())
```

既定値は `in_scaling: float = 340`、`out_scaling: float = 380`、`in_offset: float = 270`、`out_offset: float = 270` であり、設定ファイル内に固定の数値として定義されている（[model/config.py](https://github.com/official-stockfish/nnue-pytorch/blob/master/model/config.py)）。
`in_` はネット出力側の換算、`out_` は教師の探索値側の換算に使う。
この値は2022年6月のコミット `b6865d5`（Refined loss function）で最適化して以来変わっておらず、最適化器の対象にも入っていない。
Fairy-Stockfish向けの派生版variant-nnue-pytorchは、2024年1月のコミット `7b85e51` でも2021年3月の形（教師410、ネット361、`nnue2score = 600`）のままである。

Stockfish本体の勝率モデルは `1 / (1 + exp((a - eval) / b))` で、aとbは駒数の3次多項式である（`src/uci.cpp` の `win_rate_params`）。
係数は[WDL_modelリポジトリ](https://github.com/official-stockfish/WDL_model)がfishtestの長時間対局から `curve_fit` で求め、`src/uci.cpp` の配列 `as[]` と `bs[]` へコミットする。
勝率モデルは2020年6月に導入され、係数は「Update the WDL model」のコミットで年に1回から数回更新されている（ローカルのクローンで2024年に5回、2025年に1回、2026年に2回を確認した）。
これは学習側のKではなく表示とセンチポーン正規化の係数だが、「データから事前に推定し、定数としてリポジトリにコミットする」型の代表的な実例である。

### NNUE学習器の定数

bulletは `TrainingSchedule` の `eval_scale` を利用者の設定で受け取り、`sigmoid(rscale * score)` で教師を作る。推定コードはなく、例では400を使う（[schedule.rs](https://github.com/jw1912/bullet/blob/main/crates/bullet_lib/src/trainer/schedule.rs)）。
CudADは `static constexpr float SigmoidScalar = 2.5 / 400;`、GrapheusはKoivistoに `2.5 / 400`、Berserkに `1.0 / 160.0` を直書きする（[CudAD](https://github.com/Luecx/CudAD/blob/main/src/archs/Koivisto.h)、[Grapheus](https://github.com/Luecx/Grapheus/blob/main/src/models/berserk.h)）。
いずれも教師側とネット側で同じ値を使い、推論の換算尺度を兼ねる。

### Lc0

Lc0は価値ヘッドの出力を `90 * tan(1.5637541897 * wl)` のような `tan` 型の式でセンチポーンへ換算し、係数はStockfishの評価値との一致で求めてコミットしてきた（[search.cc](https://github.com/LeelaChessZero/lc0/blob/master/src/search/classic/search.cc)）。
これは表示用の換算であり、学習側のKではない。

## minaseへの示唆

他エンジンの主流の慣行に倣うなら、教師Kは定数として学習器に与える。
ただし、minaseの教師データは生成器の世代と探索ノード数が異なる複数の分類からなり、記録された教師Kも分類間で大きく異なるので、単一の定数に固定すると一部の分類で教師値の尺度が歪む。
dlshogiの `--use_evalfix` は、まさにこの状況で分類ごとの推定を選び、精度の向上を報告している。
したがって、設計上の論点は分類ごとに異なる値を持たせるかどうかではなく、学習を実行するたびに再推定するかどうかである。
分類を作った時点で1回推定して記録し、以後は定数として与える方式は、Texel法の原著者の実践とdlshogiの既定と同じであり、分類ごとの較正を保ちながら、訓練集合の分割や間引きへの依存をなくす。
どちらを採るかは設計書で決める。

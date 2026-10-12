# Python学習器のRustへの移植と撤去の設計書

## 先に読む要約

本マイルストーンは、評価関数の学習に使ってきたPythonの学習ツール（`tools/train/`、パッケージ`minase_train`）を撤去し、学習をRustの学習器だけで行えるようにする。
NNUE（着手ごとに第1層の出力を差分更新できる小さなニューラルネット）の学習器は既にRustで書き直され（`crates/nnue-trainer`）、PyTorchの学習器と同じ結果を出すことを一度確かめている。
残るのは、PST（駒と升の組ごとの点数表）の学習器、FM（2駒の位置関係を表す補正の2次モデル、Factorization Machine）の学習器、PSTとFMを同時に学ぶ同時学習器、および学習の前後に使う診断である。
これらを、Rustの学習器へ現行の手順で使う機能だけ移植し、Pythonが使った局面の学習順をRustへ与えて同じ結果になることを一度だけ確かめる。
確かめた後はPythonとの互換を保たず、Pythonの学習器、PyTorchとの照合の経路、およびPythonの依存を削除する。
あわせて、学習器のクレートをNNUE専用の名前`nnue-trainer`から`minase-trainer`へ改め、`nnue`、`pst`、`fm`、`joint`、`diag`のサブコマンドにまとめる。
エンジンの評価関数と探索は変えないので、棋力の測定は行わない。
完了条件は、移植した各学習器と診断の一致が測定記録に書かれ、`tools/train/`のPythonの学習ツールが削除されていることである。

## 状態

起案。2026年10月12日に起案した。
着手は利用者の指示を待つ。
フェーズ0（改名）は、[学習率スケジュール](nnue-lr-schedule.md)と[NNUEの隠れ層と位相補間の出力](nnue-hidden-layer.md)の実装より先に行う。隠れ層の計画の実装は、本書のフェーズ2の後に始める。

## 目的

利用者は2026年10月12日に、学習器の正しさを確かめるためにPyTorchと同じ出力ができることを一度確認したら、互換を捨ててPythonの学習器を削除し、PSTとFMの学習器もRustへ移植してPythonの学習器を完全に捨てることを決めた。
Pythonの学習器を残すと、学習の数式を2つの言語で保守し続けることになり、Rustの学習器へ機能を足すたびにPython側との照合を求める圧力が生じる。
NNUEの学習器ではこの照合が既に済んでおり、PSTとFMにも同じ一度きりの照合を課せば、以後はRust側だけを正として保守できる。

本書で使う語を次のように定める。
MNSDは教師データ（局面、探索値、対局結果）のファイル形式、MNPTはPST（版2）またはPSTとFM（版3）の重みファイルの形式、MNUEはNNUEの重みファイルの形式である。
線形経路は、NNUEの出力のうちPSTと同じ形の部分（発火した特徴の重みを足し、序中盤用と終盤用の2端点を盤上総駒数で補間する）であり、非線形経路は第1層と出力部からなる残りの部分である。
鏡映は盤の左右反転であり、鏡映共有のPSTは左右対称な升の組で重みを共有する。
視点は、局面を手番側と相手側のどちらから見て特徴に番号を付けるかの区別であり、NNUEは両視点、PSTとFMは手番側の視点だけを使う。
教師値は、探索値を教師用K（教師データの分類ごとに訓練集合から推定する換算の尺度）で勝率に換算した値と対局結果を、来歴（教師データの生成条件を記録したファイル）に書かれた混合比λで混ぜたものである。
出力用Kは、学習したモデルの出力を勝率のロジットからセンチポーンへ換算する尺度であり、重みファイルに入っている。
v0は駒得だけで評価する最初の評価関数であり、駒除去差分の損失は、駒を1枚除いたときの評価の変化が参照（v0の駒価値または初期のPST）と同じ向きになることを求める補助的な損失である。
FMの補正は`Φ = ½ Σ_f a_f[(Σ_i v_if)² − Σ_i v_if²]`であり、iは発火した特徴、fは32個の成分、v_ifは特徴iの成分fの埋め込み、a_fは成分fの符号つきの係数である。
整数のFMは、埋め込みを2の指数e乗倍した整数で持ち、補正を2の(2e+1)乗で割って求めるので、整数を変えずにeを1増やすと補正は1/4になる。
PSTの候補Lは[PSTのエポック数の延長](pst-longer-training.md)で学んだPST、Pcはその初期値、FMの候補Faは[補正1/4のFMの再学習](fm-quarter-current-pst.md)で学んだFM、J75は[PSTとFMの同時学習](pst-fm-joint-training.md)でλ=0.75で学んだ候補である。

撤去の前提は、Pythonの学習器が担ってきた現行の手順をRustで再現できることである。
現行の評価関数の重み（`crates/minase/nets/pst.bin`）は、同時学習器の候補J75の出力のFMの係数aに倍率611/1024を掛けて量子化し直したもの（以下、倍率の焼き込み）である（[FMの倍率のSPSA調整](fm-scale-spsa.md)）。
J75の初期値は、PSTの学習器の候補Lと、FMの学習器の候補Faである。
したがって、PSTの学習、FMの学習と縮小、同時学習、倍率の焼き込みの4つを移植すれば、現行の重みに至る手順をRustだけでたどれる。

## 適用範囲

本マイルストーンで行う作業は次のとおりである。

- クレート`crates/nnue-trainer`を`crates/minase-trainer`へ、実行ファイルを`minase-trainer`へ改名し、既存の機能を`nnue`サブコマンドの下へ移す。
- PSTの学習器（`pst train`）、FMの学習器と縮小と倍率の焼き込み（`fm train`、`fm quarter`、`fm scale`）、同時学習器（`joint train`）を移植する。
- NNUEの診断（駒除去差分、評価値の分布、着手前後の評価差）とPSTの比較診断を`diag`サブコマンドへ移植する。
- 移植した学習器と診断について、Pythonとの一致を一度だけ確かめ、測定記録に書く。
- Pythonの学習ツール（`tools/train/`の`pyproject.toml`、`uv.lock`、`src/`、`tests/`、`scripts/`、`README.md`、`pst.example.toml`）、`dump-batch`サブコマンド、および呼び出し元がなくなる`minase dev pst-probe`と`minase dev nnue-probe`を削除する。
- 学習の手引き（`docs/guides/pst-training.md`）と、Pythonの学習器を前提に書かれた設計書と文書を、Rustの学習器の手順へ書き直す。

次の事項は移植しない。

- PSTの学習器の、左右鏡映した升の重みを共有しないモデル（`single`、`tapered`）、複数の学習率を1エポックずつ試して選ぶ機能、`--train-half`、`--rescore`、および学習の段取りをまとめた`pst-workflow`。現行の手順は鏡映共有のモデルと1つの学習率だけを使い、`--rescore`は空の指定（`-`）だけが使われている。段取りは手引きに書いたコマンドの列で置き換える。
- 学習ログでの検証損失の詳細な内訳（世代、由来、評価値の帯、対局の平均）。内訳はPSTの比較診断が出す。
- 駒数分布の報告（`taper-report`）と先読み教師の診断（`lookahead-diag`）。後者は、入力に使う王の位置の特徴のファイル（MNKF形式）を書き出すRustのコードが既になく、実行できない。
- 同時学習器に付属した診断（`removal-census`、`move-diffs`）。
- NNUEの診断のうち、Pythonの整数参照と照合する`compare`。照合の相手がなくなるからである。
- `docs/measurements/`と`docs/research/`にある、`minase_train`を読み込む研究用のPythonスクリプト。過去の測定の記録であり、書き直さない。

## 依存関係

Rustの学習器（[Rust製NNUE学習器（GPU）](nnue-trainer-rust.md)）は、MNSDの読み込み、対局単位の検証分割、先読み教師、教師用Kの推定、来歴の照合、v0の駒価値を参照とする駒除去差分の損失、3群のAdamW、チェックポイントと再開、および量子化誤差の検査を持つ。
NNUEの線形経路は、鏡映を共有し盤上総駒数で2端点を補間するPSTそのものであり、PSTの学習器と順伝播、二値交差エントロピー、駒除去差分の損失、切り詰めの範囲が同じ式である。
Adamに重み減衰を掛けない更新は、重み減衰0のAdamWと同じ式である。
一方で、線形経路だけを学ぶモードはない。第1層の幅は2以上、エポックの上限は50に限られ、`--lambda-override`は既定値が1.0なので「来歴のλを上書きしない」を指定できない。除去損失の参照はv0に固定され、書き出せる形式はMNUEだけである。

Pythonの学習器の仕様と現行の使い方は次のとおりである。

- PSTの学習器（`minase_train.pst.train`）: 候補Lは、10ファイル（訓練27,745,843局面）、鏡映共有モデル、学習率0.03、200エポック、改善なし5エポックで停止、除去損失の係数1000（参照は`--init`のMNPT）、先読み教師γ=0.9・n=40、来歴のλ（0.75）で学び、GPUで1エポック約2分だった（[記録](../measurements/pst-longer-training-training.md)）。出力はMNPT版2、`pst-float.npz`、`pst.training.json`である。
- FMの学習器（`minase_train.fm.train`）: 整数PSTを固定し、手番側の視点だけの13,680特徴に対して`Φ = ½ Σ_f a_f[(Σ_i v_if)² − Σ_i v_if²]`（ランク32）を学ぶ。訓練では局面ごとに確率0.5で左右鏡映し、観測マスクは訓練集合とその全鏡映から全特徴の空間で数える。損失は二値交差エントロピーに`10⁻⁴·mean(Φ²)`を加え、1群のAdamW（学習率10⁻³、重み減衰10⁻⁴、バッチ4,096）で学ぶ。候補FaはGPUで1エポック約2分、最良エポック8だった（[記録](../measurements/fm-quarter-current-pst-training.md)）。出力はMNPT版3と`fm-float.npz`であり、量子化は、指数eを30から1ずつ下げ、整数にした埋め込みの絶対値が16,383以下に収まる最大のeを選ぶ。`quarter`は指数を1増やし、`scale`は係数aに倍率を掛けて量子化し直す。
- 同時学習器（ブランチ`pst-fm-joint-training`の`minase_train.joint.train`）: 鏡映共有のPSTとFMを、1つのλの二値交差エントロピー、`10⁻⁴·mean(Φ²)`、係数1000の駒除去差分の損失（参照はv0、FMの寄与を含む）で同時に学ぶ。AdamWを2群（PST: 学習率0.03、重み減衰0。FM: 学習率10⁻³、重み減衰10⁻⁴）で使い、訓練では確率0.5で鏡映する。J75は10ファイル、λ=0.75、最良エポック2、7エポックで停止し、全体で3,881秒だった（[記録](../measurements/pst-fm-joint-training-training.md)）。出力はMNPT版3と、PSTとFMの実数の重みをまとめた`pst-float.npz`である。
- 乱数: Pythonは`torch.randperm`と`torch.rand`で順列と鏡映を決める。Rustの順列はSplitMix64なので、同じシードでも学習の結果はバイト単位では一致しない。

AlphaZero型探索の計画（[設計書](alphazero.md)）は`tools/train/zero/`に`minase_train`を読み込まない独立のPythonプロジェクトを置くので、本書の削除と衝突しない。

## 設計判断

| 項目 | 判断 |
|---|---|
| クレートの名前と構成 | `crates/nnue-trainer`を`crates/minase-trainer`へ改名し、実行ファイルを`minase-trainer`とする。サブコマンドは`nnue`（`train`、`check-equivalence`、`bench-step`）、`pst train`、`fm`（`train`、`quarter`、`scale`）、`joint train`、`diag`（`nnue-removal`、`nnue-distribution`、`nnue-moves`、`pst-compare`）とする。MNSDの読み込み、教師の構成、検証分割、最適化器、チェックポイント、量子化の検査はすべての学習器で共有する。PSTとFMの学習を別のクレートに分ける案は、これらの共有部品を二重に持つか、さらに共有のクレートを切り出すことになるので採らない。改名は、学習器を変える[学習率スケジュール](nnue-lr-schedule.md)と[NNUEの隠れ層と位相補間の出力](nnue-hidden-layer.md)の実装と差分が衝突するので、最初のフェーズで行う。 |
| PSTの学習器の実現 | NNUEの学習器に、非線形経路を持たない「線形経路だけ」のモデルを加え、それを`pst train`とする。順伝播、損失、駒除去差分、切り詰め、教師、分割、停止規則は既存の部品をそのまま使う。PSTの学習器を別に書く案は、同じ式を二重に持つので採らない。 |
| PSTの除去の参照 | `--removal-reference v0`と`--removal-reference init`を設ける。候補Lは`--init`のMNPT（Pc）を参照にし、J75とNNUEはv0を参照にしたので、現行の手順に両方が要る。 |
| λとエポックの上限 | `pst`、`fm`、`joint`では`--lambda-override`を省略可能とし、省略時は来歴のλを使う（候補Lの動作）。`nnue`は現行どおり既定1.0とする。エポックの上限は、`nnue`は50のまま、`pst`、`fm`、`joint`は200とする。候補Lが200エポック目で最良だったので、50では再現できないからである。 |
| FMと同時学習のモデル | FMの成分を、手番側の視点だけの13,680特徴に対する符号つき二乗の項として加える。`fm train`は整数PSTを固定してFMの成分だけを学び、`joint train`は線形経路とFMの成分を2群のAdamWで同時に学ぶ。訓練時の確率0.5の左右鏡映、全特徴の空間で数える観測マスク、`10⁻⁴·mean(Φ²)`の正則化はPythonと同じにする。NNUEの両視点の特徴や鏡映共有とは別の特徴の作り方になるが、FMとPSTは視点を1つしか持たないので、その違いを特徴の作り方の関数として分ける。 |
| 計算の場所 | PST、FM、同時学習は、まずCPUの並列実装で動かす。型はNNUEの学習器のCPU参照と同じく、重み、勾配、AdamWの状態をf32とし、局面方向とバッチ方向の和と損失の集計は、加算の順序による結果の揺れを避けるため、f64で固定の順に行う。Pythonの学習器も重みと状態をf32で持つ。そのうえで、フェーズ2から4で1エポックの時間を測る。PSTの順伝播は1局面あたり発火した特徴（最大145個）の重みの行の和であり、除去損失は1局面あたり平均約61件の除去で行の差を取るだけである。FMは同じ特徴について32成分の和を取る。どちらも、NNUEの幅128の第1層と補助損失（CPUで約6,600局面/秒、[検査の記録](../measurements/nnue-trainer-rust-checks.md)）より1局面あたりの計算が桁違いに少ないので、CPUで足りる見込みがある。測った1エポックの時間が、Pythonの学習器のGPUでの時間（同じ局面数に換算した値）を超えた学習器に限り、GPUのカーネルを別のフェーズとして加える。測定なしにGPUのカーネルを書く案は採らない。 |
| 一度きりの照合 | Pythonの学習器を、リポジトリにコミットしない一時的な改変で、照合用の再生ファイルを書き出すようにする。Rustの学習器には、それを読む一時的な引数`--replay <ディレクトリ>`を加える。再生ファイルは、入力ファイルの並びとSHA-256、エポックごとの訓練局面の列（各局面を（入力ファイルの番号、ファイル内の記録の番号）の組で表す）、各局面の鏡映の有無、学習前の実数の重み（PST、FMの埋め込みとa）、観測マスクを持つ。バッチは列をバッチの大きさで先頭から区切ったものとする。Rustは、ファイルの並びと検査和の不一致、列の不足、重複、訓練集合の外の局面、重みとマスクの形の不一致を拒否する。照合は次の3段で行う。(1) 最適化器の更新を3回行った後の損失、全パラメータ、AdamWの状態が、`|Rust − Python| ≤ 10⁻⁵ + 10⁻⁴·|Python|`で一致する。この許容誤差はNNUEの学習器をPyTorchと照合したときと同じ式である。(2) 再生ファイルで短縮した本学習を両方で行い、各エポックの検証損失が同じ式で一致し、最良エポックと停止エポックが一致する。(3) (2) で書き出した整数の重みの差がすべて±1単位以内で、FMの指数が一致する。`--replay`とPython側の改変は、照合の後に削除する。 |
| 焼き込みと書き出しのバイト一致 | `fm scale --fm-scale 611`をJ75の出力に適用して、現行の`crates/minase/nets/pst.bin`がバイト単位で再生成されることを確かめる。J75の`pst-float.npz`を量子化し直すとJ75の`pst.bin`にバイト単位で一致することと、`fm quarter`がPythonの出力とバイト単位で一致することも確かめる。これらは固定した実数の重みからの書き出しであり、乱数を含まないので、照合(3)の±1単位ではなくバイト単位の一致を求める。 |
| 診断の移植 | NNUEの`removal`、`distribution`、`moves`と、PSTの比較診断（世代と評価値の帯ごとの損失、整数評価の一致、量子化誤差、代表局面の除去差分の符号反転、成り手の照合、導出した駒価値）を、`minase`のライブラリを直接呼ぶRustのコマンドにする。Pythonの診断は`minase dev nnue-probe`と`minase dev pst-probe`を子プロセスとして呼んでいたが、Rustからはライブラリの評価関数を直接呼べるので、両コマンドは利用者がいなくなって削除する。移植の一致は、同じネットに対するPythonとRustの出力を比べて一度だけ確かめる。PythonはNumPyの乱数で診断の標本を選ぶので、照合のときだけPythonが選んだ標本の番号を一時的な引数`--replay-sample`でRustへ渡し、標本の番号の検査和が一致することを確かめてから値を比べる。整数の値は完全な一致、実数の値は照合(1)と同じ式で比べ、入力のパス、実行環境、実行時間の欄は比較から除く。`--replay-sample`は照合の後に削除する。 |
| 削除の順序 | NNUEに関わるPythonのコード（学習器、整数参照、`nnue-diagnostics`、照合スクリプト、`rust_float_to_npz.py`）と`dump-batch`は、NNUEの診断をRustへ移して一致を確かめた直後に削除する。[NNUEの完全代替](nnue-replacement.md)のフェーズ4に残る診断（駒除去差分、分布、着手前後の差）は、移植したRustの診断で行う。PST、FM、同時学習のPythonは、それぞれの照合が済んだ後、最後のフェーズでまとめて削除する。 |
| 量子化誤差の検査 | 量子化誤差を測る部品は共有するが、合格基準は学習器ごとにPythonの学習器と同じにする。`pst train`は書き出したPSTの平均絶対誤差が2センチポーン以下、`fm train`は補正をそのまま使う場合と1/4にした場合の両方で2センチポーン以下、`joint train`は学習前の初期値と書き出した重みの両方で2センチポーン以下、`nnue train`は現行どおり`K/40`以下とする。基準を満たさない場合は、書き出した物を残して失敗で終了する。 |
| 再開と鏡映の乱数 | `pst`、`fm`、`joint`も`nnue`と同じく`--resume <出力ディレクトリ>`でエポック境界から再開でき、学習の軌跡を変える引数は設定ハッシュで照合する。訓練時の鏡映の有無は、状態を持つ乱数ではなく（シード、エポック、局面の番号）のSplitMix64のハッシュの1ビットで決める。こうすると、途中で止めて再開した学習でも、止めずに通した学習と同じ局面が同じ向きで鏡映される。 |
| 同時学習器のソース | 同時学習器はブランチ`pst-fm-joint-training`にだけあり、masterにも本書を置くブランチ`nnue-replacement`にもない。照合では、そのコミットの`tools/train/`を`git archive`でリポジトリの外へ展開し、既存のPython環境で実行する。ブランチをマージして取り込む案は、削除するためのコードを一度取り込むことになるので採らない。 |
| 文書 | `docs/guides/pst-training.md`は、`pst-workflow`の段取りをRustのコマンドの列に置き換えて書き直す。`tools/train/README.md`にある記録の項目の定義は`crates/minase-trainer/README.md`へ移す。[Rust製NNUE学習器（GPU）](nnue-trainer-rust.md)の「Pythonとの等価性」の節は、移植時に一度確かめた照合の結果へのリンクに縮め、以後の正しさの契約をCPU参照、数値微分、GPUとCPU参照の等価性に書き換える。 |

## 引数の対応

移植する学習器の引数は、Pythonの学習器の引数を次のように引き継ぐ。
表にない引数は移植しない。

| サブコマンド | 引数（既定値） |
|---|---|
| `pst train` | `--data`（複数）、`--init`（MNPT。出力用Kと駒価値もここから引き継ぐ）、`--output`、`--lr`（0.03）、`--epochs`（必須、上限200）、`--patience`（必須）、`--batch`（16,384）、`--seed`（1）、`--removal-penalty`（1000）、`--removal-reference v0|init`（必須）、`--lambda-override`（省略時は来歴）、`--lookahead-gamma`と`--lookahead-plies`、`--validation-sample`（10,000）、`--threads` |
| `fm train` | `--data`、`--init`（MNPT版2、PSTを固定）、`--output`、`--epochs`、`--patience`（ともに必須）、`--rank`（32）、`--batch`（4,096）、`--seed`（1）、`--lr`（10⁻³）、`--weight-decay`（10⁻⁴）、`--lambda-res`（10⁻⁴）、`--lambda-override`、先読みの2引数、`--validation-sample`、`--threads` |
| `fm quarter` | `--input`、`--output` |
| `fm scale` | `--input`、`--float`、`--fm-scale`（0は版2を書く）、`--output`、`--data` |
| `joint train` | `--data`、`--base`（MNPT版3）、`--pst-init`、`--fm-init`、`--fm-reference`（縮小前のFMのMNPT版3。初期値の検査に使う）、`--output`、`--lambda-override`（必須）、`--epochs`（200）、`--patience`（5）、`--batch`（16,384）、`--seed`（1）、`--pst-lr`（0.03）、`--fm-lr`（10⁻³）、`--fm-weight-decay`（10⁻⁴）、`--lambda-res`（10⁻⁴）、`--removal-penalty`（1000）、先読みの2引数、`--validation-sample`、`--threads` |

Pythonの`pst train`の必須引数`--k`（出力用K）は移植せず、`--init`のMNPTのKを使う。候補Lの学習は`--init`のMNPTと同じKを`--k`に指定していたので、この仕様でも同じ結果になり、照合で一致を確かめる。
全学習器に共通の引数として、`--resume <出力ディレクトリ>`と`--threads`を持つ。
`--device`は移植しない。
計算の場所は「設計判断」のとおり実装で決まり、GPUのカーネルを加えた学習器だけが`--device`を持つ。
`pst`、`fm`、`joint`の出力（MNPT、実数の重みのnpz、学習記録のJSON）のファイル名と中身の並びは、Pythonの学習器と同じにする。
後続の学習と診断が、これらのファイルを入力に使うからである。
チェックポイントと再開は、`nnue`と同じ仕組みを`pst`、`fm`、`joint`にも使う。

## 所要時間の見積り

実装はコーディングエージェントのcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。
フェーズ0は約半日、フェーズ1は約1日、フェーズ2から4はそれぞれ約1〜2日、フェーズ5は約1日、フェーズ6は約半日と見込む。
照合の学習は、Pythonの学習器を1ファイル、数エポックに短縮した条件で行い、PST、FM、同時学習の各1組で合計約1〜2時間と見込む。
GPUのカーネルが要る学習器が出た場合は、その学習器ごとに約2〜3日を加える。

## 実装フェーズ

各フェーズの完了時に`cargo test`（ワークスペース全体）と、GPUを使う`gpu_`で始まるテストが通ることを確かめる。

### フェーズ0　クレートの改名

`crates/nnue-trainer`を`crates/minase-trainer`へ、実行ファイルを`minase-trainer`へ改名し、既存の`train`、`check-equivalence`、`bench-step`を`nnue`サブコマンドの下へ移す。
ワークスペースの`Cargo.toml`、クレートのREADME、設計書、手引きの参照を新しい名前に直す。
過去の測定記録と`data/`の実行スクリプトの参照は直さない。

完了条件は、改名の前後で、世代3の1ファイル（`data/gen3/generated-1500000.bin`）の`nnue train`の1エポック学習の`nnue.bin`がバイト単位で同じであることである。

### フェーズ1　NNUEの診断の移植とPythonのNNUEの削除

`diag nnue-removal`、`diag nnue-distribution`、`diag nnue-moves`を加える。
[NNUEの完全代替](nnue-replacement.md)のフェーズ4の2つのネットについて、Pythonの`nnue-diagnostics`の出力とRustの出力を比べ、整数の値は完全に、実数の値は「設計判断」の照合(1)と同じ式で一致することを確かめる。
一致を確かめた後、PythonのNNUEの学習器（`minase_train.nnue`）、`nnue-diagnostics`、`scripts/crosscheck_reference.py`、`scripts/crosscheck_batch.py`、`scripts/rust_float_to_npz.py`、`dump-batch`サブコマンドを削除する。

完了条件は、診断の一致が[照合の記録](../measurements/python-trainer-retirement-checks.md)に書かれ、削除後のワークスペースのテストが通ることである。

### フェーズ2　PSTの学習器

線形経路だけのモデル、`--removal-reference`、`--lambda-override`の省略、エポックの上限200、MNPT版2と`pst-float.npz`と`pst.training.json`の書き出しを加え、`pst train`とする。
一時的な`--replay`を加え、Python側の一時的な改変で書き出した再生ファイルを使って、候補Lの条件を1ファイルと数エポックに短縮した学習を両方で行い、「設計判断」の照合(1)から(3)を確かめる。
1エポックの時間を測り、Pythonの時間と比べる。

完了条件は、照合の結果と時間が[照合の記録](../measurements/python-trainer-retirement-checks.md)に書かれ、連続2エポックの学習と1エポックで止めて再開した学習の重みとAdamWの状態がビット単位で一致することである。フェーズ3と4にも同じ再開の条件を課す。

### フェーズ3　FMの学習器、縮小、倍率の焼き込み

FMの成分、鏡映の確率的な適用、全特徴の空間の観測マスク、正則化、MNPT版3と`fm-float.npz`の書き出し、指数の探索と補正0の検査、`fm quarter`、`fm scale`を加える。
候補Faの条件を短縮した学習で、フェーズ2と同じ照合を行う。
`fm quarter`の出力がPythonとバイト単位で一致し、`fm scale --fm-scale 611`をJ75の出力に適用した結果が`crates/minase/nets/pst.bin`とバイト単位で一致することを確かめる。

完了条件は、照合の結果と時間が記録されていることである。

### フェーズ4　同時学習器

`joint train`を加える。
初期値の検査として、`--pst-init`を量子化し直すと`--base`のPSTに、`--fm-init`を量子化し直すと`--fm-reference`の縮小前のFMに一致することを確かめる（Pythonの`verify-init`と同じ検査を学習の開始時に行う）。
J75の条件を短縮した学習で、フェーズ2と同じ照合を行う。
Pythonの同時学習器は、ブランチ`pst-fm-joint-training`のコミットを`git archive`でリポジトリの外へ展開して実行する。

完了条件は、照合の結果と時間が記録されていることである。

### フェーズ5　PSTの比較診断

`diag pst-compare`を加え、候補Lと基準の組について、Pythonの`pst-diagnostics`の出力と比べて一致を確かめる。

完了条件は、一致が記録されていることである。

### フェーズ6　Pythonの撤去と文書の書き直し

`--replay`、`--replay-sample`、`tools/train/`のPythonの学習ツール、`minase dev pst-probe`、`minase dev nnue-probe`、`.gitignore`の`/tools/train/.venv`を削除する。
`docs/guides/pst-training.md`をRustのコマンドの列で書き直し、`tools/train/README.md`の記録の項目の定義を`crates/minase-trainer/README.md`へ移す。
[Rust製NNUE学習器（GPU）](nnue-trainer-rust.md)の「Pythonとの等価性」の節を書き換え、[NNUEの完全代替](nnue-replacement.md)、[Athénanの計画](athenan.md)、[評価特徴の再学習](evaluation-terms-relearning.md)、[デバッグの手引き](../guides/debugging.md)、[独立の参照で診断を照合する教訓](../lessons/compare-diagnostics-with-independent-reference.md)、[NNUEの隠れ層と位相補間の出力](nnue-hidden-layer.md)のPythonの学習器への参照を直す。
`pst-probe`と`nnue-probe`を参照する文書とテストも直す。

完了条件は、`git grep minase_train`の該当が`docs/measurements/`と`docs/research/`の過去の記録だけになり、ワークスペースのテストが通ることである。

## 検証

完了前に次を確認する。

- `cargo test`（ワークスペース全体）と、`gpu_`で始まるGPUのテストがすべて通る。
- 照合の記録に、フェーズ1から5の一致の結果、各学習器の1エポックの時間、およびPython側に加えた一時的な改変の差分が書かれている。
- `crates/minase/nets/pst.bin`が`fm scale`でバイト単位で再生成される。
- 書き直した`docs/guides/pst-training.md`の手順で、候補Lの条件の学習が最初から最後まで実行できる（短縮した条件でよい）。

## 完了条件

フェーズ6の完了条件を満たし、照合の記録がそろった時点で完了とする。
採否を測る候補を持たない基盤整備なので、[設計書の書き方](README.md)の規則により、状態は完了（採用）とする。
照合のいずれかが一致しない場合は、原因を調べてRust側を直す。原因がPython側の誤りだった場合は、その内容を利用者に報告し、Pythonに合わせるかRustの正しい振る舞いを採るかの判断を照合の記録に書いてから進める。

## 参考資料

- [Rust製NNUE学習器（GPU）](nnue-trainer-rust.md)、[NNUEの共通土台と初代のネットによるPST＋FMの完全代替](nnue-replacement.md)、[学習率スケジュール](nnue-lr-schedule.md)、[NNUEの隠れ層と位相補間の出力](nnue-hidden-layer.md)
- [PSTとFMの同時学習](pst-fm-joint-training.md)、[FMの倍率のSPSA調整](fm-scale-spsa.md)、[補正1/4のFMの再学習](fm-quarter-current-pst.md)、[PSTのエポック数の延長](pst-longer-training.md)
- [PSTの学習の手引き](../guides/pst-training.md)
- [同時学習の記録](../measurements/pst-fm-joint-training-training.md)、[倍率の焼き込みの検査](../measurements/fm-scale-spsa-candidate.md)

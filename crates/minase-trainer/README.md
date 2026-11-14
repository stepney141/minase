このクレートはminaseの評価関数を学習し、学習済み重みを変換、診断する。
駒位置テーブル（PST）は`pst train`、駒の組の相互作用を表すFactorization Machine（FM）は`fm train`、両者の同時学習は`joint train`、差分更新可能なニューラルネットワーク（NNUE）は`nnue train`を使う。
NNUEでは浮動小数点モデル、処理1〜8のCPU参照実装、およびCUDAとCPUでの学習を提供する。
処理の番号と式は`src/nnue/reference/mod.rs`のdocコメントが定める。
GPU側では第1層の前進、全パラメータの勾配、部分和の縮約、AdamWによる更新と切り詰め、検証を実行できる。
学習の数学的な契約は各モジュールのdocコメントにあり、方式を選んだ理由は[ADRの索引](../../docs/adr/README.md)から引ける。

評価関数ごとに独立した学習パイプラインを持ち、評価関数の知識を要しないデータ処理と機械学習の部品だけを共有する。
`src/`の配置は次のとおりである。

| ディレクトリ | 役割 |
|---|---|
| `data/` | データの読み込み、分割、教師値の構築を共有する。 |
| `nn/` | Adamの要素更新、並列処理、配列の入出力、停止判定などの汎用部品を提供する。 |
| `nnue/` | NNUEの特徴構築、モデル、CPU参照計算、GPU処理と学習を担う。 |
| `pst/` | PST専用のバッチ構築、計算、モデルと学習を担う。 |
| `fm/` | FMの学習、量子化、補正の縮小と倍率の焼き込みを担う。 |
| `joint/` | PSTとFMの同時学習と初期値の検査を担う。 |
| `diagnostics/` | 評価関数を診断する。 |

## サブコマンドと文書

各サブコマンドを実行する手順は次の表の手引きにあり、入出力の形式と記録の欄は表に示すモジュールのdocコメントが定める。
学習データの由来はMNSDレコードに保存する。教師値を使うコマンドは`--lambda`を必須とし、実戦対局にはλ=0を適用する。学習データの形式は`../minase/src/training/records.rs`、教師値と検証分割は`src/data/`のdocコメントに定める。

| サブコマンド | 役割 | 手順 | 形式と記録の欄 |
|---|---|---|---|
| `pst train` | 鏡映共有PSTをCPUで学習する。 | [PSTとFMの学習手順](../../docs/guides/pst-training.md) | `src/pst/` |
| `fm train`、`fm quarter`、`fm scale` | 固定PSTへ加えるFM補正をCPUで学習し、補正の縮小と倍率の焼き込みを行う。 | [PSTとFMの学習手順](../../docs/guides/pst-training.md#6-fmの補正を学習する) | `src/fm/` |
| `joint train` | PSTとFMを同時学習する。 | [PSTとFMの学習手順](../../docs/guides/pst-training.md#6-fmの補正を学習する) | `src/joint/` |
| `nnue train` | NNUEをCUDAまたはCPUで学習する。 | [NNUEの学習と診断の手順](../../docs/guides/nnue-training.md) | `src/nnue/train/`、`src/nnue/options.rs` |
| `nnue fit-linear` | 学習済みのNNUEの線形の経路の序中盤の列を、対局結果へのロジスティック回帰で補正する。 | [NNUEの学習と診断の手順](../../docs/guides/nnue-training.md#線形の経路を対局結果で補正する) | `src/nnue/fit_linear/` |
| `nnue check-equivalence`、`nnue bench-step` | GPUの計算をCPU参照と照合し、1更新の時間を測る。 | [NNUEの学習と診断の手順](../../docs/guides/nnue-training.md#学習器を変更したとき) | `src/nnue/gpu/equivalence.rs`、`src/nnue/gpu/bench.rs` |
| `diag pst-compare` | 2つのPSTを教師分類と位相帯ごとに比較する。 | [PSTとFMの学習手順](../../docs/guides/pst-training.md#5-pstを診断する) | `src/diagnostics/pst_compare.rs`、`src/diagnostics/pst_statistics.rs` |
| `diag nnue-distribution`、`diag nnue-removal`、`diag nnue-moves` | NNUEの評価分布、駒除去差分、合法手の評価差を診断する。 | [NNUEの学習と診断の手順](../../docs/guides/nnue-training.md#学習した重みを診断する) | `src/diagnostics/nnue_distribution.rs`、`src/diagnostics/nnue_removal.rs`、`src/diagnostics/nnue_moves.rs` |

NNUEの経路の公開API、GPUの実装、およびテストの契約は`src/nnue/`の各モジュールとテストのdocコメントにあり、チェックポイントの形式は`src/nnue/model/checkpoint.rs`の`Checkpoint`のdocコメントにある。

`nnue train`の学習の長さは、教師の量に依存しないsuperbatchで指定する。
`--superbatches N`、`--validation-superbatches V`、`--lr-schedule`を必須とし、1 superbatchの更新回数Bは`--batches-per-superbatch`で指定する。Bの既定値は6,104である。
学習率の半減期と総長もsuperbatchで数え、N×B回の更新を終えた`latest.npz`から`nnue.bin`を書き出す。
検証は学習前、V superbatchごと、および学習の最後に行い、損失は停止や重みの選択には使わない。
時間切れでは`summary.json`に`incomplete`を記録して終了コード1で終わり、最後に確定した`latest.npz`から`--resume`で続ける。
再開時はNとV、時間の上限、スレッド数、bucketの容量と作業ディレクトリを変更できるが、Bと学習率の予定は一致を要求する。

NNUEの訓練損失と検証損失は、ネットの勝率pと教師値の勝率qの差の2乗(p−q)²の平均とする。
`--teacher-cp-per-logit`を省略すると`--base`の勝率尺度を使い、`calibrate`または較正ファイルを指定するとその尺度を使う。
第1層と出力部の重み減衰の既定値はいずれも0である。
`nnue fit-linear`と教師尺度の較正は、二値交差エントロピーを使う。

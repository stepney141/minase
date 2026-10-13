このクレートはminaseの評価関数を学習し、学習済み重みを変換、診断する。
駒位置テーブル（PST）は`pst train`、駒の組の相互作用を表すFactorization Machine（FM）は`fm train`、両者の同時学習は`joint train`、差分更新可能なニューラルネットワーク（NNUE）は`nnue train`を使う。
NNUEでは浮動小数点モデル、カーネル1〜9のCPU参照実装、およびCUDAとCPUでの学習を提供する。
カーネルの番号と式は`src/nnue/reference/mod.rs`のdocコメントが定める。
GPU側では第1層の前進、駒除去差分の補助損失、全パラメータの勾配、部分和の縮約、AdamWによる更新と切り詰め、検証を実行できる。
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

各サブコマンドの入出力と記録の欄は、次の表の文書が定める。
学習データ、来歴ファイル、教師値、および検証分割は全コマンドで共通であり、[学習データと教師値](../../docs/reference/trainer/data-and-teachers.md)にある。

| サブコマンド | 役割 | 文書 |
|---|---|---|
| `pst train` | 鏡映共有PSTをCPUで学習する。 | [PST、FM、同時学習の記録](../../docs/reference/trainer/pst-fm-training-records.md)、[PSTとFMの学習手順](../../docs/guides/pst-training.md) |
| `fm train`、`fm quarter`、`fm scale` | 固定PSTへ加えるFM補正をCPUで学習し、補正の縮小と倍率の焼き込みを行う。 | [PST、FM、同時学習の記録](../../docs/reference/trainer/pst-fm-training-records.md) |
| `joint train` | PSTとFMを同時学習する。 | [PST、FM、同時学習の記録](../../docs/reference/trainer/pst-fm-training-records.md) |
| `nnue train` | NNUEをCUDAまたはCPUで学習する。 | [NNUEの学習](../../docs/reference/trainer/nnue-training.md) |
| `nnue check-equivalence`、`nnue bench-step` | GPUの計算をCPU参照と照合し、1更新の時間を測る。 | [NNUEの学習](../../docs/reference/trainer/nnue-training.md#gpuの検査と速度測定) |
| `diag pst-compare` | 2つのPSTを教師分類と位相帯ごとに比較する。 | [PSTの比較診断の記録](../../docs/reference/trainer/pst-compare-records.md) |
| `diag nnue-distribution`、`diag nnue-removal`、`diag nnue-moves` | NNUEの評価分布、駒除去差分、合法手の評価差を診断する。 | [NNUEの診断の記録](../../docs/reference/trainer/nnue-diagnostics-records.md) |

NNUEの経路の公開API、GPUの実装、およびテストの契約は`src/nnue/`の各モジュールとテストのdocコメントにあり、チェックポイントの形式は[NNUEの学習](../../docs/reference/trainer/nnue-training.md#チェックポイントの形式)にある。

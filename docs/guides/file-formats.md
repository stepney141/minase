# ファイル形式の一覧

本書は、minaseのコード（`crates/`の各クレート）がディスクに読み書きするファイル形式を一覧にする。
各形式のバイト配置と欄の仕様は、その形式を書き出すモジュールのdocコメントにあり、本書は形式ごとにその場所を示すだけである。
仕様をdocコメントに置くのは、形式を変えるコミットで仕様も同時に直せるようにするためである。
形式を追加、変更、または廃止するコミットでは、本書の該当行も同じコミットで直す。

形式は、ディスク上のスキーマ（バイト配置または欄の構成）ごとに1行とする。
同じスキーマを持つ複数のファイルは1行にまとめ、同じファイル名でもスキーマが違えば別の行にする。
テストだけが使うファイル、標準出力と標準エラー、`scripts/`の補助スクリプトだけが読み書きするファイル、およびビルドした実行ファイルそのもの（`target/match-cache/`にキャッシュするエンジンを含む）は載せない。
表の「識別と版」は、形式を見分ける手がかりと版である。
独自のバイナリ形式ではファイル先頭のマジックと版番号、JSONでは版の欄を書き、NPZやtarのような汎用のコンテナではコンテナの種類と、あれば中身の種別を示す欄を書く。
どれも持たない形式は「なし」とする。
`minase`はエンジンと補助ツールの実行ファイル、`minase-trainer`は学習器と診断（`diag`サブコマンド）の実行ファイルである。
表の書き手と読み手の欄では、節の中で実行ファイルが明らかなときはその名前を省き、`pst train`のようにサブコマンドから書く。
`<MNSD>`や`<番号>`のように山括弧で囲んだ部分は、実際のファイル名や番号に置き換わる。

## 教師データと重み

教師データは、自己対局や実戦棋譜の局面に教師の探索値と対局結果を付けた学習用のデータである。
重みは、エンジンの評価関数が読む整数の重みと、学習器が受け渡す浮動小数点の重みである。
PSTは駒の種類と位置ごとの点数表、FMは2駒の組の関係を補う補正、NNUEは差分更新できるニューラルネットの評価関数である。
MNPTは、FMを持たない重みをFM節の次元0で、FMを持つ重みを次元32で表す。
NPZはNumPyの配列を名前つきで収めた無圧縮のZIPであり、コンテナとNPYの配置は`crates/minase-trainer/src/nn/npz.rs`にある。

| 形式 | 識別と版 | 書き手 | 読み手 | 仕様 |
|---|---|---|---|---|
| MNSD（教師データ） | `MNSD`、版2 | `minase data selfplay generate`、`minase data lishogi games`、`minase-trainer diag pst-compare` | 学習器、診断、`minase data selfplay inspect`、`minase-trainer fm scale --data`、`nnue bench-step` | `crates/minase/src/training/records.rs` |
| MNUE（NNUEの重み） | `MNUE`、版5 | `minase-trainer nnue train` | エンジン、診断、`nnue train`（書き出した重みの検査） | `crates/minase/src/eval/nnue/format.rs` |
| MNPT（PSTの重みとFMの補正） | `MNPT`、版3 | `minase-trainer pst train`、`fm train`、`joint train`、`fm quarter`、`fm scale` | エンジン、学習器、診断 | `crates/minase/src/eval/pst/format.rs` |
| PSTの浮動小数点の重み`pst-float.npz`（2配列） | NPZ、版なし | `minase-trainer pst train` | `joint train --pst-init`、`diag pst-compare` | `crates/minase-trainer/src/pst/npz.rs` |
| FMの浮動小数点の重み`fm-float.npz`（3配列） | NPZ、版なし | `minase-trainer fm train` | `joint train --fm-init` | `crates/minase-trainer/src/fm/npz.rs` |
| 同時学習の浮動小数点の重み`pst-float.npz`（5配列） | NPZ、版なし | `minase-trainer joint train` | `fm scale --float`、`joint train`、`diag pst-compare` | `crates/minase-trainer/src/joint/export.rs` |

教師データは付随ファイルを持たない。
MNSDは1局ごとに開始局面を1回だけ書き、以後は1手ごとに指した手、価値、印、および方策を並べる対局単位の形式であり、読み手は手順を再生して各局面を復元する。
教師の分類と検証集合は、MNSDのヘッダと対局の由来から決まる。

## 学習器の保存物

学習器`minase-trainer`は、`--output-dir`で指定したディレクトリに、設定、エポックごとの記録、終了報告、およびチェックポイントを書き出す。
チェックポイントは、学習を中断して再開するために保存するモデルと最適化器の状態であり、状態の配列と、進捗と設定を記したJSONの配列`meta`をNPZに収める。
再開時には、`meta`に収めた設定と現在の`config.json`を欄ごとに比べる。
NNUEの学習器が`--buckets`を2以上にしたときに作業ディレクトリへ書く一時ファイル（局面、検証局面、教師の勝率尺度の推定の入力、およびエポックごとのbucket）は、その実行だけが読み書きして終了時に削除する実行専用のファイルであり、保存形式ではないので表に載せない。
配置は`crates/minase-trainer/src/data/spill.rs`のdocコメントにあり、方針は[保存するファイル形式の版は1つにし、実行専用の一時ファイルだけは独自の配置を認める](../adr/keep-one-version-per-saved-format-and-allow-run-private-scratch-files.md)にある。

| 形式 | 識別と版 | 書き手 | 読み手 | 仕様 |
|---|---|---|---|---|
| NNUEのチェックポイント`latest.npz`、`best.npz` | NPZ、`meta`の`kind = "nnue"` | `nnue train` | `nnue train`（終了時の書き出しと`--resume`）、`diag nnue-removal --float` | `crates/minase-trainer/src/nnue/model/checkpoint.rs` |
| PSTのチェックポイント`latest.npz`、`best.npz` | NPZ、`meta`の`kind = "pst"` | `pst train` | `pst train`（終了時の書き出しと`--resume`） | `crates/minase-trainer/src/pst/checkpoint.rs` |
| FMのチェックポイント`latest.npz`、`best.npz` | NPZ、`meta`の`kind = "fm"` | `fm train` | `fm train`（終了時の書き出しと`--resume`） | `crates/minase-trainer/src/fm/checkpoint.rs` |
| 同時学習のチェックポイント`latest.npz`、`best.npz` | NPZ、`meta`の`kind = "joint"` | `joint train` | `joint train`（終了時の書き出しと`--resume`） | `crates/minase-trainer/src/joint/checkpoint.rs` |
| NNUEの学習の設定`config.json` | なし | `nnue train` | `nnue train --resume` | `crates/minase-trainer/src/nnue/train/record.rs` |
| PSTの学習の設定`config.json` | なし | `pst train` | `pst train --resume` | `crates/minase-trainer/src/pst/record.rs` |
| FMの学習の設定`config.json` | なし | `fm train` | `fm train --resume` | `crates/minase-trainer/src/fm/record.rs` |
| 同時学習の設定`config.json` | なし | `joint train` | `joint train --resume` | `crates/minase-trainer/src/joint/record.rs` |
| NNUEのエポックの記録`training.jsonl` | なし | `nnue train` | `nnue train --resume` | `crates/minase-trainer/src/nnue/train/record.rs`の`EpochRecord` |
| PSTのエポックの記録`training.jsonl` | なし | `pst train` | `pst train --resume` | `crates/minase-trainer/src/pst/record.rs`の`EpochRecord` |
| FMのエポックの記録`training.jsonl` | なし | `fm train` | `fm train --resume` | `crates/minase-trainer/src/fm/record.rs`の`EpochRecord` |
| 同時学習のエポックの記録`training.jsonl` | なし | `joint train` | `joint train --resume` | `crates/minase-trainer/src/joint/record.rs`の`EpochRecord` |
| NNUEの終了報告`summary.json` | なし | `nnue train` | なし | `crates/minase-trainer/src/nnue/train/mod.rs`の`Summary` |
| PSTの終了報告`summary.json` | なし | `pst train` | なし | `crates/minase-trainer/src/pst/record.rs`の`Summary` |
| FMの終了報告`summary.json` | なし | `fm train` | なし | `crates/minase-trainer/src/fm/record.rs`の`Summary` |
| 同時学習の終了報告`summary.json` | なし | `joint train` | なし | `crates/minase-trainer/src/joint/record.rs`の`Summary` |

終了報告は、学習が途中で失敗した場合や、同時学習の開始時の量子化検査に通らなかった場合には、`status`と`reason`を中心とする少数の欄だけを持つ。
欄の構成は同じ`Summary`のdocコメントにある。

## 取り込み、診断、および重みの変換の報告

| 形式 | 識別と版 | 書き手 | 読み手 | 仕様 |
|---|---|---|---|---|
| lishogiの取り込みの報告（`--report`） | なし | `minase data lishogi games`、`data lishogi openings` | なし | `crates/minase/src/bin/minase/lishogi_import/filter.rs`の`Report` |
| PSTの比較診断の報告`report.json` | なし | `minase-trainer diag pst-compare` | なし | `crates/minase-trainer/src/diagnostics/pst_compare.rs`の`Report` |
| 診断の標本の通し番号`diagnostic-indices-*.npy` | NPY 1.0 | `minase-trainer diag pst-compare` | なし | `crates/minase-trainer/src/diagnostics/pst_sample.rs`の`write_indices` |
| 評価値の分布の診断の報告（`--output`で指定） | なし | `minase-trainer diag nnue-distribution` | なし | `crates/minase-trainer/src/diagnostics/nnue_distribution.rs`の`Report` |
| 駒除去差分の診断の報告（`--output`で指定） | なし | `minase-trainer diag nnue-removal` | なし | `crates/minase-trainer/src/diagnostics/nnue_removal.rs`の`Report` |
| 合法手の評価差の診断の報告（`--output`で指定） | なし | `minase-trainer diag nnue-moves` | なし | `crates/minase-trainer/src/diagnostics/nnue_moves.rs`の`Report` |
| FMの補正を縮める変換の報告`<出力の拡張子を除いた名前>.quarter.json` | なし | `minase-trainer fm quarter` | なし | `crates/minase-trainer/src/fm/convert.rs`の`QuarterReport` |
| FMの倍率を焼き込む変換の報告`<出力の拡張子を除いた名前>.scale.json` | なし | `minase-trainer fm scale` | なし | `crates/minase-trainer/src/fm/convert.rs`の`ScaleReport` |
| 教師の勝率尺度の較正ファイル（`--output`で指定） | `format = "minase-trainer-teacher-calibration"` | `minase-trainer data calibrate` | `nnue train`、`pst train`、`fm train`、`joint train`、`diag pst-compare`（`--teacher-cp-per-logit`） | `crates/minase-trainer/src/data/calibration.rs` |

## 対局ハーネスとSPSAの実行ディレクトリ

対局ハーネス`minase match run`とSPSA（同時摂動確率近似による探索係数の調整）`minase spsa`は、測定ごとの実行ディレクトリに条件と結果を書き、中断からの再開や集計ではそのファイルを読む。

| 形式 | 識別と版 | 書き手 | 読み手 | 仕様 |
|---|---|---|---|---|
| 対局ハーネスの実行条件`manifest.json` | `format_version = 5` | `minase match run` | `minase match run --resume`、`minase match report` | `crates/minase/src/bin/minase/match_runner/storage.rs` |
| 対局ハーネスの起動の記録`invocations.json` | なし | `minase match run` | `minase match run --resume`、`minase match report` | `crates/minase/src/bin/minase/match_runner/storage.rs` |
| 対局ハーネスの累計実行時間`summary.json` | なし | `minase match run` | `minase match run --resume`、`minase match report` | `crates/minase/src/bin/minase/match_runner/storage.rs` |
| 対局ハーネスのペアの記録`pairs/<番号>.json` | なし | `minase match run` | `minase match run --resume`、`minase match report` | `crates/minase/src/harness/records/mod.rs` |
| SPSAの実行条件`manifest.json` | なし | `minase spsa` | `minase spsa --resume`、`minase spsa apply` | `crates/minase/src/bin/minase/spsa_runner/storage.rs` |
| SPSAの起動の記録`invocations.json` | なし | `minase spsa` | `minase spsa --resume` | `crates/minase/src/bin/minase/spsa_runner/storage.rs` |
| SPSAの反復の記録`iterations/<番号>.json` | なし | `minase spsa` | `minase spsa --resume`、`minase spsa apply` | `crates/minase/src/bin/minase/spsa_runner/storage.rs` |
| 対局用バイナリの検査和`target/match-cache/<キー>/minase.sha256` | なし | `minase match run`、`minase spsa` | 同じコマンド | `crates/minase/src/harness/commit.rs` |
| コミットを展開する一時アーカイブ`target/match-cache/.source-*.tar` | tar | `minase match run`、`minase spsa` | 同じコマンド | `crates/minase/src/harness/commit.rs` |
| 排他ロック用の空ファイル`.match_runner.lock`、`.build.lock` | なし | `minase match run`、`minase spsa` | 同じコマンド、`minase match report`、および`minase spsa apply`がロックを取るだけで、内容は読まない | `crates/minase/src/harness/storage.rs`、`crates/minase/src/harness/commit.rs` |

## テキストの入力と出力

| 形式 | 識別と版 | 書き手 | 読み手 | 仕様 |
|---|---|---|---|---|
| 開始局面一覧 | なし | `minase data lishogi openings` | `minase data selfplay generate --openings` | `crates/minase/src/bin/minase/lishogi_import/mod.rs` |
| lishogiの棋譜のNDJSON | なし | lishogiのAPI（`scripts/fetch_lishogi_games.py`が保存する） | `minase data lishogi games`、`data lishogi openings` | `crates/minase/src/bin/minase/lishogi_import/input.rs` |
| SPSAの係数ファイル | なし | 利用者が用意する（`minase spsa params`が行を標準出力へ書く） | `minase spsa` | `crates/minase/src/bin/minase/spsa_runner/params.rs` |
| SPSAが既定値を書き戻すマクロ`parameters!` | なし | `minase spsa apply` | `minase spsa apply` | `crates/minase/src/bin/minase/spsa_runner/apply.rs` |
| エンジンの入出力の記録（`--io-log`） | なし | `minase --io-log` | なし | `crates/minase/src/bin/minase/io_log.rs` |

## AlphaZero型探索のネットの照合

ネットの入力の符号化と方策のラベルをPythonの学習器と照合する固定局面集と、ネットの出力をPyTorch、ONNX Runtime、およびTensorRTの間で照合するための推論の結果である。
照合の手順は[ネットの出力の照合の手引き](alphazero-network-check.md)にある。
ONNXのネットそのものとTensorRTのエンジンのキャッシュは、ONNXとONNX Runtimeが形式を定めるので載せない。

| 形式 | 識別と版 | 書き手 | 読み手 | 仕様 |
|---|---|---|---|---|
| 固定局面集`tools/train/zero/fixtures/nn-positions.jsonl` | なし | `minase dev nn-fixture` | minaseのテスト、`tools/train/zero/nn_torch_reference.py`、学習器の試験 | `crates/minase/src/bin/minase/nn_fixture.rs` |
| 固定局面集の推論の結果（`--out`で指定） | なし | `minase dev nn-eval`、`tools/train/zero/nn_torch_reference.py` | `tools/train/zero/nn_compare.py` | `crates/minase/src/bin/minase/nn_eval.rs` |

## 同じ名前で中身が違うファイル

次のファイルは、同じ名前で別の形式を持つ。
読むときは、どのコマンドの出力ディレクトリにあるかで形式を見分ける。

- `manifest.json`と`invocations.json`は、対局ハーネスの実行ディレクトリとSPSAの実行ディレクトリの両方にある。SPSAの`manifest.json`は版の欄を持たず、係数とSPSAの設定を持つ。
- `summary.json`は、対局ハーネスでは累計実行時間と中断状態だけを持ち、学習器では学習の終了報告である。学習器の`summary.json`の欄は学習器ごとに異なる。
- `config.json`と`training.jsonl`は4種の学習器がそれぞれ書き、欄は学習器ごとに異なる。
- `pst-float.npz`は、PSTの学習器が書くと2つの配列、同時学習が書くと5つの配列を持つ。PSTの2つの配列だけを使う読み手は、5つの配列を持つファイルからも同じ名前の2つの配列を読む。
- `latest.npz`と`best.npz`は4種の学習器がそれぞれ書き、収める配列は学習器ごとに異なる。`meta`の`kind`で学習器を見分ける。

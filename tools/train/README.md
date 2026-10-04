# minase-train

本ディレクトリは、minaseの評価関数のうちPST（駒の種類と位置に応じた評価表）を、自己対局の教師データから学習するPythonのツール群である。
エンジン本体はRustで書かれており、学習ツールとはファイルを介してだけやり取りする。
本体の`selfplay_gen`が自己対局の局面と探索値を書き出し、学習ツールがそれを読んでPSTを学習し、エンジンが読み込める重みファイル`crates/minase/nets/pst.bin`の形式で書き出す。
学習の手順、設定項目、および採否の判断は [PSTの学習手順](../../docs/guides/pst-training.md) が定めており、本書は環境の構築とソースの構成だけを説明する。

## 環境の構築

依存関係は [uv](https://docs.astral.sh/uv/) で管理する。
Python 3.12以上、PyTorch、およびNumPyの版は `uv.lock` に固定されており、次のコマンドで `tools/train/.venv` に同じ環境を作れる。

```bash
uv sync --project tools/train
```

コマンドはすべてリポジトリのルートで実行する。
`uv run --project tools/train` は作業ディレクトリを変えないので、設定ファイルやデータのパスはリポジトリのルートから解釈される。
GPUで学習する場合は、この環境からCUDAを利用できる必要がある。

依存関係の版を上げる場合は、`uv lock --upgrade-package <パッケージ名>` で `uv.lock` を更新する。
学習結果の来歴（`environment.json`）にはPyTorchなどの版が記録されるので、版の更新は学習条件の変更として扱う。

## テストの実行

テストは標準ライブラリのunittestで書かれており、`tests/` にある。

```bash
uv run --project tools/train python -m unittest discover -s tools/train/tests
```

## コマンド

`pyproject.toml` は次の5つのコマンドを定義する。
いずれも `uv run --project tools/train <コマンド> --help` で引数を確認できる。

| コマンド | 用途 |
|---|---|
| `pst-workflow` | 学習の準備、自己対局の生成、学習、診断の4工程を、入力と出力の検査和を記録しながら順に実行する。通常の学習はこのコマンドだけを使う |
| `train-pst` | 学習器を直接呼ぶ。初期重みの作成（`init`）、教師値の尺度Kの推定（`estimate-k`）、学習（`train`）を持つ |
| `pst-diagnostics` | 基準のPSTと候補のPSTを、局面帯別の損失、駒の除去、および成りで比較する |
| `taper-report` | 教師データの駒数分布と、序中盤と終盤の2つの重み（端点）を区別して学習できるかどうかを報告する |
| `lookahead-diag` | 先読み教師を使う前に、先読み値と探索値の差の分布と教師Kの変化を調べる |

たとえば学習工程の準備は次のように実行する。
設定ファイルの書き方は、[設定例](pst.example.toml) と学習手順の文書にある。

```bash
uv run --project tools/train pst-workflow prepare --config pst-gen2.toml
```

## ファイル形式

学習ツールが読み書きするファイルは、いずれも先頭4バイトの識別子を名前とする独自のバイナリ形式である。

| 形式 | 内容 | 書き出す側 | 読み込むモジュール |
|---|---|---|---|
| MNSD | 自己対局の局面、探索値、および対局結果 | `selfplay_gen generate`、`lishogi_import` | `data/mnsd.py` |
| MNRS | MNSDの各局面に別の探索で付け直した教師値 | `selfplay_gen rescore` | `data/mnsd.py` |
| MNKF | MNSDの各局面に対応する追加の評価特徴の列 | 段階9の実験ブランチ（masterには書き出す側がない） | `data/mnsd.py` |
| MNPT | PSTの重み、探索用の駒価値、および出力の尺度K | `train-pst` | `data/mnpt.py`、本体の評価関数 |

MNSDには、パスの末尾に `.provenance.json` を付けた来歴ファイルが必須である。
来歴は教師の生成条件と教師値の混合比を記録し、`data/mnsd.py` が検査和とともに検証する。

## ソースの構成

ソースは `src/minase_train/` にあり、機能ごとに3つのサブパッケージへ分かれている。
依存の向きは、`workflow.py` と `diagnostics/` が `pst/` を、`pst/` が `data/` を使う一方向であり、`data/` は学習の手法を知らない。

`data/` は、学習データと重みファイルの読み書き、および学習器と診断が共有する前処理を持つ。

| ファイル | 内容 |
|---|---|
| `data/mnsd.py` | MNSD、MNRS、MNKF、および来歴を検証して読み込み、複数のファイルを1つの通し番号で扱う `Dataset` を提供する。訓練と検証の分割、教師の分類、教師値の付け直しの適用もここで行う |
| `data/mnpt.py` | MNPTの読み書き、駒価値の検証、初期重み、および実数の重みを16ビット整数へ丸める量子化を提供する |
| `data/features.py` | 盤上の駒を「駒状態×升」の特徴番号へ変換し、手番側から見た視点変換と左右の鏡映を行う |
| `data/taper.py` | 盤上の駒数から序中盤と終盤の重みを混ぜる補間係数を計算し、局面を駒数帯へ分ける |
| `data/lookahead.py` | 同じ対局の将来の探索値を、手数差による幾何加重平均で先読み教師値へ変換する |

`pst/` は、PSTの学習そのものを持つ。

| ファイル | 内容 |
|---|---|
| `pst/model.py` | 線形PSTのモデル（単一、2端点、左右鏡映で重みを共有する2端点の3種類）と、評価値を勝率へ換算するロジットを計算する |
| `pst/teacher.py` | 探索値と対局結果を混ぜて教師の勝率を作り、探索値を勝率へ換算する尺度Kを推定する |
| `pst/removal.py` | 駒を1枚取り除いたときの評価差が、初期の重みと同じ符号になることを促す追加の損失を計算する |
| `pst/evaluate.py` | 整数と実数の重みで局面を評価する参照実装であり、量子化誤差の検査とRust側との一致確認に使う |
| `pst/train.py` | 検証損失、学習ループ、および `train-pst` のコマンドラインを持つ |

`diagnostics/` は、学習済みの重みや教師データを調べる報告を持つ。

| ファイル | 内容 |
|---|---|
| `diagnostics/comparison.py` | `pst-diagnostics` の本体。本体の `pst_probe` を呼び、Rustの評価とPythonの参照評価の一致も確認する |
| `diagnostics/taper_report.py` | `taper-report` の本体 |
| `diagnostics/lookahead_teacher.py` | `lookahead-diag` の本体 |

残る2つのファイルは、パッケージ全体から使われる。

| ファイル | 内容 |
|---|---|
| `workflow.py` | `pst-workflow` の本体。基準コミットのワークツリーで `selfplay_gen` と `pst_probe` をビルドし、生成、学習、診断の各工程の入力と出力を検査和で照合する。準備の時点で本パッケージのソースの検査和も記録し、ソースが変わった実行ディレクトリの続行を拒否する |
| `checksum.py` | ファイル全体のSHA-256を計算する |

## テストの構成

`tests/` のテストファイルは、検査する主な関数の定義元モジュールごとに `test_<モジュール名>.py` と名付けている。
たとえば `data/mnsd.py` のテストは `tests/test_mnsd.py` に、`workflow.py` のテストは `tests/test_workflow.py` にある。
複数のテストファイルが使う人工データの作成と、実装から独立した参照計算は `tests/helpers.py` にまとめている。

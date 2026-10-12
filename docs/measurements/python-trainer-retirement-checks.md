# Python学習器の移植と撤去の照合の記録

## 目的

[Python学習器のRustへの移植と撤去の設計書](../plans/python-trainer-retirement.md)の各フェーズについて、Rustへ移植した学習器と診断がPythonの学習器と同じ結果を出すことを一度だけ確かめた結果と、完了条件の検査結果を記録する。
棋力測定ではないので、ペンタノミアル度数やLLRは持たない。

## 環境

学習機はRTX 3060 Ti（8 GB、ドライバ615.71）、CPUはCore Ultra 7 265KF（20コア）、メモリ30 GB、Rust 1.98.0である。
Pythonの照合は本体のチェックアウトの`tools/train/.venv`（torch 2.13.0、NumPy 2.5.2）で行った。
検査の間、同じ機械でlishogiのBot（`minase`、1スレッド）が動いていた。

## 照合の基準

整数の値は完全な一致を求める。
実数の値は、Pythonの値をP、Rustの値をRとして、`|R − P| ≤ 10⁻⁵ + 10⁻⁴·|P|`を求める。
この許容誤差は、[Rust製NNUE学習器](../plans/nnue-trainer-rust.md)をPyTorchと照合したときと同じ式である。
入力のパス、実行環境、実行時間の欄は比較から除く。

## フェーズ0　クレートの改名

`crates/nnue-trainer`を`crates/minase-trainer`へ改名し、既存のサブコマンドを`nnue`の下へ移した。
完了条件の検査として、世代3の1ファイル（`data/gen3/generated-1500000.bin`、2,958,151局面）でP型、幅128、1エポックの`nnue train`を改名の前後で実行し、書き出した`nnue.bin`を比べた。

```sh
minase-trainer nnue train --feature-set P --width 128 \
  --data data/gen3/generated-1500000.bin \
  --init-pst data/pst-fm-joint-training/j75/pst-float.npz \
  --base crates/minase/nets/pst.bin \
  --lookahead-gamma 0.9 --lookahead-plies 40 \
  --batch 16384 --epochs 1 --seed 1 --output-dir <新規のディレクトリ>
```

| 検査 | 結果 |
|---|---|
| 改名前のバイナリで2回学習した`nnue.bin`の一致（基準そのものの再現性） | 一致（SHA-256 `c81290da…`） |
| 改名前と改名後の`nnue.bin`の一致 | バイト単位で一致 |
| `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` | 通過 |
| GPUのテスト（`cargo test --release -p minase-trainer -- --ignored gpu_ --test-threads=1`） | 30件通過 |

## フェーズ1　NNUEの診断の移植

Pythonの`nnue-diagnostics`の`distribution`、`removal`、`moves`を、`minase-trainer diag nnue-distribution`、`diag nnue-removal`、`diag nnue-moves`として移植した。
照合には、[NNUEの完全代替](../plans/nnue-replacement.md)のフェーズ4で学んだ2つのネット（`data/nnue-replacement-train/P`と`data/nnue-replacement-train/HalfKP-hm-b8`）について、当時Pythonが書き出した報告を使った。
基準のPST＋FMは`crates/minase/nets/pst.bin`（SHA-256 `efe2c145…`）である。

Pythonは標本の番号をNumPyの乱数（`numpy.random.default_rng(1).choice`）で選ぶ。
照合では、NumPyで同じ番号を計算してlittle-endianのi64のファイルに書き、報告の`sample_indices_sha256_le_i64`と一致することを確かめたうえで、一時的な引数`--replay-sample`でRustへ渡した。
分布の診断は30ファイル（87,067,079局面）の全局面から2万局面、駒除去差分の診断は同じ30ファイルの検証集合から1万局面である。
着手前後の評価差の診断は、当時の入力（世代3の先頭2万局面のMNSD、SHA-256 `8ddff937…`）を使った。

| 診断 | 比較した値の数 | 結果 |
|---|---|---|
| `nnue-distribution`（2つのネット） | 整数76、実数434 | すべて一致（実数の最大絶対差8.2×10⁻¹²） |
| `nnue-removal`（P型、510,845件の除去） | 整数106、実数44 | すべて一致（実数の最大絶対差8.0×10⁻¹⁴） |
| `nnue-removal`（HalfKP-hm-b8、510,845件の除去） | 整数106、実数40 | すべて一致（実数の最大絶対差1.1×10⁻¹³） |
| `nnue-moves`（P型、2万局面の全合法手） | 整数160,004、実数320,000 | すべて一致（実数の最大相対差1.3×10⁻¹⁵） |
| `nnue-moves`（HalfKP-hm-b8） | 整数160,004、実数320,000 | すべて一致（実数の最大相対差1.2×10⁻¹⁵） |

駒除去差分の報告では、`float`欄のSHA-256だけが異なる。
Pythonはチェックポイントをnpzへ変換したファイルを読み、Rustは学習器のチェックポイント`nnue-float.bin`を直接読むので、識別するファイルが違うためである。
駒除去差分の一括計算と除去後の局面の再評価の最大差は、P型で1.8×10⁻¹²、HalfKP-hm-b8で1.9×10⁻¹²センチポーンであり、診断の内部基準（10⁻⁶センチポーン）を満たす。
v0の参照と逆向きの除去は、P型で3件、HalfKP-hm-b8で0件であり、[NNUEの学習の記録](nnue-replacement-training.md)と同じである。
Rustの報告では、Pythonの報告にあったプローブのコマンドと実行ファイルのSHA-256の欄がなくなる。

実行時間は、分布の診断が778秒、駒除去差分の診断が各772〜793秒、着手前後の評価差の診断が各約27秒だった。
前2者の時間のほぼ全部は、30ファイルの全記録を単一スレッドで復号して検証分割を作る前処理（`Dataset::open`）である（`perf record`で確認）。
この前処理は学習器の`nnue train`と共通であり、本マイルストーンでは速度を変えない。

診断の移植で学習器のデータ経路（特徴の抽出）を共有の関数へ整理したので、フェーズ0と同じ1エポックの学習をP型とHalfKP-hm-b8で実行し、改名前のバイナリの`nnue.bin`とバイト単位で一致することを確かめた。

一致を確かめた後、PythonのNNUEの学習器と診断（`minase_train.nnue`、`nnue-diagnostics`）、PyTorchとの照合スクリプト（`crosscheck_reference.py`、`crosscheck_batch.py`、`rust_float_to_npz.py`）とそのテスト、およびRustの`nnue dump-batch`を削除した。
削除後の検査の結果は次のとおりである。

| 検査 | 結果 |
|---|---|
| `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` | 通過 |
| `cargo test -p minase-trainer` | 124件通過 |
| 残るPythonのテスト（`python -m unittest discover -s tools/train/tests`） | 160件通過 |

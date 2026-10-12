# Python学習器の移植と撤去の照合の記録

## 目的

[Python学習器のRustへの移植と撤去の設計書](../plans/python-trainer-retirement.md)の各フェーズについて、Rustへ移植した学習器と診断がPythonの学習器と同じ結果を出すことを一度だけ確かめた結果と、完了条件の検査結果を記録する。
棋力測定ではないので、ペンタノミアル度数やLLRは持たない。

## 環境

学習機はRTX 3060 Ti（8 GB、ドライバ615.71）、CPUはCore Ultra 7 265KF（20コア）、メモリ30 GB、Rust 1.98.0である。
Pythonの照合は本体の作業ツリー（worktreeではない側）の`tools/train/.venv`（torch 2.13.0、NumPy 2.5.2）で行った。
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
照合には、[NNUEの完全代替](../plans/nnue-replacement.md)のフェーズ4で学習した2つのネット（`data/nnue-replacement-train/P`と`data/nnue-replacement-train/HalfKP-hm-b8`）について、当時Pythonが書き出した報告を使った。
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
この前処理は学習器の`nnue train`と共通であり、本マイルストーンでは高速化の対象にしない。

診断の移植で学習器のデータ経路（特徴の抽出）を共有の関数へ整理したので、フェーズ0と同じ1エポックの学習をP型とHalfKP-hm-b8で実行し、改名前のバイナリの`nnue.bin`とバイト単位で一致することを確かめた。

一致を確かめた後、PythonのNNUEの学習器と診断（`minase_train.nnue`、`nnue-diagnostics`）、PyTorchとの照合スクリプト（`crosscheck_reference.py`、`crosscheck_batch.py`、`rust_float_to_npz.py`）とそのテスト、およびRustの`nnue dump-batch`を削除した。
削除後の検査の結果は次のとおりである。

| 検査 | 結果 |
|---|---|
| `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` | 通過 |
| `cargo test -p minase-trainer` | 124件通過 |
| 残るPythonのテスト（`python -m unittest discover -s tools/train/tests`） | 160件通過 |

## フェーズ2　PSTの学習器

`minase-trainer pst train`は、NNUEの学習器と部品を共有しないPST専用のパイプライン（`crates/minase-trainer/src/pst/`）として実装した。
NNUEと共有するのは、データの読み込み（`src/data/`）と汎用の機械学習の部品（`src/nn/`、Adamの要素ごとの更新、二値交差エントロピー、入力順を保つ並列の`map`など）だけである。
当初はNNUEの線形経路の部品とバッチ供給の仕組みを流用したが、評価関数の異なる学習器のパイプラインを共通化しすぎないよう、利用者の判断で分けた。
分けた前後で、PSTの学習の結果（重み、Adamの状態、各エポックの損失、`pst.bin`）と、NNUEの1エポック学習の`nnue.bin`（両方の特徴集合）、GPUのテスト、NNUEの診断の出力は、いずれもビット単位で変わらなかった。

### 照合の条件

候補Lの条件を、世代3の1ファイル（`data/gen3/generated-1500000.bin`、訓練2,814,063局面、検証144,088局面）に短縮した。
初期値と除去損失の参照は`data/pst-longer-training/pst-base.bin`（候補Lの初期値Pc）、学習率0.03、バッチ16,384、除去損失の係数1000、先読み教師γ=0.9・n=40、λは来歴の値、シード1である。
Pythonの学習器（`minase_train.pst.train`、`--model mirrored`、`--k 1072.6529541015625`、GPU）には、リポジトリにコミットしない一時的な改変を加え、初期重み、各エポックの訓練局面の順序、および指定した回数の更新の後の状態を再生ディレクトリへ書き出させた。
Rustの学習器は一時的な引数`--replay`でその順序と初期重みを読み、`--replay-steps`で指定した回数の更新の後の状態を書き出す。
Rustは、再生ディレクトリの入力ファイルの並びとSHA-256、バッチの大きさ、各エポックの局面が訓練集合と過不足なく一致すること、および初期重みが`--init`から作った値とビット単位で一致することを検査してから使う。

### 結果

| 照合 | 結果 |
|---|---|
| (1) 3回の更新の後の損失、重み、Adamの1次と2次のモーメント | すべて許容誤差内で一致。二値交差エントロピーの最大相対差1.9×10⁻⁸、重みの最大相対差1.1×10⁻⁷。3回目の更新では駒除去差分の罰則が発火し（1.09×10⁻¹⁰）、その値も一致 |
| (2) 5エポック（patience 2）の短縮学習の各エポックの検証損失 | エポック0から5まで差は1.03〜1.11×10⁻⁸（許容誤差の最大0.02%）。最良エポックと停止エポックはともに5で一致 |
| (3) 書き出した整数の重み | `pst.bin`がバイト単位で一致（整数の差0）。`pst-float.npz`の差は最大2.4×10⁻⁴（許容誤差内） |
| 連続2エポックと、1エポックで止めて`--resume`で再開した学習 | `pst.bin`、`pst-float.npz`、各エポックの損失がビット単位で一致 |
| 量子化誤差の検査（検証1万局面） | 平均0.545センチポーン、最大2.08センチポーン（基準は平均2センチポーン以下） |

### 1エポックの時間

PythonのGPUの学習器は、同じ1ファイルで21.8万〜24.1万局面/秒（1エポック約12〜13秒）だった。
Rustの学習器はCPUの18スレッドで、1エポックの訓練が1.69〜1.84秒（153万〜167万局面/秒）であり、Pythonの約7倍速い。
したがって、PSTの学習器にGPUのカーネルは加えない。
5エポックの短縮学習の全体（前処理、検証、書き出しを含む）は、Pythonが68秒、Rustが33秒だった。

最初の実装では1エポックの訓練に17.1秒かかっていた。
`perf record`で調べると、更新の計算は合計1.6秒だけで、残りは、バッチを1つずつ単一スレッドで構築する処理と、PSTが使わないNNUE用の逆索引の構築だった。
PST専用のバッチを、ワーカーごとに容量1のチャネルを持つ構築ワーカーで更新と重ねて作る形に改め、1エポックの訓練が上記の1.7〜1.8秒になった。

### Pythonに加えた一時的な改変

照合に使ったPython側の改変（`tools/train/src/minase_train/pst/train.py`に対する差分）は次のとおりである。

```diff
--- tools/train/src/minase_train/pst/train.py
+++ target/py-replay/tools/train/src/minase_train/pst/train.py
@@ -3,6 +3,7 @@
 from __future__ import annotations
 
 import argparse
+import hashlib
 import json
 import math
 import resource
@@ -58,6 +59,61 @@
 )
 
 
+class ReplayDump:
+    """一度きりのRust照合に使う初期値、訓練順、更新状態を書き出す。"""
+
+    def __init__(self, directory, dataset, model, batch, steps):
+        self.directory = Path(directory)
+        self.directory.mkdir(parents=True, exist_ok=True)
+        if any(self.directory.iterdir()):
+            raise ValueError("--replay-dump directory must be empty")
+        if len(dataset.paths) > 2**32 or np.any(np.diff(dataset.offsets) > 2**32):
+            raise ValueError("replay file and record indices must fit u32")
+        self.steps = steps
+        self.losses = []
+        files = []
+        for path in dataset.paths:
+            with Path(path).open("rb") as stream:
+                files.append({"path": str(Path(path).resolve()),
+                              "sha256": hashlib.file_digest(stream, "sha256").hexdigest()})
+        self.metadata = {"format": "minase-replay-1", "trainer": "pst",
+                         "files": files, "batch": batch, "epochs": 0}
+        weights = expanded_model_weights(model).detach().cpu().numpy()
+        np.savez(self.directory / "initial.npz", middlegame=weights[:, 0], endgame=weights[:, 1])
+        self.write_metadata()
+
+    def write_metadata(self):
+        (self.directory / "replay.json").write_text(json.dumps(self.metadata, indent=2) + "\n")
+
+    def order(self, dataset, indices):
+        files = np.searchsorted(dataset.offsets[1:], indices, side="right")
+        positions = np.column_stack((files, indices - dataset.offsets[files])).astype("<u4")
+        epoch = self.metadata["epochs"] + 1
+        positions.tofile(self.directory / f"epoch-{epoch:03}.positions")
+        np.zeros(indices.size, dtype=np.uint8).tofile(self.directory / f"epoch-{epoch:03}.mirror")
+        self.metadata["epochs"] = epoch
+        self.write_metadata()
+
+    def record(self, bce, removal, total):
+        if self.steps is not None:
+            self.losses.append([bce.item(), removal.item(), total.item()])
+
+    def finish(self, model, optimizer):
+        if self.steps is None or len(self.losses) != self.steps:
+            return False
+        losses = np.asarray(self.losses, dtype=np.float64)
+        arrays = dict(zip(("loss_bce", "loss_removal", "loss_total"), losses.T))
+        state = optimizer.state[model.weight]
+        for prefix, values in (("", model.weight), ("exp_avg_", state["exp_avg"]),
+                               ("exp_avg_sq_", state["exp_avg_sq"])):
+            expanded = values[model.canonical_indices[:FEATURE_COUNT]].detach().cpu().numpy()
+            for column, name in enumerate(("middlegame", "endgame")):
+                arrays[prefix + name] = expanded[:, column]
+        arrays["step"] = np.asarray(int(state["step"].item()), dtype=np.int64)
+        np.savez(self.directory / "python-steps.npz", **arrays)
+        return True
+
+
 def validation_loss(
     model: nn.Module,
     dataset: Dataset,
@@ -192,7 +248,8 @@
     indices: NDArray[np.int64],
     removal_penalty: float,
     removal_reference: Tensor | None,
-) -> TrainEpochResult:
+    replay: ReplayDump | None = None,
+) -> TrainEpochResult | None:
     """訓練レコードを1エポック学習し、重み共有しないモデルには鏡映拡張を施す。"""
     indices = _selected_indices(dataset, indices)
     if not math.isfinite(removal_penalty) or removal_penalty < 0.0:
@@ -205,6 +262,8 @@
     elif removal_reference is not None:
         raise ValueError("zero removal penalty requires reference None")
     order = torch.randperm(indices.size, generator=generator, device=device)
+    if replay is not None:
+        replay.order(dataset, indices[order.cpu().numpy()])
     bce_total = removal_total = total = 0.0
     observations = (
         np.zeros(FEATURE_COUNT, dtype=np.int64) if count_features else None
@@ -250,6 +309,8 @@
         else:
             penalty = bce.new_zeros(())
             loss = bce
+        if replay is not None:
+            replay.record(bce, penalty, loss)
         loss.backward()
         optimizer.step()
         # MNPTの1/8センチポーン単位のi16に収まる範囲で学習する。
@@ -258,6 +319,8 @@
                 raise ValueError("trained weights contain a non-finite value")
             for parameter in model.parameters():
                 parameter.clamp_(min=-4096.0, max=4095.875)
+        if replay is not None and replay.finish(model, optimizer):
+            return None
         bce_total += float(bce.item()) * positions.shape[0]
         removal_total += float(penalty.item()) * positions.shape[0]
         total += float(loss.item()) * positions.shape[0]
@@ -330,6 +393,12 @@
 
 def command_train(arguments: argparse.Namespace) -> None:
     """trainサブコマンドを実行する。"""
+    if arguments.replay_dump is not None and (len(arguments.lr) != 1 or arguments.model != "mirrored"):
+        raise ValueError("--replay-dump requires one --lr and --model mirrored")
+    if arguments.replay_dump is not None and (arguments.train_half is not None or arguments.rescore is not None):
+        raise ValueError("--replay-dump does not support --train-half or --rescore")
+    if arguments.replay_steps is not None and (arguments.replay_dump is None or arguments.replay_steps <= 0):
+        raise ValueError("--replay-steps must be positive and requires --replay-dump")
     validate_lambda_override(arguments.lambda_override)
     if arguments.epochs <= 0 or arguments.batch <= 0 or arguments.validation_sample <= 0:
         raise ValueError("--epochs, --batch, and --validation-sample must be positive")
@@ -396,6 +465,8 @@
 
     selected_rate = arguments.lr[0]
     updates_per_epoch = (training_indices.size + arguments.batch - 1) // arguments.batch
+    if arguments.replay_steps is not None and arguments.replay_steps > updates_per_epoch * arguments.epochs:
+        raise ValueError("--replay-steps exceeds available updates")
     total_updates = 0
     if len(arguments.lr) > 1:
         candidates: list[tuple[float, float]] = []
@@ -437,6 +508,8 @@
     print(f"selected learning rate: {selected_rate:g}")
 
     model = make_model(initial, device, arguments.model)
+    replay = (ReplayDump(arguments.replay_dump, dataset, model, arguments.batch, arguments.replay_steps)
+              if arguments.replay_dump is not None else None)
     optimizer = torch.optim.Adam(model.parameters(), lr=selected_rate)
     generator = torch.Generator(device=device).manual_seed(arguments.seed)
     breakdown = {}
@@ -481,7 +554,10 @@
             indices=training_indices,
             removal_penalty=arguments.removal_penalty,
             removal_reference=removal_reference,
+            replay=replay,
         )
+        if training is None:
+            return
         total_updates += updates_per_epoch
         elapsed = time.perf_counter() - started
         positions_per_second = training_indices.size / elapsed
@@ -512,6 +588,8 @@
             print(f"early stop: epoch={epoch} best_epoch={best_epoch} patience={arguments.patience}")
             break
 
+    if arguments.replay_steps is not None:
+        raise ValueError("training stopped before --replay-steps updates")
     assert best_weights is not None
     print(f"best epoch: {best_epoch} validation_loss={best_loss:.9f}")
 
@@ -612,6 +690,8 @@
     train_parser.add_argument("--seed", type=int, default=1)
     train_parser.add_argument("--validation-sample", type=int, default=10000)
     train_parser.add_argument("--device", default="cuda")
+    train_parser.add_argument("--replay-dump", type=Path, help="一時的なRust照合用の再生ファイルの保存先")
+    train_parser.add_argument("--replay-steps", type=int, help="一時的な照合用に最初のN更新の状態を書き出して終了する")
     train_parser.set_defaults(handler=command_train)
     return parser
 
```

## フェーズ3　FMの学習器、縮小、倍率の焼き込み

FMの学習器は、NNUEとPSTから独立した専用のパイプライン（`crates/minase-trainer/src/fm/`）として実装した。
乱数を含まない変換（FMの量子化、MNPT版3の書き出し、`fm quarter`、`fm scale`）を先に実装し、既存の成果物とのバイト一致を確かめた。

| 検査 | 入力 | 結果 |
|---|---|---|
| `fm quarter` | 候補Faの`data/fm-quarter-current-pst-fa/fm.bin` | 出力が`fm-quarter.bin`と、報告が`fm-quarter.quarter.json`とバイト単位で一致 |
| `fm scale --fm-scale 611` | J75の`data/pst-fm-joint-training/j75/pst.bin`と`pst-float.npz` | 出力が現行の`crates/minase/nets/pst.bin`（SHA-256 `efe2c145…`）とバイト単位で一致 |
| `fm scale --fm-scale 0` | 同上 | 出力が、FMを除いた版2の`data/fm-scale-spsa/pst-j75-pst-only.bin`（`8f45a144…`）とバイト単位で一致 |
| `fm scale`の検証標本の報告 | 同上と、同時学習と同じ10ファイル | 当時の`data/fm-scale-spsa/pst-611.scale.json`と、整数7件が一致し、実数6件の最大差は1.1×10⁻¹⁶ |

検証標本の報告の照合では、Pythonが`numpy.random.default_rng(1).choice`で選んだ1万局面の番号を昇順に並べて、一時的な引数`--replay-sample`でRustへ渡した。
実数の差は、昇順に並べ替えたことによる加算の順序の違いから生じる。
この報告の作成には、10ファイル（約2,900万局面）の前処理を含めて245秒かかった。

### FMの学習器の照合

`minase-trainer fm train`は、固定した整数PSTの上でFMの補正だけを学習する。
照合の条件は、候補Faの条件を世代3の1ファイル（`data/gen3/generated-1500000.bin`、訓練2,814,063局面）に短縮したものである。
固定する初期値は候補Lの`data/pst-longer-training/training/pst.bin`（SHA-256 `b2d2cd1b…`。Faの学習記録の`init`と同じ）、λは1.0を明示し、先読み教師γ=0.9・n=40、ランク32、バッチ4,096、学習率10⁻³、重み減衰10⁻⁴、正則化の係数10⁻⁴、シード1である。
Pythonの学習器（`minase_train.fm.train`、GPU）には、PSTと同じ方式の一時的な改変を加え、初期値（観測マスクを適用した後のV、a、mask）、各エポックの訓練局面の順序、および各局面の左右鏡映の有無を書き出させた。
FMでは鏡映の有無が乱数で決まるので、Rustは再生ファイルの鏡映ビットをそのまま使う。
初期値のVは正規乱数で作られるので、Rustは`initial.npz`の値をそのまま使い、`mask`がRustの数えた観測マスクと完全に一致することと、未観測の行が0であることだけを検査する。

| 照合 | 結果 |
|---|---|
| (1) 3回の更新の後の損失、V、a、AdamWの1次と2次のモーメント | すべて許容誤差内で一致。二値交差エントロピーの最大相対差4.8×10⁻⁸、Vの最大絶対差3.0×10⁻⁸ |
| (2) 4エポック（patience 3）の短縮学習の各エポックの検証損失 | エポック0から4まで差は0.9〜1.5×10⁻⁸。最良エポック2と停止エポック4が一致 |
| (3) 書き出した整数の重み | 埋め込み437,760個のうち91個が±1単位で異なり、残りは一致。指数（10）、符号、PSTの部分、駒価値、Kは一致。`fm-float.npz`のV、a、maskは許容誤差内で一致 |
| 連続2エポックと、1エポックで止めて`--resume`で再開した学習 | `fm.bin`、`fm-float.npz`、各エポックの損失がビット単位で一致 |
| 量子化誤差の検査（検証1万局面） | 補正をそのまま使う場合の平均0.49センチポーン、1/4にした場合の平均0.49センチポーン（基準は平均2センチポーン以下） |

1エポックの時間は、同じ1ファイルで、PythonのGPUが検証を含めて16.1〜16.2秒、RustのCPU（18スレッド）が訓練6.0秒と検証0.05秒（約46万局面/秒）だった。
Rustが約2.6倍速いので、FMの学習器にもGPUのカーネルは加えない。
乱数によらない検査点として、候補Faの10ファイル全体（訓練27,745,843局面）で`fm train`を1エポック実行し、Rustが数えた観測マスクが、当時のPythonの`data/fm-quarter-current-pst-fa/fm-float.npz`の`mask`と完全に一致することを確かめた（観測済み9,522特徴、未観測4,158特徴）。
学習前の検証損失は、Faの学習記録の0.4323040461に対してRustが0.4323039845であり、許容誤差内で一致した。
この条件の1エポックの訓練は60秒（約46万局面/秒）、前処理と書き出しを含む全体は357秒だった。
バッチの構築を更新と重ねるワーカーの部品は、評価関数に依存しないので汎用の部品（`src/nn/`）へ移し、PSTとFMの両方から使う。
移した後も、PSTの学習の結果はビット単位で変わらなかった。

照合に使ったPython側の改変（`tools/train/src/minase_train/fm/train.py`に対する差分）は次のとおりである。

```diff
--- tools/train/src/minase_train/fm/train.py
+++ target/py-replay/tools/train/src/minase_train/fm/train.py
@@ -370,6 +370,62 @@
     return output.with_suffix(".training.json")
 
 
+class ReplayDump:
+    """一度きりのRust照合用に、乱数を消費せず初期値と更新状態を保存する。"""
+
+    def __init__(self, directory, dataset, model, batch, steps):
+        self.directory = Path(directory)
+        self.directory.mkdir(parents=True, exist_ok=True)
+        if any(self.directory.iterdir()):
+            raise ValueError("--replay-dump directory must be empty")
+        if len(dataset.paths) > 2**32 or np.any(np.diff(dataset.offsets) > 2**32):
+            raise ValueError("replay file and record indices must fit u32")
+        self.steps, self.losses, self.mirrors = steps, [], []
+        self.metadata = {"format": "minase-replay-1", "trainer": "fm", "batch": batch, "epochs": 0,
+                         "files": [{"path": str(Path(path).resolve()), "sha256": sha256_file(path).hex()}
+                                   for path in dataset.paths]}
+        np.savez(self.directory / "initial.npz",
+                 V=model.embedding.weight[:FEATURE_COUNT].detach().cpu().numpy(),
+                 a=model.a.detach().cpu().numpy(), mask=model.observed[:FEATURE_COUNT].cpu().numpy())
+        self.write_metadata()
+
+    def write_metadata(self):
+        (self.directory / "replay.json").write_text(json.dumps(self.metadata, indent=2) + "\n")
+
+    def order(self, dataset, indices):
+        files = np.searchsorted(dataset.offsets[1:], indices, side="right")
+        positions = np.column_stack((files, indices - dataset.offsets[files])).astype("<u4")
+        self.metadata["epochs"] += 1
+        epoch = self.metadata["epochs"]
+        positions.tofile(self.directory / f"epoch-{epoch:03}.positions")
+        self.mirrors = []
+        self.write_metadata()
+
+    def mirror(self, selected):
+        self.mirrors.append(selected.astype(np.uint8, copy=True))
+
+    def finish_epoch(self):
+        epoch = self.metadata["epochs"]
+        np.concatenate(self.mirrors).tofile(self.directory / f"epoch-{epoch:03}.mirror")
+
+    def record(self, bce, reg, total):
+        if self.steps is not None:
+            self.losses.append([bce.item(), reg.item(), total.item()])
+
+    def finish(self, model, optimizer):
+        if self.steps is None or len(self.losses) != self.steps:
+            return False
+        self.finish_epoch()
+        arrays = dict(zip(("loss_bce", "loss_reg", "loss_total"), np.asarray(self.losses, dtype=np.float64).T))
+        for prefix, field in (("", None), ("exp_avg_", "exp_avg"), ("exp_avg_sq_", "exp_avg_sq")):
+            for name, parameter in (("V", model.embedding.weight), ("a", model.a)):
+                value = parameter if field is None else optimizer.state[parameter][field]
+                arrays[prefix + name] = value.detach().cpu().numpy()[:FEATURE_COUNT] if name == "V" else value.detach().cpu().numpy()
+        arrays["step"] = np.asarray(int(optimizer.state[model.a]["step"].item()), dtype=np.int64)
+        np.savez(self.directory / "python-steps.npz", **arrays)
+        return True
+
+
 def command_train(args: argparse.Namespace) -> None:
     """対局単位の分割でFMだけを学習し、最良エポックの保存物を出力する。"""
     for name in ("rank", "epochs", "patience", "batch", "validation_sample"):
@@ -379,6 +435,8 @@
         value = getattr(args, name)
         if not math.isfinite(value) or value < 0 or (name == "lr" and value == 0):
             raise ValueError(f"invalid --{name.replace('_', '-')}")
+    if args.replay_steps is not None and (args.replay_dump is None or args.replay_steps <= 0):
+        raise ValueError("--replay-steps must be positive and requires --replay-dump")
     validate_lambda_override(args.lambda_override)
     device = torch.device(args.device)
     if device.type == "cuda" and not torch.cuda.is_available():
@@ -388,6 +446,8 @@
                       lookahead=lookahead_options(args.lookahead_gamma, args.lookahead_plies))
     if not dataset.training_indices.size or not dataset.validation_indices.size:
         raise ValueError("training and validation sets must both be nonempty")
+    if args.replay_steps is not None and args.replay_steps > math.ceil(dataset.training_indices.size / args.batch) * args.epochs:
+        raise ValueError("--replay-steps exceeds available updates")
     teacher_ks, teacher_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
     output = Path(args.output)
     report_path = training_report_path(output)
@@ -399,6 +459,8 @@
         torch.cuda.manual_seed_all(args.seed)
     counts = feature_observations(dataset, args.batch)
     model = FM(args.rank, counts > 0, device)
+    replay = (ReplayDump(args.replay_dump, dataset, model, args.batch, args.replay_steps)
+              if args.replay_dump is not None else None)
     optimizer = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=args.weight_decay)
     generator = torch.Generator(device=device).manual_seed(args.seed)
     steps = (dataset.training_indices.size + args.batch - 1) // args.batch
@@ -439,6 +501,8 @@
         print(f"epoch 0: validation_loss={initial_loss:.9f}", flush=True)
         for epoch in range(1, args.epochs + 1):
             order = torch.randperm(dataset.training_indices.size, generator=generator, device=device)
+            if replay is not None:
+                replay.order(dataset, dataset.training_indices[order.cpu().numpy()])
             total = bce_total = 0.0
             for start in range(0, len(order), args.batch):
                 indices = dataset.training_indices[order[start:start + args.batch].cpu().numpy()]
@@ -447,13 +511,18 @@
                 board, lion = mirror(records["board"], records["lion"])
                 reflected = feature_indices(board, records["stm"], lion)
                 selected = (torch.rand(len(indices), generator=generator, device=device) < 0.5).cpu().numpy()
+                if replay is not None:
+                    replay.mirror(selected)
                 features[selected] = reflected[selected]
                 targets = build_targets(records, teacher_ks, dataset.generations(indices), dataset.teacher_lambdas,
                                         scores=dataset.teacher_scores(indices))
                 optimizer.zero_grad(set_to_none=True)
                 logits, phi = batch_logits(model, mg, eg, k, features, phase_numerators(records["board"]), device)
                 bce = F.binary_cross_entropy_with_logits(logits, torch.as_tensor(targets, device=device))
-                loss = bce + args.lambda_res * phi.square().mean()
+                reg = args.lambda_res * phi.square().mean()
+                loss = bce + reg
+                if replay is not None:
+                    replay.record(bce, reg, loss)
                 if not torch.isfinite(loss):
                     raise ValueError("non-finite training loss")
                 loss.backward()
@@ -470,6 +539,12 @@
                 report["total_steps"] += 1
                 total += float(loss.detach()) * len(indices)
                 bce_total += float(bce.detach()) * len(indices)
+                if replay is not None and replay.finish(model, optimizer):
+                    report.update(status="replay_steps")
+                    save_report()
+                    return
+            if replay is not None:
+                replay.finish_epoch()
             loss, distribution = validation_loss(model, dataset, mg, eg, k, teacher_ks, args.batch, device)
             report["epochs"].append({"epoch": epoch, "train_loss": bce_total / dataset.training_indices.size,
                                      "total_loss": total / dataset.training_indices.size,
@@ -482,6 +557,8 @@
             save_report()
             if should_stop_early(epoch, best_epoch, args.patience):
                 break
+        if args.replay_steps is not None:
+            raise ValueError("training stopped before --replay-steps updates")
         assert best is not None
         model.load_state_dict(best)
         v = model.embedding.weight[:FEATURE_COUNT].detach().cpu().numpy()
@@ -542,6 +619,8 @@
     train.add_argument("--lookahead-gamma", type=float)
     train.add_argument("--lookahead-plies", type=int)
     train.add_argument("--device", choices=("cpu", "cuda"), required=True)
+    train.add_argument("--replay-dump", type=Path, help="一時的なRust照合用の再生ディレクトリ")
+    train.add_argument("--replay-steps", type=int, help="最初のN更新の状態を書き出して終了する")
     train.set_defaults(handler=command_train)
     quarter = commands.add_parser("quarter", help="学習後のMNPT v3の補正を1/4にする")
     quarter.add_argument("--input", required=True, type=Path)
```

## フェーズ4　同時学習器

`minase-trainer joint train`は、PSTとFMの部品を組み合わせた同時学習の専用のパイプライン（`crates/minase-trainer/src/joint/`）である。
学習の開始時に、Pythonの`verify-init`と同じ初期値の検査（`--pst-init`を量子化すると`--base`のPSTに、`--fm-init`を`--base`のKで量子化すると`--fm-reference`のFMに一致すること）を行う。
Pythonの同時学習器はブランチ`pst-fm-joint-training`のコミット`b43ff45`にだけあるので、`git archive`でリポジトリの外（gitに無視される`target/`の下）へ展開し、本体の仮想環境で実行した。

照合の条件は、J75の条件を世代3の1ファイル（`data/gen3/generated-1500000.bin`）に短縮したものである。
`--base`は候補Faの`data/fm-quarter-current-pst-fa/fm-quarter.bin`（SHA-256 `85e4e455…`。J75の学習記録の`base`と同じ）、`--pst-init`は候補Lの`data/pst-longer-training/training/pst-float.npz`、`--fm-init`は候補Faの`fm-float.npz`、`--fm-reference`は候補Faの`fm.bin`、λは0.75を明示し、先読み教師γ=0.9・n=40、バッチ16,384、その他は既定値である。
再生の方式はFMと同じであり、Rustは、`initial.npz`のPSTが`--pst-init`から作った値と、VとaがRustの初期値とビット単位で一致し、`mask`が数え直したマスクと一致することを検査してから使う。

| 照合 | 結果 |
|---|---|
| (1) 3回の更新の後の損失、PST、V、a、AdamWの状態 | 全17配列が許容誤差内で一致。駒除去差分の損失（v0の参照、毎回発火）の最大相対差1.9×10⁻⁸、二値交差エントロピーの最大相対差7.8×10⁻⁸ |
| (2) 3エポックの短縮学習の各エポックの検証損失 | エポック0から3まで差は0.8〜1.5×10⁻⁸。最良エポック2と停止エポック3が一致 |
| (3) 書き出した整数の重み | PSTの2端点は整数の差0。FMの埋め込みは437,760個のうち33個が±1単位で異なり、指数（9）と符号は一致。`pst-float.npz`の5配列は許容誤差内で一致 |
| 連続2エポックと、1エポックで止めて`--resume`で再開した学習 | `pst.bin`と`pst-float.npz`がビット単位で一致 |
| 量子化誤差の検査（検証1万局面） | 学習前0.81センチポーン、書き出した重み0.83センチポーン（Pythonは0.81と0.82）。基準は平均2センチポーン以下 |
| J75の`pst-float.npz`の量子化し直し（`fm scale --fm-scale 1024`、倍率1は量子化し直しだけを行う） | J75の`pst.bin`とバイト単位で一致 |

訓練集合から数え直した観測マスクと`--fm-init`のマスクの不一致は584特徴であり、PythonとRustで同じだった（1ファイルに短縮したため、10ファイルで数えたFaのマスクと異なる）。
1エポックの時間は、PythonのGPUが検証を含めて約29秒、RustのCPUが訓練6.9秒（約41万局面/秒）であり、Rustが約4倍速いので、同時学習器にもGPUのカーネルは加えない。
同時学習器を加えた後も、PSTとFMの学習の結果はビット単位で変わらなかった。

照合に使ったPython側の改変（`b43ff45`の`tools/train/src/minase_train/joint/train.py`に対する差分）は次のとおりである。

```diff
--- a/tools/train/src/minase_train/joint/train.py
+++ b/tools/train/src/minase_train/joint/train.py
@@ -177,12 +177,74 @@
                 raise ValueError(f"{name} loss gradient stayed zero for {GRADIENT_STEPS} steps")
 
 
+class ReplayDump:
+    """一度きりのRust照合用に、追加の乱数を使わず学習入力と状態を保存する。"""
+
+    def __init__(self, directory, dataset, model, batch, steps):
+        self.directory = Path(directory)
+        self.directory.mkdir(parents=True, exist_ok=True)
+        if any(self.directory.iterdir()):
+            raise ValueError("--replay-dump directory must be empty")
+        if len(dataset.paths) > 2**32 or np.any(np.diff(dataset.offsets) > 2**32):
+            raise ValueError("replay file and record indices must fit u32")
+        self.steps, self.losses, self.mirrors = steps, [], []
+        self.metadata = {"format": "minase-replay-1", "trainer": "joint", "batch": batch, "epochs": 0,
+                         "files": [file_record(path) for path in dataset.paths]}
+        np.savez(self.directory / "initial.npz", **model.floating_weights())
+        self.write_metadata()
+
+    def write_metadata(self):
+        write_report(self.directory / "replay.json", self.metadata)
+
+    def order(self, dataset, indices):
+        files = np.searchsorted(dataset.offsets[1:], indices, side="right")
+        positions = np.column_stack((files, indices - dataset.offsets[files])).astype("<u4")
+        self.metadata["epochs"] += 1
+        epoch = self.metadata["epochs"]
+        positions.tofile(self.directory / f"epoch-{epoch:03}.positions")
+        self.mirrors = []
+        self.write_metadata()
+
+    def mirror(self, selected):
+        self.mirrors.append(selected.astype(np.uint8, copy=True))
+
+    def finish_epoch(self):
+        epoch = self.metadata["epochs"]
+        np.concatenate(self.mirrors).tofile(self.directory / f"epoch-{epoch:03}.mirror")
+
+    def record(self, bce, removal, reg, total):
+        if self.steps is not None:
+            self.losses.append([value.item() for value in (bce, removal, reg, total)])
+
+    def finish(self, model, optimizer):
+        if self.steps is None or len(self.losses) != self.steps:
+            return False
+        self.finish_epoch()
+        arrays = dict(zip(("loss_bce", "loss_removal", "loss_reg", "loss_total"),
+                          np.asarray(self.losses, dtype=np.float64).T))
+        for prefix, field in (("", None), ("exp_avg_", "exp_avg"), ("exp_avg_sq_", "exp_avg_sq")):
+            parameter = model.pst.weight
+            value = parameter if field is None else optimizer.state[parameter][field]
+            endpoints = value[model.pst.canonical_indices[:FEATURE_COUNT]].detach().cpu().numpy()
+            arrays[prefix + "middlegame"], arrays[prefix + "endgame"] = endpoints.T
+            for name, parameter in (("V", model.fm.embedding.weight), ("a", model.fm.a)):
+                value = parameter if field is None else optimizer.state[parameter][field]
+                value = value.detach().cpu().numpy()
+                arrays[prefix + name] = value[:FEATURE_COUNT] if name == "V" else value
+        arrays["step"] = np.asarray(int(optimizer.state[model.fm.a]["step"].item()), dtype=np.int64)
+        np.savez(self.directory / "python-steps.npz", **arrays)
+        return True
+
+
 def train_epoch(model: JointModel, dataset: Dataset, teacher_ks: np.ndarray, reference: Tensor,
                 optimizer: torch.optim.Optimizer, generator: torch.Generator,
-                args: argparse.Namespace, report: dict, epoch: int, started: float) -> dict:
+                args: argparse.Namespace, report: dict, epoch: int, started: float,
+                replay: ReplayDump | None = None) -> dict | None:
     """訓練局面を並べ替え、確率0.5の鏡映で1エポック更新する。"""
     device = model.pst.weight.device
     order = torch.randperm(dataset.training_indices.size, generator=generator, device=device)
+    if replay is not None:
+        replay.order(dataset, dataset.training_indices[order.cpu().numpy()])
     totals = dict.fromkeys(("train_loss", "removal_loss", "phi_squared_mean", "total_loss"), 0.0)
     for start in range(0, len(order), args.batch):
         indices = dataset.training_indices[order[start:start + args.batch].cpu().numpy()]
@@ -191,6 +253,8 @@
         board, lion = mirror(records["board"], records["lion"])
         reflected = feature_indices(board, records["stm"], lion)
         selected = (torch.rand(len(indices), generator=generator, device=device) < 0.5).cpu().numpy()
+        if replay is not None:
+            replay.mirror(selected)
         features[selected] = reflected[selected]
         features = torch.as_tensor(features, device=device)
         ratios = torch.as_tensor(phase_ratios(records["board"]), dtype=torch.float32, device=device)
@@ -205,6 +269,8 @@
         loss = bce + args.lambda_res * residual + args.removal_penalty * penalty
         if not bool(torch.isfinite(loss)):
             raise ValueError("non-finite training loss")
+        if replay is not None:
+            replay.record(bce, penalty, args.lambda_res * residual, loss)
         loss.backward()
         record_gradients(model, report)
         optimizer.step()
@@ -214,6 +280,8 @@
             model.pst.weight.clamp_(-4096, 4095.875)
             model.fm.fix_unobserved()
         report["total_steps"] += 1
+        if replay is not None and replay.finish(model, optimizer):
+            return None
         for name, value in zip(totals, (bce, penalty, residual, loss)):
             totals[name] += float(value.detach()) * len(indices)
         if report["total_steps"] % 500 == 0:
@@ -223,6 +291,8 @@
                       if device.type == "cuda" else "")
             print(f"epoch={epoch} updates={report['total_steps']} elapsed_seconds={elapsed:.3f} "
                   f"positions_per_second={positions / elapsed:.1f}{memory}", flush=True)
+    if replay is not None:
+        replay.finish_epoch()
     return {name: value / dataset.training_indices.size for name, value in totals.items()}
 
 
@@ -257,6 +327,8 @@
                 raise ValueError(f"invalid --{name.replace('_', '-')}")
         if args.seed < 0:
             raise ValueError("seed must be nonnegative")
+        if args.replay_steps is not None and (args.replay_dump is None or args.replay_steps <= 0):
+            raise ValueError("--replay-steps must be positive and requires --replay-dump")
         validate_lambda_override(args.lambda_override)
         if device.type == "cuda":
             if not torch.cuda.is_available():
@@ -268,6 +340,8 @@
                           lookahead=lookahead_options(args.lookahead_gamma, args.lookahead_plies))
         if not dataset.training_indices.size or not dataset.validation_indices.size:
             raise ValueError("training and validation sets must both be nonempty")
+        if args.replay_steps is not None and args.replay_steps > math.ceil(dataset.training_indices.size / args.batch) * args.epochs:
+            raise ValueError("--replay-steps exceeds available updates")
         teacher_ks, teacher_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
         torch.manual_seed(args.seed)
         observations = feature_observations(dataset, args.batch)
@@ -277,6 +351,8 @@
             {"params": model.pst.parameters(), "lr": args.pst_lr, "weight_decay": 0},
             {"params": model.fm.parameters(), "lr": args.fm_lr, "weight_decay": args.fm_weight_decay},
         ])
+        replay = (ReplayDump(args.replay_dump, dataset, model, args.batch, args.replay_steps)
+                  if args.replay_dump is not None else None)
         generator = torch.Generator(device=device).manual_seed(args.seed)
         v0 = initial_weights()
         reference = make_removal_reference(v0, v0, device)
@@ -317,7 +393,11 @@
         print(f"epoch 0: validation_loss={initial_loss:.9f}", flush=True)
         for epoch in range(1, args.epochs + 1):
             training = train_epoch(model, dataset, teacher_ks, reference, optimizer, generator,
-                                   args, report, epoch, started)
+                                   args, report, epoch, started, replay)
+            if training is None:
+                report.update(status="replay_steps")
+                save_report()
+                return
             loss, distribution = validation_loss(model, dataset, teacher_ks, args.batch, device)
             previous = report["epochs"][-1]
             overfitting = (previous["train_loss"] is not None and training["train_loss"] < previous["train_loss"]
@@ -335,6 +415,8 @@
                   f"validation_loss={loss:.9f}", flush=True)
             if should_stop_early(epoch, best_epoch, args.patience):
                 break
+        if args.replay_steps is not None:
+            raise ValueError("training stopped before --replay-steps updates")
         model.load_state_dict(best)
         weights = model.floating_weights()
         report["improved_over_epoch_zero"] = best_loss < initial_loss
@@ -378,6 +460,8 @@
     train.add_argument("--lookahead-gamma", type=float)
     train.add_argument("--lookahead-plies", type=int)
     train.add_argument("--device", choices=("cpu", "cuda"), required=True)
+    train.add_argument("--replay-dump")
+    train.add_argument("--replay-steps", type=int)
     train.set_defaults(handler=command_train)
     verify = commands.add_parser("verify-init")
     for name in ("base", "pst-init", "fm-init", "fm-reference", "output"):
```

## フェーズ5　PSTの比較診断

Pythonの`pst-diagnostics`（`minase_train.diagnostics.comparison`）を、`minase-trainer diag pst-compare`として移植した。
Pythonは`minase dev pst-probe`を子プロセスとして呼んでいたが、Rustは`minase`のライブラリを直接呼ぶ。
整数のPST評価の一致の検査は、独立に計算した整数評価とエンジンの評価を比べる形のまま残した。

照合には、候補Lの診断と同じ入力（`data/pst-longer-training/config.toml`の10ファイル、先読みγ=0.9・n=40、λは来歴、基準`pst-base.bin`、候補`training/pst.bin`と`training/pst-float.npz`、各分類と帯で最大10,000局面、シード1）を使った。
2026年10月6日に保存した報告は、その後にプローブへ加わった欄（成り手の捕獲と終局の有無、評価差の内訳）を持たないので、比較の相手には、同じ入力でPythonの診断を現行の`minase dev pst-probe`とともに実行し直した出力を使った。
Pythonは標本をNumPyの乱数で選ぶので、Rustには、保存済みの帯ごとの標本の番号（`diagnostic-indices-generation{g}-band{b}.npy`）を一時的な引数`--replay-sample`で与えた。

| 比較 | 結果 |
|---|---|
| 帯ごとの標本の番号（10ファイル）、標本の局面（`diagnostic-samples.bin`）、代表局面（`representatives.bin`）、駒除去の局面（`removals.bin`） | 13ファイルすべてがバイト単位で一致 |
| `report.json` | キーと並びが一致し、整数3,438件が完全に一致、実数100件の最大相対差4.5×10⁻⁶（許容誤差内） |
| 量子化誤差（10万局面） | 両方とも平均0.5119587779、最大1.7417755127センチポーン |
| 整数評価とエンジンの評価の一致 | 両方とも基準と候補の10万局面すべてで一致 |

実行時間はRustが281秒、Pythonが71秒だった。
Rustの時間の大半は、10ファイルの全記録を単一スレッドで復号して検証分割を作る前処理であり、フェーズ1のNNUEの診断と同じ原因である。
診断は実験ごとに1回だけ走らせるので、本マイルストーンでは高速化の対象にしない。

## フェーズ6　Pythonの撤去

Pythonの学習ツール（`tools/train/`の`minase_train`、テスト、`uv.lock`、設定例）、照合のためだけに加えた一時的な引数（`--replay`、`--replay-steps`、`--replay-sample`）、および呼び出し元がなくなった`minase dev pst-probe`と`minase dev nnue-probe`を削除した。
学習記録の各欄の定義は、学習器のREADME（`crates/minase-trainer/README.md`）へ移した。

撤去の前後で、次の出力がバイト単位で同じであることを確かめた。

| 対象 | 条件 |
|---|---|
| `pst train`の`pst.bin`と`pst-float.npz` | 世代3の1ファイル、候補Lの条件で2エポック |
| `fm train`の`fm.bin`と`fm-float.npz` | 同じ1ファイル、候補Faの条件で2エポック |
| `joint train`の`pst.bin`と`pst-float.npz` | 同じ1ファイル、J75の条件で2エポック |
| `nnue train`の`nnue.bin` | 同じ1ファイル、P型とHalfKP-hm-b8、幅128で1エポック（フェーズ0の改名前と同一） |
| `diag nnue-moves`の報告 | フェーズ1と同じ2万局面 |
| `fm quarter`と`fm scale --fm-scale 611`の出力 | 候補Faの`fm-quarter.bin`と、現行の`crates/minase/nets/pst.bin` |

検査は、`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（1,092件通過）、GPUのテスト（30件通過）である。

書き直した[PSTとFMの学習手順](../guides/pst-training.md)に従い、短縮した条件で最初から最後まで実行した。
基準コミットのworktreeでの生成（16局、1手2,000ノード）と検査、世代3の1ファイルを加えた`pst train`（2エポック）、`diag pst-compare`（各分類と帯で最大1,000局面）、`fm train`、`fm quarter`、`joint train`、`fm scale`がすべて正常に終了した。
候補の初期局面の評価は、診断の`representatives`と学習の`summary.json`の`initial_position_score`でともに36センチポーンだった。

この通しで、`diag pst-compare`の基準にFMを含むMNPT版3（現行の`crates/minase/nets/pst.bin`）を与えると、エンジンの評価にFMの補正が加わって独立の整数参照と一致せず、診断が停止することが分かった。
撤去前のPythonの診断も同じ入力で同じく停止したので、移植の誤りではなく、PSTの比較診断がPSTだけの重み（版2）を前提にしていることによる。
`diag pst-compare`は版3の入力を開始時に明示的なエラーで拒否するように改め、手引きには、PSTの学習と診断の基点に版2の重み（現行の重みでは`data/fm-scale-spsa/pst-j75-pst-only.bin`）を使うことを書いた。

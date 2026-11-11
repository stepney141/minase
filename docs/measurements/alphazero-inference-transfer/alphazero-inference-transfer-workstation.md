# 作業機での推論の転送削減の確認

## 目的

[推論の転送を減らす設計書](../../plans/alphazero-inference-transfer.md)のうち、作業機（GPU 1枚）で行う確認の記録である。
設計書は、ニューラルネットワーク（以下、ネット）の推論でホストとGPUの間の転送を減らす4つの候補を、共通の準備のコミットSの上に1つずつ積み重ねる。
候補は、出力をピン留めメモリで受け取るA、入力をuint8で送るB、方策をFP16で返すC、および方策の出力を35,856ラベルから固定の表で16,108ラベルへ詰めるDである。
確かめたのは次の4点である。

1. 設計書のフェーズ2の最初の確認として、ONNX Runtime 1.31.1で、出力をCUDAのピン留めメモリへ結び付けた推論が、結び付けを保つ方式、呼び出しごとに結び付け直す方式、および束縛（`IoBinding`）ごと作り直す方式のそれぞれで通るかどうか。
2. 設計書のフェーズ5の「出力の確認」として、各候補の推論の出力が、学習器の参照の出力と照合の許容誤差の中で一致すること。
3. Aの自己対局の生成器（以下、生成器）が、1スレッド、GPU 1枚の生成で、親のコミットSと同じ対局のデータを書くこと。
4. 各候補のUSIエンジンが、既定のTensorRTの実行プロバイダで、準備、3回の連続の探索、および最大バッチ1,024での推論器の作り直しと探索を行い、続けてCUDAの実行プロバイダでの探索を終えること。

生成の速さによる採否は、借りた機械で測って別の記録に残す。

## コマンドライン

測定日は2026年11月11日である。
`<コミットのソース>`はブランチ`alphazero-inference-transfer`の各コミットのソースを展開したディレクトリ、`<S>`は作業機の一時ディレクトリである。

### ピン留めメモリの出力の結び付け方

`ort` 2.0.0-rc.13だけに依存する使い捨ての試験プログラムを、リポジトリの外に作って走らせた。
このプログラムは、推論器`OnnxEvaluator`と同じ設定でセッションを作る。設定は、ONNX Runtimeのライブラリの実行時の読み込み、CPUへの退避の禁止、および機器番号0である。TensorRTでは、これにバッチの大きさが最小1、最適128、最大128の最適化プロファイル、FP16、およびエンジンのキャッシュが加わる。
出力の置き場所は、`MemoryInfo::new(AllocationDevice::CUDA_PINNED, 0, AllocatorType::Device, MemoryType::CPUOutput)`で指定した。
入力は、ホストの通常のメモリに0か1の疑似乱数を入れたテンソルを作り、呼び出しごとに`bind_input`で結び付けた。
出力の結び付け方は、次の3つを別々のプロセスで試した。

| 方式 | 出力の結び付け |
|---|---|
| 保つ | 最初に1回だけ`bind_output_to_device`を呼ぶ |
| 結び付け直す | 呼び出しごとに`clear_outputs`の後で`bind_output_to_device`を呼ぶ |
| 作り直す | 呼び出しごとに`create_binding`で束縛を作り、`bind_output_to_device`を呼ぶ |

各方式で、バッチ128の連続の推論（TensorRTは1,000回、CUDAは200回）の後に、バッチを1から128まで1ずつ増やし、続けて128から1まで減らす256回の推論を行った。
すべての呼び出しで、同じ入力を同じセッションの`session.run`でも推論し、方策と価値の出力の差の絶対値の最大を求めた。
ネットは2回目の実行の初期ネット（20ブロック256チャネル）である。

### 出力の照合

各候補について、そのコミットの学習器で3つのネットをチェックポイントから書き出し直し、同じ作業機で参照を作った。
`<ネット>`は、2回目の実行の初期ネット（NASの`cycles-rl2/cycle-0000/`）と、複数GPU化で1サイクル学んだ2つのネット（`multi-gpu-vast/cycle-0001/train/`と`multi-gpu-vast-2/cycle-0001/train/`）である。
SとAは、ネットの入力と出力の名前、型、および形（以下、署名）が同じなので、Sで書き出したネットと参照をAでも使った。

```console
cd <コミットのソース>/tools/train/zero
python -m minase_zero export-onnx --checkpoint <ネット>/checkpoint.pt --out <S>/<候補>/<ネット>.onnx
python -m minase_zero fixture-eval --checkpoint <ネット>/checkpoint.pt --onnx <S>/<候補>/<ネット>.onnx \
  --fixture <コミットのソース>/crates/minase/tests/fixtures/nn-positions.jsonl --out <S>/<候補>/<ネット>.pytorch.json
```

続いて、そのコミットの`minase`で、3つのネットを、3つの実行プロバイダ（CPU、CUDA、TensorRT）と3つのバッチの大きさ（1、7、256）で推論し、その27通りの出力を、そのコミットの`nn_compare.py`で参照と比べた。

```console
minase dev nn-eval --nn-model <S>/<候補>/<ネット>.onnx --backend <cpu|cuda|tensorrt> --batch <1|7|256> \
  --out <出力>.json [--cache-dir <キャッシュ>]
python nn_compare.py --reference <S>/<候補>/<ネット>.pytorch.json --candidate <出力>.json \
  --policy <方式> --policy-tolerance <方策の許容誤差> --value-tolerance <価値の許容誤差>
```

CPUの比較は、S、A、Bでは`--policy logits`、CとDでは`--policy fp16-logits`とし、許容誤差はどちらも方策1×10⁻⁴、価値1×10⁻⁴とした。
`fp16-logits`は、参照のlogitをFP16へ丸めてから比べ、FP16の1目盛りの差を許す方式で、Cのコミットで`logits`と置き換えた。
CUDAとTensorRTの比較は、すべての候補で`--policy probabilities`、方策1×10⁻²、価値5×10⁻²とした。

### 生成の一致

SとAの生成器で、組み替えた初期配置から64局を、1スレッド、GPU 1枚、同時64局、探索32回で生成した。
ネットは2回目の実行の初期ネットである。
`--generation-commit`には、SにはSのコミットを、Aには40桁の0を与えた。
Sは2回生成し、同じコミットの2回が一致することも確かめた。

```console
minase data zero-selfplay --output-dir <出力先> --nn-model <S>/S/rl2-init.onnx --nn-backend tensorrt \
  --generation-commit <コミット> --seed 9100000000 --start-position shuffled-setup --games 64 \
  --target-positions 1000000000 --chunk-games 64 --simulations 32 --playthrough-percent 100 \
  --threads 1 --devices 1 --games-per-thread 64
```

### USIエンジン

各候補の`minase --protocol usi --rules engine-default --search mcts --nn-model <S>/<候補>/mgv1.onnx`に、次の順でコマンドを送った。実行プロバイダの既定はTensorRTである。
`mgv1.onnx`は`multi-gpu-vast/cycle-0001`のネットである。

1. `usi`、`setoption name NnCacheDir value <キャッシュ>`、`isready`、`usinewgame`。
2. `position startpos`から、直前の`bestmove`を手順に足しながら`go nodes 800`を3回。
3. `setoption name NnBatchSize value 1024`、`isready`、`go nodes 4096`。
4. `setoption name NnBackend value cuda`、`setoption name NnBatchSize value 256`、`isready`、`go nodes 800`。
5. `quit`。

## エンジン

| 変種 | コミット | 内容 | 初期ネットのONNXのSHA-256 |
|---|---|---|---|
| S | `353ef0ff` | 書き出し直しのサブコマンドと測定のスクリプト | `8be01e2e…` |
| A | `0b87abe7` | TensorRTとCUDAで、出力をピン留めメモリで受け取る | `8be01e2e…`（Sと同じファイル） |
| B | `2f7b02f4` | 入力をuint8で送り、グラフの中でfloat32へ変換する | `94e589da…` |
| C | `15b0f1d1` | 方策をFP16で返す | `7b764285…` |
| D | `43441d00` | 方策を16,108ラベルへ詰める | `7a9f3ec7…` |

Sで書き出し直した初期ネットは、NASの元のファイルとバイト単位で一致した。
1サイクル学んだ2つのネットは、作業機で書き出し直すと、借りた機械で書き出した元のファイルとSHA-256が異なった。これは、CPUが異なるとバイト列の一致が崩れるという既知の事実のとおりである（[教訓](../../lessons/compare-reexported-onnx-bytes-only-on-the-exporting-machine.md)）。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で、PyTorch 2.11.0（CUDA 13.0）、ONNX Runtime 1.31.1、TensorRT 10.14.1、および`ort` 2.0.0-rc.13を使った。

## 結果

### ピン留めメモリの出力の結び付け方

| 実行プロバイダ | 保つ | 結び付け直す | 作り直す |
|---|---|---|---|
| TensorRT | 2回目の呼び出しで失敗 | 1,256回すべて成功、差0 | 1,256回すべて成功、差0 |
| CUDA | 2回目の呼び出しで失敗 | 456回すべて成功、差0 | 456回すべて成功、差0 |

結び付けを保つ方式は、どちらの実行プロバイダでも、2回目の`run_binding`が次のエラーで失敗した。

```text
gpu_data_transfer.cc:104 ... GPUDataTransfer::CopyTensorAsync(...) const dst_data != src_data was false.
```

これは、ONNX Runtime 1.28.3で出力をピン留めメモリへ結び付けたときと同じ拒否である（[実現性の実測](../alphazero-environment/alphazero-feasibility.md)）。
1.31.1では、1回目の呼び出しは通り、保持された出力の領域が2回目の呼び出しで転送の元と先の両方になって拒まれる。
結び付け直す方式と作り直す方式では、出力の置き場所は`CudaPinned`でホストから読め、方策と価値は`session.run`の出力とビット単位で一致し、非有限の値は0個であった。
したがってAは、束縛を1つ持ち続け、出力の結び付けだけを呼び出しごとにやり直す方式で実装した。

この試験の時間は採否に使わない。
試験プログラムが測った`run_binding`の時間には、`bind_input`で行われる入力の転送が入らず、`session.run`の時間とは範囲が異なるからである。

生データは[alphazero-inference-transfer-workstation/](alphazero-inference-transfer-workstation/)の`pinned-binding-*.txt`にある。

### 出力の照合

5つの変種のそれぞれで、27通りの推論と比較がすべて終了コード0で終わった。
次の表は、参照との差の絶対値の最大を、初期ネット（9通り）と、1サイクル学んだ2つのネット（18通り）に分けて示す。
logitの差は、どの変種でも、丸める前の参照との差である。

| 変種 | ネット | CPUのlogit | CPUの確率 | CUDAの確率 | TensorRTの確率 | TensorRTの価値 |
|---|---|---|---|---|---|---|
| S、A、B | 初期 | 4.8×10⁻⁷ | 1.0×10⁻⁸ | 5.2×10⁻⁶ | 3.4×10⁻⁵ | 2.7×10⁻⁴ |
| S、A、B | 学習済み | 1.0×10⁻⁵ | 7.6×10⁻⁷ | 6.4×10⁻⁴ | 4.7×10⁻³ | 1.4×10⁻² |
| C | 初期 | 1.2×10⁻⁴ | 4.1×10⁻⁶ | 6.1×10⁻⁶ | 3.4×10⁻⁵ | 2.7×10⁻⁴ |
| C | 学習済み | 3.9×10⁻³ | 9.3×10⁻⁴ | 1.1×10⁻³ | 4.5×10⁻³ | 1.4×10⁻² |
| D | 初期 | 1.2×10⁻⁴ | 4.1×10⁻⁶ | 6.1×10⁻⁶ | 3.7×10⁻⁵ | 2.7×10⁻⁴ |
| D | 学習済み | 3.9×10⁻³ | 9.3×10⁻⁴ | 1.1×10⁻³ | 4.7×10⁻³ | 1.4×10⁻² |

Aの27通りの出力は、合法手のlogitと価値のすべてが、Sの出力と値まで一致した。
Aは推論の計算を変えないという設計書の前提のとおりである。
Bは、CPUとCUDAの差がSと同じで、TensorRTでは27通りのうち1通りだけ、表に載せていないlogitの差の最大が変わった（6.6×10⁻²から5.1×10⁻²）。ONNXのファイルが変わるとTensorRTのエンジンが作り直されるので、その差と考えられる（推論）。

Cで方策をFP16へ丸めると、CPUのlogitは、丸める前の参照から最大3.9×10⁻³変わった。
学習済みのネットの合法手のlogitは絶対値の最大が約14.6で、絶対値が8以上16未満の範囲のFP16の1目盛りは2⁻⁷（約7.8×10⁻³）なので、丸めの誤差の上限はその半分の約3.9×10⁻³である。
探索が使う合法手の確率の差は、CPUで最大9.3×10⁻⁴であり、TensorRTの照合の許容誤差1×10⁻²の約10分の1である。
TensorRTの確率と価値の差の最大は、Cを入れてもほぼ変わらなかった。TensorRTは内部の計算がすでにFP16だからである。
Dは、CPUとCUDAの18通りの出力が、合法手のlogitと価値のすべてでCの出力と値まで一致した。詰めた後の添字から読んだlogitが、Cで元のラベルの位置から読んだlogitと同じだったことになる。
BからC、CからDでは、TensorRTの27通りのうち1〜4通りで差の最大が変わったが、どれも上の表の範囲に収まった。

生データは`check-<変種>.jsonl`にある。

### 生成の一致

Sの2回とAの1回の生成は、どれも64局、9,110手、訓練局面2,325で、集計の`summary.json`は3つとも同じ内容であった。
生成器は、1回の生成で、投了以外で終えた対局と投了で終えた対局の2つの教師データのファイル（MNSD形式）を書く。
Sの2回の生成は、この2つのファイルがどちらもバイト単位で一致した。
SとAの生成は、2つのファイルのどちらも、先頭から41〜80バイト目の範囲でだけ異なった。
この範囲は、ヘッダの生成コミットの欄（40バイト）である。
したがってAは、生成コミットの欄を除いて、親のコミットと同じ対局のデータを書いた。

集計とMNSDファイルのSHA-256は`generation-*`にある。

### USIエンジン

A、B、C、Dの4つとも、すべての`isready`が`readyok`を返し、5回の`go`が`bestmove`を返して、終了コード0で終わった。
エラーの行は出なかった。
最大バッチ1,024での推論器の作り直しは、TensorRTのエンジンの作成を含めて41〜44秒かかった。

ログは`usi-<変種>.log`にある。

## 結論

Aは、出力の結び付けを呼び出しごとにやり直す方式でだけ、ONNX Runtime 1.31.1のTensorRTとCUDAで動く。
A、B、C、Dのどれも、作業機では照合の許容誤差を守り、USIエンジンとして動いた。
Aは親と同じ対局のデータを書いた。
作業機はGPUが1枚なので、機器番号が1以上のGPUでのピン留めメモリの結び付けは確かめていない。これは、GPUを4枚積んだ借りた機械の測定で確かめる。

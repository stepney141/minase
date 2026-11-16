# 定例の測定のスクリプトの試運転

## 目的

2回目の強化学習の定例の測定に使うスクリプトが、20ブロック256チャネルの初期ネットで最後まで動くことを、サイクル1を始める前に確かめる（[設計書](../../plans/alphazero-rl-2.md)のフェーズ1の2と「検証」の節）。
対象は、出力の照合の`check_outputs.py`、対局を起動する`run_rl2_match.py`の3つのサブコマンド（短時間条件の持ち時間で対局する`stc`、乱択のエンジンと対局する`random`、2つのネットどうしで対局する`versus`）、および固定の相手のスクリプト`run_match.py`の2つのサブコマンド（`alphabeta`、`hachu`）である。
スクリプトは、[定例の測定のスクリプトのディレクトリ](alphazero-rl-2-routine-scripts/)と、[強さを測る固定の相手のスクリプトのディレクトリ](../mcts-fixed-opponents/mcts-fixed-opponents-scripts/)にある。
対局は各1ペアだけであり、強さを測る測定ではない。

## コマンドライン

作業機のブランチ`alphazero-rl-2`の作業ツリーの根で、bashで次のように実行した。
ネットは初期ネット`/mnt/synology_nas/minase/alphazero/cycles-rl2-az/cycle-0000/model.onnx`（以下、`$N`）である。

```bash
source tools/train/zero/env.sh
export XDG_CACHE_HOME=$HOME/.cache
A=/mnt/synology_nas/minase/alphazero
N=$A/cycles-rl2-az/cycle-0000/model.onnx
C=46ce05ff328104ed86432469e5a4c08b78ef85c3
R=docs/measurements/alphazero-rl-2/alphazero-rl-2-routine-scripts/run_rl2_match.py
F=docs/measurements/mcts-fixed-opponents/mcts-fixed-opponents-scripts/run_match.py
H=target/release/minase
taskset -c 0-9 python3 -I docs/measurements/alphazero-rl-2/alphazero-rl-2-routine-scripts/check_outputs.py \
  --net $N --reference $A/cycles-rl2-az/cycle-0000/check/pytorch.json --out data/alphazero-rl-2/check-c0000
python3 -I $R stc --harness $H --name alphazero-rl-2-smoke-stc --mcts-commit $C --net $N --seed 2026111601 --pairs 1
python3 -I $R random --harness $H --name alphazero-rl-2-smoke-random --mcts-commit $C --net $N --seed 2026111602 --pairs 1
python3 -I $R versus --harness $H --name alphazero-rl-2-smoke-versus --mcts-commit $C --net $N --baseline-net $N \
  --seed 2026111603 --pairs 1
python3 -I $F alphabeta --harness $H --name alphazero-rl-2-smoke-alphabeta --mcts-commit $C --net $N --nodes 500 \
  --seed 2026111604 --pairs 1 --concurrency 2
python3 -I $F hachu --harness $H --name alphazero-rl-2-smoke-hachu --mcts-commit $C --net $N \
  --hachu /home/stepney141/board-games/hachu-debian/hachu --depth 3 --seed 2026111605 --pairs 1 --concurrency 2
```

`check_outputs.py`は作業ツリーの`target/release/minase`（コミット46ce05ff）を使った。
ONNX RuntimeのCPUの推論は全コアを使うので、照合だけは`taskset`で10コアに限った。
5つの対局の実行ディレクトリは`data/matches/<測定名>.tar.zst`へ圧縮して残した。

## エンジン

MCTSのエンジンは、コミット46ce05ff（ブランチ`alphazero-rl-2`で生成器の規則を`lishogi`に切り替えたコミット）の`minase`に`--search mcts`を指定したものであり、ネットは初期ネット（SHA-256 `4b430a4414d250f96094c475dc489efbc865afa20e1c9bfc5b2a958cae4abf07`）、`NnBatchSize`は16、推論は既定のTensorRTのFP16である。
`stc`と`alphabeta`の相手は、コミット45d5db54のαβ探索と、そのコミットに埋め込まれたNNUEである。
`hachu`の相手には、[強さを測る固定の相手の設計書](../../plans/archived/mcts-fixed-opponents.md)で基準を測ったときと同じHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、バイナリのSHA-256は`10db1299f95626fe58875b1c24a1cee6acce0eee5fb42ed8b17ce1373a9dcbdf`）を使い、規則オプションは既定の設定とした。
`random`の相手は乱択のエンジン`random`、`versus`の相手は同じ初期ネットのMCTSである。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月16日14時10分から14時16分に行った。
ONNX Runtimeは1.31.1、TensorRTは10.14.1である。
同じ機械では、学習器の試験を走らせるCodexの作業が並行していた。
STC以外の対局はシミュレーション回数、ノード数、または深さで制限したので、結果は負荷の影響を受けない。
持ち時間で制限するSTCの1ペアは、負荷のある状態で指した。

## 結果

出力の照合は、6つの組がすべて許容誤差に収まり、41.7秒で終わった。
差の最大は次のとおりである。

| 実行プロバイダ | バッチの大きさ | logitの差 | 合法手の確率の差 | 価値の差 |
|---|---|---|---|---|
| ONNX RuntimeのCPU | 1、7、256 | 3.04×10⁻⁵ | 7.94×10⁻⁷ | 8.87×10⁻⁸ |
| TensorRTのFP16 | 1 | 4.82×10⁻⁴ | 1.28×10⁻⁵ | 2.49×10⁻⁴ |
| TensorRTのFP16 | 7 | 5.84×10⁻⁴ | 1.12×10⁻⁵ | 2.52×10⁻⁴ |
| TensorRTのFP16 | 256 | 5.10×10⁻⁴ | 1.11×10⁻⁵ | 2.63×10⁻⁴ |

5つの対局は、どれも1ペア2局を最後まで指した。
エンジンの異常終了、時間切れ、反則、および拒否着手は、どの対局でも0件であった。
10局すべてが王駒の捕獲で終わった。規則E1のとおり、詰みでは終局していない。

| サブコマンド | 規則 | ペンタノミアル度数 | 2局の手数 | 実行時間 |
|---|---|---|---|---|
| `stc` | `lishogi` | [1, 0, 0, 0, 0] | 合計181手 | 22.0秒 |
| `random` | `lishogi` | [0, 0, 0, 0, 1] | 合計243手 | 3.1秒 |
| `versus` | `lishogi` | [0, 0, 1, 0, 0] | 合計782手 | 102.0秒 |
| `alphabeta` | `lishogi` | [1, 0, 0, 0, 0] | 112手と117手 | 16.5秒 |
| `hachu` | `L1,L3,P0,P5,P6,R2,E1,E2` | [1, 0, 0, 0, 0] | 30手と85手 | 8.9秒 |

ペンタノミアル度数は、候補のMCTSから見たペアの得点（0、0.5、1、1.5、2）ごとのペア数である。
`stc`、`random`、および`versus`の手数は、ハーネスが出力した着手の数の和である。
実行時間はハーネスの`summary.json`の`active_wall_time_ns`であり、対局エンジンのビルドと、TensorRTの推論エンジンの作成を含まない。

## 結論

定例の測定のスクリプトは、今のバイナリと20ブロック256チャネルの初期ネットで最後まで動く。
`summarize.py`はノードか深さで制限した固定の相手との対局だけを集計するので、STCの対局の結果はハーネスの出力と`minase match report`で読む。

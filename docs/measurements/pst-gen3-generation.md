# 世代3の教師データの生成

## 目的

[世代3の計画](../plans/pst-gen3.md)のフェーズ1として、採用構成S0の生成器で世代3の教師データを5ファイル生成し、候補G3とG23の学習の入力を確定する。

## コマンドライン

次の設定を`data/gen3.toml`に保存し、本体の作業ツリーからワークフローの準備と生成を順に実行した。

```toml
[run]
directory = "data/gen3"
base_commit = "44ae7cb2f2c40b442eb4dc6dcdcb6b6d96ef52a7"
data = [
  "data/gen0.bin",
  "data/gen1-s100000.bin",
  "data/gen1-s200000.bin",
  "data/gen1-s300000.bin",
  "data/gen1-s400000.bin",
  "data/gen1-s500000.bin",
  "data/strength-stage7/gen2/generated-600000.bin",
  "data/strength-stage7/gen2/generated-700000.bin",
  "data/strength-stage7/gen2/generated-800000.bin",
  "data/strength-stage7/gen2/generated-900000.bin",
  "data/strength-stage7/gen2/generated-1000000.bin",
]

[generate]
seeds = [1100000, 1200000, 1300000, 1400000, 1500000]
games = 7500
nodes = 100000
concurrency = 16
max_ply = 4000
hash_mb = 16
random_moves = 0
```

設定の`[train]`と`[diagnose]`の節は採用工程の値であり、生成には使わない。

```console
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 \
  tools/train/.venv/bin/python tools/train/pst/pst_workflow.py prepare --config data/gen3.toml
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 \
  tools/train/.venv/bin/python tools/train/pst/pst_workflow.py generate --run-dir data/gen3
```

ワークフローは各シードについて、次の形の生成と検証を順に実行した。

```console
data/gen3/generator/target/release/selfplay_gen generate --output data/gen3/generated-<シード>.bin \
  --seed <シード> --games 7500 --nodes 100000 --random-moves 0 --concurrency 16 \
  --max-ply 4000 --hash-mb 16
data/gen3/generator/target/release/selfplay_gen inspect data/gen3/generated-<シード>.bin
```

## エンジン

生成器は、ワークフローの準備がmaster `44ae7cb`に固定したworktree `data/gen3/generator` でビルドした`selfplay_gen`（SHA-256 `871ee1ed…`）である。
評価関数は採用PST（ネットの検査和`8125a343…`）、規則は`L0,P0,R1,E0`、教師探索は1局面100,000ノードである。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）で、2026年10月2日11時41分から10月3日5時30分に生成した（約17時間49分）。
生成の途中で、半分割の学習（GPU）と固定ノード数の対局（同時3対局）を並行して実行した。
着手はノード数で決まるので、並行した負荷は所要時間だけに影響する。

## 結果

| 基本シード | 記録のある対局 | 手数上限で破棄 | 記録局面 | 捕獲・成りの除外率 | 詰み帯の除外率 | 再出現の除外率 | SHA-256 |
|---|---:|---:|---:|---:|---:|---:|---|
| 1100000 | 7,224 | 257 | 3,051,912 | 11.39% | 0.70% | 0.19% | `27b5e110…` |
| 1200000 | 7,232 | 253 | 3,037,742 | 11.48% | 0.70% | 0.18% | `b1965f6d…` |
| 1300000 | 7,272 | 212 | 3,041,697 | 11.85% | 0.73% | 0.19% | `8b62b9b8…` |
| 1400000 | 7,238 | 237 | 3,061,444 | 11.55% | 0.71% | 0.19% | `b21f3ba4…` |
| 1500000 | 7,229 | 235 | 2,958,151 | 11.79% | 0.71% | 0.20% | `73d24909…` |

合計は記録のある対局36,195局、記録局面15,150,946である。
記録のある対局は`inspect`がファイル内に数えた対局数である。終局した対局（7,500局から手数上限による破棄を引いた数）との差の数局から数十局は、除外の結果、記録局面が1つも残らなかった対局である。
`inspect`は5ファイルすべてについて、ヘッダの生成コミット、ネットの検査和、規則、教師ノード数、およびシードが設定と一致し、記録の整合に問題がないことを示した。

## 結論

世代3の5ファイルは、世代2と同じ生成条件で記録と検証を完了した。
候補G3とG23の学習の入力として使う。

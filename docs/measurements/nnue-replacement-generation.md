# NNUEの初代のネットのための浅い教師データの生成

## 目的

[NNUEによるPST＋FMの完全代替（初代のネット）](../plans/nnue-replacement.md)のフェーズ1として、基準M（master `8d0e382`、手の順序付けの改善を採用した版）のエンジンで1局面20,000ノードの教師データを生成し、幅64と幅128のネットの学習の入力を確定する。
試走と最初の本生成は着手時のmaster `6b97cc26`で行い、本生成は2ファイルを作った時点で停止して、生成器を`8d0e382`に改めてやり直した。両コミットの重みファイルは同一であり、探索だけが異なる。
まず64局の試走で記録局面/秒、理由別の除外率、および探索値の分布を世代3（1局面100,000ノード）と並べて測り、その速度からファイル数を確定して本生成を行う。

## コマンドライン

学習ワークフローは、ブランチ`nnue-replacement`のフェーズ0の修正（MNPT版3の基準ファイルを受理する`read_mnpt`）を含む`tools/train/`であり、そのコミットに固定した別のworktree `data/worktrees/nnue-replacement-gen` から実行した。
worktreeの`data`は本体の`data/`へのシンボリックリンクであり、実行ディレクトリは本体の`data/`に置かれる。
仮想環境は本体の`tools/train/.venv`を使い、`PYTHONPATH=tools/train/src`でworktreeのソースをimportした。

試走の設定は`data/nnue-replacement-trial.toml`、本生成の設定は`data/nnue-replacement-gen.toml`である。
両者は`run.directory`、`generate.seeds`、および`generate.games`だけが異なる。
本生成の`[generate]`は次のとおりである。

```toml
[generate]
seeds = [2000000, 2100000, 2200000, 2300000, 2400000, 2500000, 2600000, 2700000, 2800000, 2900000,
         3000000, 3100000, 3200000, 3300000, 3400000, 3500000, 3600000, 3700000, 3800000, 3900000]
games = 7500
nodes = 20000
concurrency = 12
max_ply = 4000
hash_mb = 16
random_moves = 0
```

`run.base_commit`は、試走では`6b97cc26ff42ca3ebad77276fbcbb3df5df49006`、やり直した本生成では`8d0e382021ea00caec93cbf1159597167b7c5152`であり、`run.data`は世代2と世代3の10ファイルである。
`[train]`と`[diagnose]`は生成に使わない。

```console
cd data/worktrees/nnue-replacement-gen
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 PYTHONPATH=tools/train/src \
  nice -n 19 tools/train/.venv/bin/python -m minase_train.workflow prepare --config data/nnue-replacement-trial.toml
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 PYTHONPATH=tools/train/src \
  nice -n 19 tools/train/.venv/bin/python -m minase_train.workflow generate --run-dir data/nnue-replacement-trial
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 PYTHONPATH=tools/train/src \
  nice -n 19 tools/train/.venv/bin/python -m minase_train.workflow prepare --config data/nnue-replacement-gen.toml
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 PYTHONPATH=tools/train/src \
  nohup nice -n 19 tools/train/.venv/bin/python -m minase_train.workflow generate --run-dir data/nnue-replacement-gen \
  > data/nnue-replacement-gen.generate.log 2>&1 &
```

ワークフローは各シードについて、次の形の生成と検証を順に実行する。

```console
data/nnue-replacement-gen/generator/target/release/minase data selfplay generate \
  --output data/nnue-replacement-gen/generated-<シード>.bin --seed <シード> --games 7500 --nodes 20000 \
  --random-moves 0 --concurrency 12 --max-ply 4000 --hash-mb 16
data/nnue-replacement-gen/generator/target/release/minase data selfplay inspect data/nnue-replacement-gen/generated-<シード>.bin
```

## エンジン

生成器は、ワークフローの準備がコミットに固定したworktreeでビルドした`minase`である。
試走の生成器（`6b97cc26`）のSHA-256は`11aa3135…`、やり直した本生成の生成器（`8d0e382`）は`b5aaa410…`である。同じコミットでもビルド先のディレクトリが異なると実行ファイルは一致しない（停止した最初の本生成の生成器は`5203f748…`だった）。
評価関数は現行のPST＋FM（`crates/minase/nets/pst.bin`、MNPT版3、ファイルのSHA-256 `efe2c145…`、ヘッダの本体検査和 `6c1bc5d5…`）、規則は`L0,P0,R1,E0`、教師探索は1局面20,000ノードである。
基準Mに結び付けて記録する値は次のとおりである。

- 重みファイル`crates/minase/nets/pst.bin`のSHA-256: `efe2c1455f19c1b019051d672d37204e714ed1043daec88b44422c3a7f780c53`
- 復号した探索用駒価値47個（リトルエンディアンi32）のSHA-256: `b1d8c7b0406215bcb44bb19c9a0757b52ae742d9ec3ed66f845113c287668c25`
- 出力用K: 1072.6529541015625
- J75の整数化前のPST（`data/pst-fm-joint-training/j75/pst-float.npz`）のSHA-256: `828f8c0064e6bface1b85324cdc47bb0b26f037fa8b2f89eb97dbc2949dfebdd`

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
lishogiのBot（Threads 4、同時2局）と共用するため同時対局数は12とし、`nice -n 19`で実行した。
試走の間、Botは1局を対局中だった。
試走は2026年10月9日8時20分に64局を36秒で生成した。
最初の本生成は2026年10月9日8時24分に開始し、2ファイルを検証した時点の同日10時50分に利用者の指示で停止した。やり直した本生成は2026年10月11日11時52分に開始した。

## 結果

### 試走（64局、基本シード2000000）

| 項目 | 試走（20,000ノード） | 世代3のシード1500000（100,000ノード） |
|---|---:|---:|
| 記録局面/秒 | 596.2 | 238.8（同時16） |
| 探索局面/秒 | 837.8 | 370.0（同時16） |
| 1局あたりの記録局面 | 343 | 409 |
| 平均手数 | 486 | 604 |
| 捕獲・成りの除外率 | 15.03% | 11.79% |
| 詰み帯の除外率 | 0.77% | 0.71% |
| 再出現の除外率 | 0.21% | 0.20% |
| 探索値の標準偏差 | 3,475 | 2,612 |
| 探索値の絶対値の中央値 / 90% / 99%点 | 1,184 / 6,035 / 11,441 | 1,006 / 4,590 / 7,982 |
| 絶対値3,000以上の割合 | 29.5% | 21.8% |
| 手数上限で破棄 | 1 / 64 | 235 / 7,500 |

試走の速度は、64局を同時12で生成したため最後の回が埋まらず、定常の速度より低い側の値である。
探索値の分布が世代3より広いのは、基準Mの評価関数がFMの補正で評価値の尺度を大きくしていること（[倍率の調整](../plans/fm-scale-spsa.md)）と、浅い探索が静的評価の値をそのまま返しやすいことの両方が考えられる。
除外率は合否には使わない。

### ファイル数の確定

1ファイル（7,500局）の記録局面は約2.5M局面（343×7,500）、所要時間は約1.2時間（2.5M÷596）と見込む。
設計書の規則により、24時間以内に完走するファイル数（20）と1億局面に要するファイル数（40）の小さい方として、20ファイルに確定した。
見込みの新規局面数は約5,100万局面であり、既存の約2,920万局面と合わせた抽出比率では新規が約64%になる。

### 停止した最初の本生成（`6b97cc26`、削除済み）

2026年10月9日に検証を終えた2ファイルの結果は次のとおりであり、生成器を改めたので学習には使わず、利用者の指示で削除した。

| 基本シード | 終局した対局 | 手数上限で破棄 | 記録局面 | 捕獲・成りの除外率 | 詰み帯の除外率 | 再出現の除外率 | 平均手数 | 所要時間（時間） |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 2000000 | 7,291 | 209 | 2,890,112 | 12.83% | 0.66% | 0.13% | 584 | 1.08 |
| 2100000 | 7,281 | 219 | 2,864,981 | 12.78% | 0.66% | 0.13% | 585 | 1.05 |

2ファイルの合計は14,572局、5,755,093局面であり、記録局面/秒は750と試走の596より速い。
試走の64局は最後の回が埋まらず、平均手数も486と短かったので、速度と1局あたりの局面数を低く見積もっていた。
この速度では1ファイル約1.1時間、20ファイルで約5,800万局面になり、既存の約2,920万局面と合わせた抽出比率では新規が約66%になる。
捕獲・成りの除外率は12.8%で世代3の11.8%に近く、試走の15.0%は標本が小さいことによるぶれだった。

### 本生成（`8d0e382`）

（生成中。完了後に各ファイルの記録局面、除外率、SHA-256、および所要時間を書く。）

## 結論

（生成の完了後に書く。）

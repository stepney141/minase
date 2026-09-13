# 段階7の世代2教師データの生成

## 目的

[棋力向上段階7](../plans/strength-stage7.md)で採用した鏡映共有モデルを使って世代2を5シード生成し、再学習と後続段階の起点となる教師データを保存する。

## コマンドライン

次の設定を`data/strength-stage7/gen2.toml`に保存し、ワークフローの準備と生成を順に実行した。

```toml
[run]
directory = "data/strength-stage7/gen2"
base_commit = "960c56bcbb6088d0f0466d670ceea36dffeda5b7"
data = [
  "data/gen0.bin",
  "data/gen1-s100000.bin",
  "data/gen1-s200000.bin",
  "data/gen1-s300000.bin",
  "data/gen1-s400000.bin",
  "data/gen1-s500000.bin",
]

[generate]
seeds = [600000, 700000, 800000, 900000, 1000000]
games = 7500
nodes = 100000
concurrency = 16
max_ply = 4000
hash_mb = 16
random_moves = 0

[train]
model = "mirrored"
k = 1072.6529541015625
learning_rate = 3.0
epochs = 10
batch = 16384
seed = 1
lambda = 0.75
device = "cuda"
validation_sample = 10000

[diagnose]
sample_size = 10000
seed = 1
```

```console
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 \
  tools/train/.venv/bin/python tools/train/pst/pst_workflow.py prepare \
  --config data/strength-stage7/gen2.toml
OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PYTHONUNBUFFERED=1 \
  tools/train/.venv/bin/python tools/train/pst/pst_workflow.py generate \
  --run-dir data/strength-stage7/gen2
```

2026年9月13日7時9分48秒（日本標準時）に、最初の生成器のプロセスが起動した。
ワークフローは基本シード600000、700000、800000、900000、1000000を順に処理し、各シードで7,500局を生成する。
各生成器の引数は次の形となり、シードと出力名だけが変わる。

```console
data/strength-stage7/gen2/generator/target/release/selfplay_gen generate \
  --output data/strength-stage7/gen2/generated-600000.bin --seed 600000 \
  --games 7500 --nodes 100000 --random-moves 0 --concurrency 16 \
  --max-ply 4000 --hash-mb 16
```

実際の起動には絶対パスを使い、引数列と作業ディレクトリを実行ディレクトリの`commands.jsonl`へ保存する。
生成が成功した各ファイルを同じ生成器の`inspect`で検査し、ヘッダの世代コミット、重みの検査和、規則、探索ノード数、シード、および非空の記録数を検証した後に、検査和と記録数を`generated-<seed>.json`へ保存する。

基本シードの派生前の入力範囲は600001〜607500、700001〜707500、800001〜807500、900001〜907500、1000001〜1007500である。
[着手時の監査](strength-stage7-seed-audit.md)に加え、鏡映共有候補のSTCの投入上限3,000ペアと、LTCの未保存分を含む入力20912628までの予約も照合した。
全37,500入力値について、過去の生成・測定および相互の範囲との重複と、派生後のシード衝突はともに0件である。
照合の証拠は`data/strength-stage7/gen2-readiness-audit.json`に保存した。

## エンジン

生成構成は、[短時間測定](strength-stage7-mirror-stc.md)と[長時間測定](strength-stage7-mirror-ltc.md)で採用した`960c56bcbb6088d0f0466d670ceea36dffeda5b7`である。
このコミットを固定したworktreeで`cargo build --release --locked --bin selfplay_gen`を実行し、専用のtargetディレクトリにビルドした。
生成器のSHA-256は`41f6837ddd45e76a14f53da40bffc32a6aa449b898a10919bafd850b9620cdd0`、埋め込み重みのファイル全体のSHA-256は`b26e7841b168ed8e9f4bd8fe0094b7e0dbf6f74d76fae4ee95c19c18fa269b5c`である。
同コミットの手数上限の既定値は600だが、今回の実行は[試行生成](strength-stage7-plycap-trial.md)で選んだ`--max-ply 4000`を明示する。
規則は`engine-default`の`L0,P0,R1,E0`、教師探索は1手100,000ノード、対局中のランダム着手注入は0である。
開始局面の8〜16手のランダム化は維持する。

生成の準備時に、既存6ファイルの検査和と、生成器のコミット、worktreeに変更がないこと、および実行ファイルの検査和を確認した。
後続の診断に使う`pst_probe`は`f4b3480e1e236ff5f946f26382266bedc82a5006`へ別に固定し、SHA-256は`94c33e504c5cc0d7c67b44697f7210c6c068677ec5a1a93b8b3801002113151e`である。
設定、既存データの来歴、Pythonツールの検査和、および両バイナリの固定情報は`prepared.json`へ保存した。

## 環境

CPUはIntel Core Ultra 7 265KFで、物理20コア、論理20コアである。
1対局の探索は1スレッド、同時対局数は16、置換表は各16 MBとする。
生成中は同じ計算機で学習と採否測定を実行しない。

## 結果

準備は終了コード0で完了し、生成器と診断器の固定情報、両worktreeに変更がないこと、および既存6ファイルの検査和を再確認した。
基本シード600000と700000の生成と`inspect`は、いずれも終了コード0で完了した。
現在は基本シード800000を生成中であり、全5ファイルのうち2ファイルの検証を終え、合計5,629,808局面を保存した。
完了したシードの結果を次に示す。
生成時間は生成器が報告する経過秒数であり、`inspect`の時間は含まない。

| 基本シード | 記録局面数 | 終局局数 | 手数上限による破棄局数 | 記録を含む局数 | 生成時間（秒） |
|---|---:|---:|---:|---:|---:|
| 600000 | 2,855,537 | 7,354 | 146 | 7,333 | 12,856.147939 |
| 700000 | 2,774,271 | 7,348 | 152 | 7,324 | 12,730.828819 |

各シードの勝敗と、終局しても記録対象の局面がなかった局数を次に示す。
終局局数に手数上限による破棄局数を加えると、各7,500局になる。

| 基本シード | 先手勝ち | 後手勝ち | 引き分け | 記録を含まない終局 |
|---|---:|---:|---:|---:|
| 600000 | 3,641 | 3,553 | 160 | 21 |
| 700000 | 3,616 | 3,553 | 179 | 24 |

記録を含まない終局の内訳は、基本シード600000が先手勝ち17局と後手勝ち4局、基本シード700000が先手勝ち17局と後手勝ち7局である。
個別の局番号や除外理由は、いずれも保存資料からは特定できない。
終局理由の内訳は次のとおりである。

| 終局理由 | 基本シード600000の局数 | 基本シード700000の局数 |
|---|---:|---:|
| 王駒の捕獲による勝ち | 21 | 23 |
| 詰みによる勝ち | 6,848 | 6,804 |
| 反復規則による勝ち | 316 | 329 |
| 駒の枯渇による勝ち | 9 | 13 |
| 反復規則による引き分け | 157 | 179 |
| 駒の枯渇による引き分け | 3 | 0 |

これ以外の終局理由は0局であり、ランダム着手の注入も0回だった。
記録局面の結果成分は次のとおりである。

| 基本シード | 手番側の負け | 引き分け | 手番側の勝ち |
|---|---:|---:|---:|
| 600000 | 1,430,558 | 49,309 | 1,375,670 |
| 700000 | 1,391,165 | 46,166 | 1,336,940 |

`inspect`の出力を準備時の情報と照合し、世代コミット、規則、教師探索ノード数、シード、および記録数の一致を確認した。
ファイル全体のSHA-256は次のとおりである。

| 基本シード | SHA-256 |
|---|---|
| 600000 | `2f29579a170462a820c8174f1db348306bd6d586aaa33914cc79ba07d380e4c5` |
| 700000 | `a6d737ca4ecc14667c8f61c015ac3388f933dd4f9ccc43110970c67c20db2634` |

両ファイルとも全レコードの独立集計で`inspect`の集計値が一致し、監査結果を`data/strength-stage7/gen2/seed-600000-audit.json`と`seed-700000-audit.json`に保存した。
探索した局面数とノード数、および記録局面の生成速度は次のとおりである。

| 基本シード | 探索局面数 | 探索ノード数 | 記録局面の生成速度（局面/秒） |
|---|---:|---:|---:|
| 600000 | 3,990,221 | 399,022,100,000 | 222.114510 |
| 700000 | 3,929,144 | 392,914,400,000 | 217.917548 |

実行ディレクトリは`data/strength-stage7/gen2/`、準備の標準出力は`data/strength-stage7/gen2-prepare.log`、生成ワークフローの標準出力は`data/strength-stage7/gen2-generate.log`に保存する。
各生成器の進捗と集計は実行ディレクトリの`generate-<seed>.log`、検査結果は`inspect-<seed>.log`に保存する。

## 結論

2ファイルの生成と検証を完了し、残る3ファイルを生成している。
5ファイルの検証が完了してから世代0〜2を合わせた端点の識別性診断へ進む。

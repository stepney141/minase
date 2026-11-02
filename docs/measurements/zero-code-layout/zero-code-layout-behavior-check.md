# AlphaZero型探索のコードの配置の整理の前後での挙動の一致の確認

## 目的

[コードの配置の整理](../../plans/archived/zero-code-layout.md)の前と後で、αβ探索のbench、AlphaZero型の自己対局の出力、および教師データの集計が一致することを確かめる。

## コマンドライン

測定日は2026年11月2日である。
αβ探索のbenchは、次のコマンドで実行した。
一致の判定では、経過時間と速さの欄を除いた、15局面の各行と集計の行を比べた。

```console
taskset -c 3 target/release/minase dev bench --depth 6 --threads 1
```

自己対局は、次のコマンドを毎回新しい出力先で実行し、出力の全ファイルをバイト単位で比べた。
ネットは、強化学習の再出発に使った、ランダムに初期化した最初のネットである（SHA-256 `b2de222c…7c48`）。

```console
target/release/minase data zero-selfplay --output-dir <新しい出力先> \
  --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0000.onnx --nn-backend cpu \
  --generation-commit 0000000000000000000000000000000000000000 --seed 20261102 \
  --games 8 --chunk-games 8 --target-positions 1000000000 --simulations 32 \
  --playthrough-percent 100 --threads 1 --games-per-thread 8
```

教師データの集計は、整理の前の`teacher_stats`の実行ファイルと、整理の後の`minase data teacher-stats`の標準出力を、次の3組の入力についてバイト単位で比べた。

- `tools/train/zero/fixtures/zero-selfplay-small.mnsd`。
- 上の自己対局の`chunk-00000.mnsd`と`chunk-00000.resigned.mnsd`。
- NASのサイクル28の塊0と塊1（`chunk-00000`と`chunk-00001`）の学習ファイルと投了ファイル（886局、399,211局面）。王手限定の3手以内の勝ち775局、投了47局、千日手の引き分け60局を含む。

## エンジン

整理の前はmasterのコミット43bf6962である。
`teacher_stats`の実行ファイルは、同じソースからビルドしたものである（SHA-256 `e2c36aab…0abe0`）。
整理の後はブランチ`zero-code-layout`のコミット1b62e81bである。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

Intel Core Ultra 7 265KF（20コア、ハイパースレッディングなし）、メモリ30 GBの作業機である。
benchは1スレッド、自己対局は`--threads 1`でONNX RuntimeのCPU実行プロバイダを使った。

## 結果

benchは、整理の前と後の両方で探索ノード数の合計が52,746であり、15局面のノード数、最善手、および評価値がすべて一致した。

自己対局は8局、137局面で、全局が王手限定の3手以内の勝ちで終わった。
1回の所要時間は、作業機の負荷により16秒から83秒であった。
整理の前の同じコミットの2回の実行で、`chunk-00000.json`、`chunk-00000.mnsd`、`chunk-00000.resigned.mnsd`、`config.json`、および`summary.json`がバイト単位で一致し、整理の後の実行もこれらと一致した。
`chunk-00000.mnsd`のSHA-256は`58316beb…afcd`、`chunk-00000.resigned.mnsd`は`0ca5ac98…3ef0`である。

教師データの集計は、3組の入力のすべてで標準出力がバイト単位で一致した。

## 結論

整理は探索、自己対局、および集計の結果を変えていない。

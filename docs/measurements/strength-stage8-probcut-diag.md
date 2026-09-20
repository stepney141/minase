# strength-stage8-probcut-diag

## 目的

[棋力向上段階8](../plans/strength-stage8.md)で見送ったProbCutについて、長時間条件（LTC）の到達深さに相当するbench深さ8で対象になり得るノードの割合を数え、次期の候補として扱うかの材料を残す。

## コマンドライン

診断は本番の経路へ入れず、[診断差分](strength-stage8-probcut-diag/diagnostic.patch)を段階8の最終構成へ当てた別ターゲットディレクトリのビルドで数えた。

```console
git apply docs/measurements/strength-stage8-probcut-diag/diagnostic.patch
CARGO_TARGET_DIR=target/diag cargo build --release --bin bench
target/diag/release/bench --depth 6 --threads 1 --repetitions 1
target/diag/release/bench --depth 8 --threads 1 --repetitions 1
```

## エンジン

段階8の最終構成（SEEによる捕獲手の枝刈りとcorrection historyを採用したコミット`0c1bd76`）である。
`bench`は規則セット`engine-default`を固定で使い、総ノード数は深さ6で1,813,911、深さ8で10,128,396である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、rustc 1.98.0であり、2026年9月21日に1スレッドで測った。
件数は決定的であり、経過時間は使わない。同じ機械で固定200ペアの対局測定が並行していたので、最低優先度で実行した。

## 結果

分母は、置換表の即時打ち切りを通過した通常探索のノードである。
対象になり得るノードは、残り深さ5以上の非PVノードのうち、βが詰み帯の外にあり、null moveの直後でないものである。

| bench深さ | 通常探索のノード | 対象になり得るノード | うち静的評価が`β − 2p`以上 |
|---|---:|---:|---:|
| 6 | 266,674 | 1,981（0.74%） | 1,579（0.59%） |
| 8 | 1,779,697 | 26,247（1.47%） | 19,744（1.11%） |

## 結論

ProbCutの対象になり得るノードは、LTCの到達深さに相当する深さ8でも1.47%であり、発動率の基準の5%に届かない。
深さ6の0.74%からは約2倍に増えているので、到達深さがさらに伸びた段階で測り直す価値はあるが、現時点では次期の候補に加えない。

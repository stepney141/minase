# strength-stage8-correction-bench

## 目的

[棋力向上段階8](../plans/strength-stage8.md)のフェーズ5として、SEEによる捕獲手の枝刈りを採用した構成のbench深さ6で、静的評価の補正の更新の条件（変種A、B、C）、鍵、および重みを、設計書の規則で決め直す。

## コマンドライン

計測は本番の経路へ入れず、[再診断の計測差分](strength-stage8-rediag-tools/diagnostic.patch)を採用構成へ当てた別ターゲットディレクトリのビルドで行い、候補は[評価スクリプト](strength-stage8-rediag-tools/evaluate.py)で評価した。

```console
git apply docs/measurements/strength-stage8-rediag-tools/diagnostic.patch
CARGO_TARGET_DIR=target/diag cargo build --release --bin bench
target/diag/release/bench --depth 6 --threads 1 --repetitions 1
MINASE_STAGE8_DIAG=data/experiments/stage8-correction-rediag/passive target/diag/release/bench --depth 6 --threads 1 --repetitions 1
MINASE_STAGE8_DIAG=data/experiments/stage8-correction-rediag/verified MINASE_STAGE8_VERIFY=1 target/diag/release/bench --depth 6 --threads 1 --repetitions 1
```

全27表の値は[results.txt](strength-stage8-correction-bench/results.txt)と[results.json](strength-stage8-correction-bench/results.json)にある。

## エンジン

SEEによる捕獲手の枝刈りの採用構成（探索がコミット`c1c7227`と同一のコミット`f0e23cf`）である。improvingフラグは不採用なので含まない。
`bench`は規則セット`engine-default`を固定で使い、深さ6の総ノード数は1,556,722である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、rustc 1.98.0であり、2026年9月20日に1スレッドで測った。
件数は決定的であり、経過時間は使わない。

## 結果

3回の実行の総ノード数はすべて1,556,722であり、4標本は参照探索の有無でバイト単位で一致した。

### 観測点の内訳

観測点は157,576件である。「利き」は手番側の王駒に相手の利きが届くことを表す。

| 保存する値 | 非捕獲・利きなし | 非捕獲・利きあり | 捕獲・利きなし | 捕獲・利きあり |
|---|---:|---:|---:|---:|
| 正確な値 | 420 | 10 | 146 | 3 |
| 上限 | 3,639 | 108 | 2,883 | 10 |
| 下限 | 29,908 | 41 | 118,222 | 2,186 |

最善手が捕獲手である観測点は123,450件（78.3%）を占める。
共通の評価集合（最善手が捕獲手でなく、王駒に利きが届かない観測点）は33,967件で、補正なしの平均絶対誤差は357である。

### 共通の評価集合での改善率

変種Aは全観測点で更新し、変種Bは共通の評価集合の観測点でだけ更新し、変種Cは変種Bに上限かつ最善手が捕獲手の2,883件を加えて更新する。
列は重み`min(depth, 8) / 分母`の分母である。

| 変種 | 鍵 | 32 | 64 | 128 |
|---|---|---:|---:|---:|
| A | 王駒の升の組 | −0.44% | −3.03% | −4.84% |
| A | 駒種別の枚数の組 | 1.99% | −0.55% | −2.43% |
| A | 組合せ | 1.82% | −0.64% | −2.46% |
| B | 王駒の升の組 | 15.27% | 14.37% | 13.60% |
| B | 駒種別の枚数の組 | 15.41% | 14.15% | 13.00% |
| B | 組合せ | 14.98% | 13.66% | 12.46% |
| C | 王駒の升の組 | 14.03% | 13.24% | 12.57% |
| C | 駒種別の枚数の組 | 15.23% | 14.12% | 13.02% |
| C | 組合せ | 14.80% | 13.64% | 12.49% |

選ばれた表（変種B、駒種別の枚数の組、分母32）の非ゼロ補正の参照率は、適用するノード235,779件に対して157,818件、66.9%である。
表の鍵の数は15局面の合計で648個、1回の探索あたり平均43個である。

## 結論

静的評価の補正は、更新の条件を変種B（最善手が捕獲手でなく、手番側の王駒に相手の利きが届かない）、鍵を双方の駒種別の枚数の組と手番側、重みを`min(depth, 8) / 32`として実装へ進める。改善率は15.41%で基準の10%以上、非ゼロ補正の参照率は66.9%で基準の5%以上である。
全観測点で更新する変種Aは、非捕獲手の判断に使う観測点での改善率が−4.84%から1.99%であり、補正が捕獲手による駒得を学習して予測に寄与しないことを確かめた。
上限の観測点を最善手によらず使う変種Cは、どの鍵と重みでも変種Bを上回らなかった。

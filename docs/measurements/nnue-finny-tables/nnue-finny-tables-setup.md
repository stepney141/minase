# Finny tablesの測定の準備（負荷の条件とシードの固定）

## 目的

[Finny tablesの設計書](../../plans/archived/nnue-finny-tables.md)のフェーズ0として、基準のコミットで探索時間とサイクル数を測り、最終測定に使う`bench`の深さ、教師生成の1回の局数K、およびシードの列を固定する。`bench`は、固定した15局面を固定の深さで探索するUSIの独自コマンドである。
あわせて、基準のコミットでの全再計算の割合`refresh_fraction`と、ネットを読み込むだけで終了する実行（以下、読み込みだけの起動）のサイクル数を記録する。

## コマンドライン

測定には、付属資料の[`finny_bench.py`](nnue-finny-tables-setup/finny_bench.py)を使った。
このスクリプトは、基準と候補のバイナリを1組として、それぞれを`taskset -c 3 perf stat -x, -e cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/LLC-load-misses/u,cpu_core/dTLB-load-misses/u`の下で実行し、組ごとのサイクル数と探索結果をJSON Linesへ書く。
組iでは、iが偶数なら基準を、奇数なら候補を先に実行する。
探索結果は、`bench`では局面ごとの深さ、ノード数、最善手、および評価値であり、教師生成では対局数、探索ノード数の合計、および教師のファイルのSHA-256である。組の2つの実行で探索結果が一致しなければ、スクリプトは終了コード1で止まる。
教師生成は、作業ディレクトリのコミットを教師のファイルに書くので、両方のバイナリを基準のコミットに固定した変更のないworktreeから起動する。
組iの教師生成の基本シードは、`--seed-base`に`i×--games`を足した値である。
本測定は基準のコミットだけを対象とするので、基準のバイナリを基準と候補の両方に渡した。

```console
git worktree add --detach data/nnue-finny-tables/cwd f2eb2fbf
B=data/nnue-finny-tables/bin/minase-f2eb2fbf
H=data/nnue-finny-tables/nets/d0d1bf33fa7aef3faa40fb0696872eb632a959313d9b03a6df216d99b4f5441d.bin
A=data/nnue-finny-tables/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin
python3 finny_bench.py loadonly --binary $B --net $H --sha256 d0d1bf33fa7aef3faa40fb0696872eb632a959313d9b03a6df216d99b4f5441d --cwd data/nnue-finny-tables/cwd
python3 finny_bench.py loadonly --binary $B --net $A --sha256 415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9 --cwd data/nnue-finny-tables/cwd
python3 finny_bench.py pairs --workload generate --baseline $B --candidate $B --net $H --pairs 2 --games 2 \
  --seed-base 9050000000 --out gen-k2.jsonl --cwd data/nnue-finny-tables/cwd
python3 finny_bench.py pairs --workload bench --baseline $B --candidate $B --net $H --depth 19 --pairs 1 \
  --out bench-halfkp-19.jsonl --cwd data/nnue-finny-tables/cwd
python3 finny_bench.py pairs --workload bench --baseline $B --candidate $B --net $A --depth 21 --pairs 1 \
  --out bench-a-21.jsonl --cwd data/nnue-finny-tables/cwd
```

`loadonly`は、標準入力に`setoption name RuleSet value JCSA`、`setoption name EvalFile value <ネット>`、`isready`、`quit`を送り、`info string network_sha256`の行が期待するSHA-256と一致すること、`readyok`が出ること、および終了コードが0であることを確かめてから、サイクル数を出力する。1回の`loadonly`の呼び出しの中で、この起動を3回繰り返す。
`bench`の深さを選ぶときは、上の`perf stat`を付けずに、同じバイナリを論理CPU 3に固定して`bench depth <深さ> threads 1 repetitions 1`を1回ずつ実行し、出力の`summary`行の経過時間を読んだ。測った深さは、HalfKP_vmのネットで16、18、19、20、A型のネットで16、18、20、21、22である。
JSON Linesの全行は、付属資料の[`gen-k2.jsonl`](nnue-finny-tables-setup/gen-k2.jsonl)、[`bench-halfkp-19.jsonl`](nnue-finny-tables-setup/bench-halfkp-19.jsonl)、および[`bench-a-21.jsonl`](nnue-finny-tables-setup/bench-a-21.jsonl)にある。

## エンジン

基準は、設計書のブランチ`nnue-finny-tables`の分岐元であるmasterのコミット`f2eb2fbfd8a6f2ca32ef7bd497e3dd615d3b69cf`である。
バイナリは、リポジトリのrelease設定（`target-cpu=native`、インライン閾値1000、LTO有効、`codegen-units=1`）で`invariants`機能なしにビルドし、SHA-256は`416da18a94440ea490740bfd74396066131f54c0deb6d4a52e86407d2c62e4f8`である。
HalfKP_vmのネットは、[HalfKP_vmの動作の確認](../nnue-tatara-framework-and-halfkp-vm/nnue-tatara-framework-and-halfkp-vm-halfkp-vm-check.md)で学んだ幅256のネット（SHA-256は`d0d1bf33fa7aef3faa40fb0696872eb632a959313d9b03a6df216d99b4f5441d`）である。
A型のネットは、[NNUEの評価関数の手引き](../../guides/evaluation-nnue.md)が探索の測定の標準とする第3世代のネット（SHA-256は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）である。
どちらのネットも、NASからローカルのディスクへ複写して読んだ。
`bench`の規則セットは`JCSA`であり、教師生成の規則セットは`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）に固定されている。

## 環境

測定機はIntel Core Ultra 7 265KF（性能コア8、高効率コア12、論理CPUは計20、メモリ30 GiB）で、Linux `6.18.63-3-lts`、Rust `1.100.0`、perf `7.3.6`である。
探索ワーカーは1、教師生成の同時対局数は1である。置換表は、教師生成が16 MiB、`bench`が`USI_Hash`の既定値である。
すべての実行を性能コアの論理CPU 3に固定した。
`/sys/kernel/mm/transparent_hugepage/enabled`は`[always] madvise never`だった。
2026年11月26日の23時44分から23時58分に測定した。
測定の前に、対局の測定、SPSA、学習、および教師生成が同じ機械で動いていないことを確かめた。
測定と並行して、フェーズ1の実装のビルドとテストが同じ機械の別のコアで動いていた。本測定は負荷の条件を決めるためのものであり、採否の判定には使わないので、並行したまま測った。

## 結果

`bench`の深さごとの経過時間と`refresh_fraction`は次のとおりである。`refresh_fraction`は、累算器の更新のうち、探索の根での更新と、少なくとも1視点で全再計算が起きた更新とが占める割合である。

| ネット | 深さ | ノード数 | 経過時間（秒） | 1局面あたり（秒） | `refresh_fraction` |
|---|---|---|---|---|---|
| HalfKP_vm | 16 | 7,807,381 | 9.57 | 0.64 | 0.036408 |
| HalfKP_vm | 18 | 20,454,063 | 25.35 | 1.69 | 0.034822 |
| HalfKP_vm | 19 | 42,904,203 | 52.65 | 3.51 | 0.036353 |
| HalfKP_vm | 20 | 65,389,505 | 80.77 | 5.38 | 0.035361 |
| A型 | 16 | 3,371,474 | 3.77 | 0.25 | 0.000005 |
| A型 | 18 | 8,521,032 | 9.27 | 0.62 | 0.000002 |
| A型 | 20 | 22,648,771 | 24.29 | 1.62 | 0.000001 |
| A型 | 21 | 37,458,176 | 40.48 | 2.70 | 0.000000 |
| A型 | 22 | 68,425,958 | 72.96 | 4.86 | 0.000000 |

HalfKP_vmの深さ19では、累算器の更新41,118,010回のうち1,494,759回が全再計算だった。A型の全再計算は、15局面の根の15回だけである。
HalfKP_vmの深さ5の`bench`で測った1.17%（設計書の「minaseで全再計算が起きる頻度」の節）と比べて、深い探索では割合が約3倍になる。

サイクル数は次のとおりである。読み込みの比は、読み込みだけの起動3回の平均サイクル数を、各負荷の1回あたりの平均サイクル数で割った値である。

| 負荷 | 1回の実行のサイクル数 | 読み込みだけの起動のサイクル数 | 読み込みの比 |
|---|---|---|---|
| HalfKP_vmの教師生成（K=2） | 143,639,786,725（4回の平均） | 982,268,357 | 0.68% |
| HalfKP_vmの`bench`（深さ19） | 292,473,220,991（2回の平均） | 982,268,357 | 0.34% |
| A型の`bench`（深さ21） | 224,894,870,319（2回の平均） | 133,950,689 | 0.06% |

読み込みだけの起動の3回は、HalfKP_vmが974,617,646、993,918,950、978,268,474サイクル、A型が133,273,975、133,656,095、134,921,996サイクルだった。
教師生成の2組（組0と組1の基本シードは9050000000と9050000002）では、2局の探索ノード数の合計が21,820,000と20,140,000だった。同じバイナリの2回の実行のサイクル数の差（後に実行した側から先に実行した側を引いた値）は、組0が−1,579,817,431サイクル（−1.1%）、組1が+922,047,435サイクル（+0.7%）だった。
すべての組で、2つの実行の探索結果は一致した。

## 結論

最終測定の負荷とシードの条件を次のとおり固定する。

- `bench`の深さは、1局面あたりの平均の探索時間が2秒以上になる最小の深さとし、HalfKP_vmのネットで19、A型のネットで21とする。
- 教師生成の1回の局数Kは2とする。
- 設計書は、読み込みだけの起動のサイクル数がある負荷の1回の実行の5%を超えたら、その負荷の深さまたは局数を増やすと定めている。上の3つの負荷ではいずれも1%未満だったので、深さと局数は増やさない。
- 組iの基本シードは、最終測定の前に基準のバイナリどうしで組の数を求める測定（設計書のフェーズ4の手順1）では`9030000000 + 2i`、最終測定では`9040000000 + 2i`とする（iは0から数える）。どちらの列も、本測定で使った`9050000000`からの列と重ならない。
- 基準のコミット`f2eb2fbf`での`refresh_fraction`は、HalfKP_vmの深さ19の`bench`で0.036353、A型の深さ21の`bench`で0.000000（根の15回だけ）である。

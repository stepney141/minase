# FM採用後のtarget-cpuによるbenchの速度差とFMの核の命令

## 目的

[fearless_simdの適用可能性の調査](../research/fearless-simd-feasibility.md)の一部として、FM（factorization machine）の補正を採用した現行のエンジンで、`.cargo/config.toml`が指定する`-C target-cpu=native`のビルドと、移植可能な`x86-64`のビルドの探索速度の差を測る。
あわせて、FMの4つの核（累算と補正を担う小さな関数）の機械語の命令の内訳を、両ビルドと、fearless_simdで書いた検証用のcrateについて数える。
FM採用前の同じ速度比較は[bench-target-cpu](bench-target-cpu.md)にある。

## コマンドライン

masterの`bc0617a`を`data/worktrees/fearless-simd`のworktreeへ取り出し、`RUSTFLAGS`で`.cargo/config.toml`の指定を上書きして2種類のビルドを別々のターゲットディレクトリに作った。
releaseの設定はワークスペースのルートの`lto = true`と`codegen-units = 1`である。

```console
RUSTFLAGS="-C target-cpu=<native|x86-64>" CARGO_TARGET_DIR=<構成ごとの場所> \
  nice -n 19 cargo build --release -j1 --bin minase
```

2つのビルドを1巡ずつ交互に5巡実行し、構成ごとに5個の値の中央値を比べた。
benchの値は、計測外のウォームアップ1回の後に測った3回の中央値である。
命令数とサイクル数は、各構成を`perf stat -e instructions:u,cycles:u`の下で1回だけ実行して数えた。

```console
taskset -c 3 <構成のminase> dev bench --depth 9 --repetitions 3
taskset -c 3 perf stat -x, -e instructions:u,cycles:u <構成のminase> dev bench --depth 9 --repetitions 1
```

命令の内訳は、`crates/minase/src/eval/pst/fm.rs`の`add`、`remove`、`replace`、`correction`に`#[inline(never)]`を付けた診断用ビルドを構成ごとに別のターゲットディレクトリへ作り、`objdump -d`で各関数の範囲を切り出して命令名（ニーモニック）ごとの出現数を数えた。
検証用のcrateは[fearless-simd-feasibility/](../research/fearless-simd-feasibility/)にあり、`cargo build --release`のAVX2向けの単相化を同じ方法で数えた。
手順の全体は[bench-native-vs-x86-64.sh](../research/fearless-simd-feasibility/bench-native-vs-x86-64.sh)にある。

## エンジン

2構成とも`bc0617a`であり、違いは`target-cpu`だけである。
規則はbench既定の`engine-default`、評価重みは埋め込みの`nets/pst.bin`（採用済みの候補LのPSTと候補Faの補正1/4のFM、[設計書](../plans/fm-quarter-current-pst.md)）を使った。
検証用のcrateはfearless_simd 1.1.0とfearless_simd_macros 0.1.0に依存する。

## 環境

2026年10月8日、Intel Core Ultra 7 265KF（性能コア8、高効率コア12、論理コア20）の環境で測定した。
Rustは`rustc 1.98.0 (88d9e12ae 2026-08-18)`を使った。
性能コアのCPU 3へ固定し、探索ワーカーは1、置換表は256 MiBとした。
測定の間、別の測定（`fm-scale-spsa-ltc`、同時対局数16）が走っており、負荷平均は約17から19だった。
このため絶対値は[FM採用前の記録](bench-target-cpu.md)（無負荷で1,928,611）より低いが、2構成は同じ負荷の下で交互に測っている。

## 結果

### 速度

ノード数は2構成で一致した（5,711,806）。
[生出力](../research/fearless-simd-feasibility/bench-native-vs-x86-64.txt)から求めた構成ごとの中央値は次のとおりである。
幅は、5巡の最大と最小の差の中央値に対する割合である。

| 構成 | bench NPS | x86-64に対する比 | 5巡の幅 | 命令数 | サイクル数 |
|---|---:|---:|---:|---:|---:|
| x86-64 | 1,820,173 | 1.000 | 0.23% | 68.91 G | 16.65 G |
| native | 1,851,399 | 1.017 | 0.63% | 66.15 G | 16.32 G |

5巡とも`native`が`x86-64`を上回り、巡ごとの比は1.5%から2.2%の範囲にあった。
命令数は`x86-64`が4.2%多く、サイクル数は2.0%多い。
`target-cpu`はエンジン全体の機械語に作用するので、この差はFMだけの差ではない。
FM採用前の記録では両構成の差が0.3%で測定の幅に埋もれていた。

### FMの核の命令の内訳

`native`のビルドでは、4つとも256ビットのAVX2命令へ自動ベクトル化されていた。

| 核 | `native`の主要な命令（回数） |
|---|---|
| `add` | `vpmovsxwd` 4、`vpaddd` 4、`vmovdqu` 4 |
| `remove` | `vpmovsxwd` 4、`vpsubd` 4、`vmovdqu` 8 |
| `replace` | `vpmovsxwd` 8、`vpsubd` 4、`vpaddd` 4、`vmovdqu` 8 |
| `correction` | `vpmulld` 4、`vpmuldq` 8、`vpmovzxdq` 16、`vpaddq` 9、`vextracti128` 5 |

`x86-64`のビルドでは、累算の3つはSSE2の128ビット命令になり、`correction`はスカラーになった。

| 核 | `x86-64`の主要な命令（回数） |
|---|---|
| `add` | `punpcklwd` 8、`psrad` 8、`paddd` 8、`movdqu` 16 |
| `remove` | `punpcklwd` 8、`psrad` 8、`psubd` 8、`movdqu` 16 |
| `replace` | `punpcklwd` 16、`psrad` 16、`psubd` 8、`paddd` 8、`movdqu` 16 |
| `correction` | `imul` 64、`movslq` 60、`movsbl` 32、`add` 32 |

検証用のcrateでfearless_simdの総称関数として書いた2つの核のAVX2向けの単相化は次のとおりである。

| 核 | fearless_simdのAVX2向けの主要な命令（回数） |
|---|---|
| `replace` | `vpmovsxwd` 8、`vpsubd` 4、`vpaddd` 4、`vmovdqu` 8 |
| `correction` | `vpmulld` 4、`vpmuludq` 42、`vpaddq` 43、`vpsllq` 11、`vpsrlq` 12、`vpshufd` 17、`vpmovsxdq` 16 |

`replace`は`native`の自動ベクトル化と同一の内訳である。
`correction`は、ライブラリが64ビット積を符号なし32ビット積3回の合成で模倣するため、乗算に関係する命令が`vpmuldq` 8回から`vpmuludq` 42回へ増えた。
この命令列の速度は測っていない。

## 結論

FMを採用した現行のエンジンでも、移植可能な`x86-64`のビルドは`native`より約1.7%遅いだけである。
FMの4つの核は`native`で既にAVX2へ自動ベクトル化されており、fearless_simdで書いた`replace`は同じ命令列、`correction`は乗算の命令が増えた列になった。
FM採用前の差が0.3%だったので約1.7%の大半はFMの命令列の差と読めるが、FM単独の寄与は測っていないため、実行時に命令集合を選ぶライブラリで移植可能ビルドが回復できる探索速度は約1.7%を超えない目安にとどまる。

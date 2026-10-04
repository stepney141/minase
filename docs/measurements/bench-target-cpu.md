# target-cpuによるbenchとperftの速度差

## 目的

リポジトリの`.cargo/config.toml`が指定する`-C target-cpu=native`のビルドを、移植性のある`x86-64`と`x86-64-v3`のビルドと比べ、探索と合法手生成の速度の差を確認する。

## コマンドライン

masterの67a08daから、`RUSTFLAGS`で`.cargo/config.toml`の指定を上書きして3種類のビルドを別々のターゲットディレクトリに作った。
releaseの設定はワークスペースのルートの`lto = true`と`codegen-units = 1`である。

```console
RUSTFLAGS="-C target-cpu=<native|x86-64|x86-64-v3>" CARGO_TARGET_DIR=<構成ごとの場所> \
  cargo build --release --bin bench --bin perft
```

3つのビルドを1巡ずつ交互に5巡実行し、構成ごとに5個の値の中央値を比べた。

```console
taskset -c 3 bench-<構成> --depth 9 --repetitions 3
taskset -c 3 perft-<構成> --rules engine-default 5
```

benchの値は、計測外のウォームアップ1回の後に測った3回の中央値である。

## エンジン

3構成とも67a08daであり、違いは`target-cpu`だけである。
規則はbench既定の`engine-default`、評価重みは埋め込みの`nets/pst.bin`を使った。

## 環境

2026年10月4日、Intel Core Ultra 7 265KF（性能コア8、高効率コア12、論理コア20）の環境で測定した。
Rustは`rustc 1.98.0 (88d9e12ae 2026-08-18)`を使った。
性能コアのCPU 3へ固定し、探索ワーカーは1、置換表は256 MiBとした。
開始前に`match_runner`と`spsa_runner`のプロセスがないことを確認した。

## 結果

ノード数は3構成で一致した（benchは6,435,317、perftは71,548,181）。
[生出力](bench-target-cpu/results.txt)の中央値は次のとおりである。

| 構成 | bench NPS | x86-64に対する比 | perft nodes/s | x86-64に対する比 |
|---|---:|---:|---:|---:|
| x86-64 | 1,922,368 | 1.000 | 29,499,354 | 1.000 |
| x86-64-v3 | 1,928,074 | 1.003 | 30,069,668 | 1.019 |
| native | 1,928,611 | 1.003 | 29,836,965 | 1.011 |

同じ構成の巡ごとの幅は、benchで最大約4%、perftで約6%あり、構成間の差はこの幅に収まった。
逆アセンブルでは、nativeのAVX2レジスタを使う命令約3,250個はメモリの複写と初期化に限られ、POPCNTは13か所、BMI2のPEXTはどの構成にも現れなかった。
主要な処理は整数のスカラー演算であり、命令セットの拡張が効く箇所がほとんどない。

## 結論

`target-cpu=native`の速度上の利点は測定の幅に埋もれる程度であり、移植性のある`x86-64`でビルドしても実質的な損失はない。

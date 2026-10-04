# LTOとcodegen-unitsによるbenchとperftの速度差

## 目的

releaseの設定の`lto = true`と`codegen-units = 1`の有無を4通りに組み合わせ、ライブラリ`minase-core`の合法手生成とエンジン`minase`の探索の速度の差を確認する。
Cargoは依存先のcrateのプロファイルを読まないため、`minase-core`の利用者と、ワークスペースのルートの設定が公開物に含まれなかった`cargo install minase`の利用者は、Cargo既定のプロファイル（LTOなし、`codegen-units = 16`）でビルドすることになる。

## コマンドライン

67a08daと同じソースから、環境変数でプロファイルを上書きして4構成を別々のターゲットディレクトリに作った。

```console
CARGO_PROFILE_RELEASE_LTO=<true|false> CARGO_PROFILE_RELEASE_CODEGEN_UNITS=<1|16> \
  CARGO_TARGET_DIR=<構成ごとの場所> cargo build --release --bin bench --bin perft
```

4つのビルドを1巡ずつ交互に5巡実行し、構成ごとに5個の値の中央値を比べた。

```console
taskset -c 3 bench-<構成> --depth 9 --repetitions 3
taskset -c 3 perft-<構成> --rules engine-default 5
```

`perft`は`minase_core`の機能だけを呼ぶので、ライブラリの合法手生成と着手の実行の速さを表す。
`bench`は探索と評価を含むエンジン全体の速さを表す。

## エンジン

4構成とも67a08daのソースであり、違いはプロファイルだけである。
`target-cpu`は`.cargo/config.toml`の`native`のままとした（[bench-target-cpu](bench-target-cpu.md)により`x86-64`との差は測定の幅に収まる）。
規則はbench既定の`engine-default`、評価重みは埋め込みの`nets/pst.bin`を使った。

## 環境

2026年10月4日、Intel Core Ultra 7 265KF（性能コア8、高効率コア12、論理コア20）の環境で測定した。
Rustは`rustc 1.98.0 (88d9e12ae 2026-08-18)`を使った。
性能コアのCPU 3へ固定し、探索ワーカーは1、置換表は256 MiBとした。
開始前に`match_runner`と`spsa_runner`のプロセスがないことを確認した。

## 結果

ノード数は4構成で一致した（benchは6,435,317、perftは71,548,181）。
[生出力](bench-lto-codegen-units/results.txt)の中央値は次のとおりである。

| 構成 | `lto` | `codegen-units` | perft nodes/s | 既定に対する比 | bench NPS | 既定に対する比 |
|---|---|---:|---:|---:|---:|---:|
| 両方あり（ワークスペースの設定） | true | 1 | 30,134,258 | 1.629 | 1,937,271 | 1.157 |
| LTOだけ | true | 16 | 29,427,966 | 1.591 | 1,876,376 | 1.120 |
| codegen-units=1だけ | false | 1 | 17,898,759 | 0.968 | 1,711,410 | 1.022 |
| Cargoの既定 | false | 16 | 18,499,966 | 1.000 | 1,674,751 | 1.000 |

同じ構成の巡ごとの幅はbenchで1%以内、perftで約6%以内だった。
両方ありとLTOだけのperftの差を除き、構成間の5巡の値の範囲は重ならなかった。
perftでcodegen-units=1だけが既定より遅い傾向は、5巡すべてで同じだった。
benchの比を[sensitivity-time2x](sensitivity-time2x.md)に基づく目安216×log2(比) Eloへ換算すると、両方ありは既定より約45 Elo、LTOだけは約35 Elo、codegen-units=1だけは約7 Elo相当になる。

既定のプロファイルのperftでは、`generate_piece_moves`、`captured_squares`、`promotion_choice`、`sliding_control`、`push_with_promotion`など`minase-core`内部の関数が個別のシンボルとして残り、LTOありではそれらが`generate_moves`へ展開されていた（`perf record -e cpu_core/cycles/u`による）。
crateの境界を越える呼び出しは`generate_moves`と`try_make_move`の各約185万回だけなので、差の主因はcrate内部のインライン展開である。

## 結論

速度差のほとんどはLTOによるもので、既定のプロファイルではライブラリの合法手生成が約61%、エンジンの探索が約86%の速さに落ちるため、エンジンのマニフェストへプロファイルを複製し、ライブラリのREADMEで利用者へLTOを推奨した。

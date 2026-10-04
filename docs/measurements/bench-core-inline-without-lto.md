# LTOなしのビルドにおける`#[inline]`の効果

## 目的

[bench-lto-codegen-units](bench-lto-codegen-units.md)で確認したLTOなしの速度低下を、`minase-core`の関数への`#[inline]`と`#[inline(always)]`でどこまで取り戻せるかを測り、LTOありのビルドへの副作用とあわせて採否を判断する。

## コマンドライン

67a08daから実験用の7版を作り、各版をLTOなし（Cargo既定、`lto = false`、`codegen-units = 16`）とLTOあり（ワークスペースの設定、`lto = true`、`codegen-units = 1`）の2通りでビルドした。
各版の差分は次の表のとおりであり、67a08daに対するパッチを保存した。

| 版 | 内容 | 属性の数 | パッチ |
|---|---|---:|---|
| entry | 公開の入口関数`generate_moves`と`try_make_move`に`#[inline]` | 2 | [entry.patch](bench-core-inline-without-lto/entry.patch) |
| hot | LTOなしのperftで個別に残った内部関数8つ（`generate_piece_moves`、`captured_squares`、`promotion_choice`、`promotion_choice_for`、`sliding_control`、`push_with_promotion`、`special_move_is_legal`、`special_step_destinations`）に`#[inline]` | 8 | [hot.patch](bench-core-inline-without-lto/hot.patch) |
| always | hotのうち`push_with_promotion`と`captured_squares`を`#[inline(always)]`に変更 | 8 | [always.patch](bench-core-inline-without-lto/always.patch) |
| v4 | alwaysに加え、LTOなしのbenchで個別に残った8つ（`ordinary_attackers_to`、`special_attackers_to`、`collect_ordinary_capturers`、`emit_ordinary_captures`、`generate_special_captures`、`captured_lions`、`SquareIter::next`、`generate::generate_moves`）に`#[inline]` | 16 | [v4.patch](bench-core-inline-without-lto/v4.patch) |
| v5 | v4に加え、`piece_control_without_special`に`#[inline]` | 17 | [v5.patch](bench-core-inline-without-lto/v5.patch) |
| v6 | v4から`generate::generate_moves`の`#[inline]`を除去 | 15 | [v6.patch](bench-core-inline-without-lto/v6.patch) |
| v7 | v4の`generate_piece_moves`を`#[inline(always)]`に変更 | 16 | [v7.patch](bench-core-inline-without-lto/v7.patch) |

```console
CARGO_PROFILE_RELEASE_LTO=<true|false> CARGO_PROFILE_RELEASE_CODEGEN_UNITS=<1|16> \
  CARGO_TARGET_DIR=<プロファイルごとの場所> cargo build --release --bin bench --bin perft
```

4回の測定のそれぞれで、対象のビルドを1巡ずつ交互に5巡実行し、5個の値の中央値を比べた。

```console
taskset -c 3 bench-<版>-<プロファイル> --depth 9 --repetitions 3
taskset -c 3 perft-<版>-<プロファイル> --rules engine-default 5
```

関数が個別のシンボルとして残るかどうかは、`taskset -c 3 perf record -e cpu_core/cycles/u -F 4000`の自己時間の一覧で判定した。

## エンジン

基準は67a08da（以下base）であり、各版はbaseに上表の属性だけを加えたものである。
`target-cpu`は`.cargo/config.toml`の`native`のままとした。
規則はbench既定の`engine-default`、評価重みは埋め込みの`nets/pst.bin`を使った。

## 環境

2026年10月4日、Intel Core Ultra 7 265KF（性能コア8、高効率コア12、論理コア20）の環境で測定した。
Rustは`rustc 1.98.0 (88d9e12ae 2026-08-18)`を使った。
性能コアのCPU 3へ固定し、探索ワーカーは1、置換表は256 MiBとした。
各測定の開始前に`match_runner`と`spsa_runner`のプロセスがないことを確認した。

## 結果

ノード数はすべてのビルドで一致した（benchは6,435,317、perftは71,548,181）ので、どの版も探索木を変えない。

### 第1回と第2回（entry、hot、always）

[第1回の生出力](bench-core-inline-without-lto/run1.txt)と[第2回の生出力](bench-core-inline-without-lto/run2.txt)の中央値は次のとおりである。
第2回はLTOなしのbaseを含まないため、always-noneの比は第1回のbase-noneに対する値である。
両方の回で測ったLTOありのbaseの差は0.3%未満だった。

| 版 | プロファイル | perft nodes/s | 同じプロファイルのbaseに対する比 | bench NPS | 同じプロファイルのbaseに対する比 |
|---|---|---:|---:|---:|---:|
| base | LTOなし | 18,457,272 | 1.000 | 1,673,202 | 1.000 |
| entry | LTOなし | 16,744,244 | 0.907 | 1,679,638 | 1.004 |
| hot | LTOなし | 26,133,554 | 1.416 | 1,829,500 | 1.093 |
| always | LTOなし | 27,632,707 | 1.497 | 1,851,675 | 1.107 |
| base | LTOあり | 30,056,490 | 1.000 | 1,931,167 | 1.000 |
| entry | LTOあり | 25,902,691 | 0.862 | 1,935,130 | 1.002 |
| hot | LTOあり | 29,603,381 | 0.985 | 1,941,900 | 1.006 |
| always | LTOあり | 29,136,551 | 0.969 | 1,966,688 | 1.018 |

公開の入口関数への`#[inline]`（entry）は、どちらのプロファイルでもperftを遅くした。

### 第3回と第4回（v4からv7）

[第3回の生出力](bench-core-inline-without-lto/run3.txt)ではv5がv4と同じ値になり、`piece_control_without_special`への`#[inline]`は効果がなかった。
次の表は、すべての版を同じ回で比べた[第4回の生出力](bench-core-inline-without-lto/run4.txt)の中央値である。
回復率は「(版 − base-none) ÷ (base-both − base-none)」であり、LTOなしの損失のうち取り戻した割合を表す。

| 版 | LTOなし bench NPS | 回復率 | LTOなし perft nodes/s | 回復率 | LTOあり bench（baseに対する比） | LTOあり perft（baseに対する比） |
|---|---:|---:|---:|---:|---:|---:|
| base | 1,681,249 | 0% | 18,535,102 | 0% | 1,946,700（1.000） | 30,364,405（1.000） |
| always | 1,859,026 | 67% | 27,742,305 | 78% | — | — |
| v4 | 1,905,149 | 84% | 26,720,278 | 69% | 1,962,632（1.008） | 28,918,158（0.952） |
| v6 | 1,904,647 | 84% | 27,885,521 | 79% | 1,962,127（1.008） | 28,948,599（0.953） |
| v7 | 1,901,308 | 83% | 28,248,687 | 82% | 1,950,792（1.002） | 29,168,276（0.961） |

v4はエンジンの回復率を上げた一方で、perftを下げた。
v4のLTOなしのperftでは、`generate::generate_moves`が展開された代わりに`generate_piece_moves`が個別の関数に戻っており、インライン展開の予算が移っただけだった。
v7はこの関数を`#[inline(always)]`で固定し、perftを取り戻した。

LTOありのperftの低下は、baseの範囲30,230,674〜30,492,317とv7の範囲28,976,177〜29,308,171が重ならず、実在する。
ところがLTOありのプロファイルでは、baseとv7の関数の構成はどちらも`generate_moves`と`piece_control_without_special`の2つで同じだった。
したがって、この低下はインライン展開の判断ではなく関数内部の命令配置などの差によるものであり、属性で狙って取り除くことはできないと判断して調整を打ち切った。

## 結論

v7はLTOなしの損失の8割強を取り戻すが、LTOありの合法手生成を約4%遅くし、`minase-core`をLTOなしで使う利用者が確認できないため採用せず、利用者へのLTOの推奨（`crates/minase-core/README.md`）で代えた。
LTOなしで使う利用者が現れた場合は、v7のパッチを出発点として再検討する。

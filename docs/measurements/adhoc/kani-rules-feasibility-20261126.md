# Kaniによる規則実装の記号的な検査の実現可能性

## 目的

有界モデル検査器Kaniが、中将棋の規則を実装したcrate `minase-core`の関数と、規則書 [chu-shogi.md](../../rules/chu-shogi.md) の条文から書いた素朴な定義との一致を、全入力について実用的な時間とメモリで検査できるかを確かめる。
対象は、駒の成りの可否を決める関数と、走り駒の利きが途中の駒で遮られる処理（以下「走りの遮蔽」）の2つである。
有界モデル検査は、プログラムを論理式へ展開し、範囲内の全入力を、論理式を真にする割り当てがあるかを判定するプログラム（SATソルバ）で検査する方法である。
Kaniは、RustをCの有界モデル検査器CBMCの入力へ変換して検査する。
この測定は、[規則実装の形式検証の実現可能性と検証を強める候補](../../proposals/rules-formal-verification.md)の候補4の根拠であり、どの設計書も計画していない。

## コマンドライン

測定日は2026年11月26日である。
Kaniは、リポジトリの中でGitが追跡しない`target/fv-exp/`へ入れた。
この日に`cargo install`が取得した`kani-verifier`の版は0.68.0である。

```console
mkdir -p target/fv-exp && cd target/fv-exp
export KANI_HOME=$PWD/kani-home
cargo install --locked kani-verifier --root $PWD/cargo-root
./cargo-root/bin/cargo-kani setup
```

検査の対象は、`crates/minase-core`を`target/fv-exp/core`へ複製し、[kani-harness.patch](kani-rules-feasibility-20261126/kani-harness.patch)を当てたものである。
パッチは、検査の記述（以下「ハーネス」）を`#[cfg(kani)]`のモジュールとして`promotion.rs`と`attacks/tables.rs`の末尾へ足し、複製をワークスペースから切り離すために`Cargo.toml`を書き換える。
本番の関数の本体は変えない。
パッチと2つのスクリプトは、[kani-rules-feasibility-20261126/](kani-rules-feasibility-20261126/)から`target/fv-exp/`へ複写して使う。

```console
A=../../docs/measurements/adhoc/kani-rules-feasibility-20261126
cp $A/kani-harness.patch $A/run.sh $A/plain.sh .
cp -r ../../crates/minase-core core
patch -p3 -d core < kani-harness.patch
./run.sh
```

[run.sh](kani-rules-feasibility-20261126/run.sh)は、後述する4つのハーネスを1つずつ`cargo kani --harness <名前> --exact`で実行し、出力の各行に時刻を付け、CBMCの常駐メモリを30秒ごとに記録する。
`run.sh`は、各ハーネスの後に、同じ利用者の`cbmc`のプロセスをすべて終了させるので、ほかにCBMCを動かしている機械では使わない。

測定では、2つ目のハーネスの実行が機械のメモリを使い切りかけたので、そこでKaniを止めた。
3つ目のハーネスは`run.sh`が続けて起動したが、直後に止めた。
判定を得られなかった2つ目以降のハーネスは、CBMCを直接実行した。

2つ目のハーネスの直接実行には、Kaniが生成した検査対象のファイルを、Kaniが使うのと同じオプションで渡した。
`<OUT>`は、`core/target/kani/`以下にある、名前が`sliding_symbolic_origin.out`で終わるファイルである。

```console
kani-home/kani-0.68.0/bin/cbmc --no-malloc-may-fail --no-undefined-shift-check \
  --no-signed-overflow-check --nan-check --no-self-loops-to-assumptions \
  --no-pointer-primitive-check --object-bits 16 --sat-solver cadical --slice-formula <OUT>
```

3つ目と4つ目のハーネスの直接実行には、[plain.sh](kani-rules-feasibility-20261126/plain.sh)を使った。
先に、`cargo kani`を起動して`cbmc`のプロセスが現れた時点で、そのコマンドラインを`/proc/<PID>/cmdline`から`<ハーネス名>.cmdline`へ保存し、すぐにKaniを止めた。
`plain.sh`は、保存したコマンドラインから`--json-ui`だけを除いてCBMCを実行する。
引数は、順に出力のファイル名の接頭辞、ハーネス名、および実行時間の上限の秒数である。
`plain.sh`は、CBMCの仮想メモリを`ulimit -v 18000000`で17.2 GiBに制限する。
本書は、メモリの量を2の30乗バイトを単位とするGiBで書く。

```console
./plain.sh sliding-production sliding_production_tables 900
./plain.sh sliding-enumerated sliding_enumerated_origin 1500
```

保存したコマンドラインの例は[cbmc.cmdline.example.txt](kani-rules-feasibility-20261126/cbmc.cmdline.example.txt)にある。
Kaniが使うオプションは、2つ目のハーネスに渡したものに`--verbosity 9`と`--json-ui`を加えたものだった。
2つ目のハーネスの直接実行には`--verbosity 9`がないので、段階ごとの時間が出力にない。

### ハーネス

ハーネスは4つある。
入力は`kani::any()`で記号的な値（その型が取り得る任意の値を表す値）にし、升は筋と段がともに12未満であることを仮定する。

| ハーネス | 検査する性質 |
|---|---|
| `promotion::kani_proofs::standard_promotion_choice` | 標準規則（規則コードはL0とP0）の`MoveRules::promotion_choice_for`の結果が、規則書の第17条から第19条を段の数値で書いた式と一致する。駒の色、駒種29種、駒が成っているかどうか、始点と終点の升、捕獲の有無、および成り権の保留の有無を記号的な値にする。入力は9,621,504通りある。式は、成り先の有無を本番の`PieceKind::promoted`から、段の値を本番の`Square::rank`から借りる。 |
| `attacks::tables::kani_proofs::sliding_symbolic_origin` | `AttackTables::sliding_control`の結果が、1升ずつ進んで最初の駒で止まる歩行（第7条第4項、第5項）の結果と一致する。始点（144升）、方向（8方向）、および占有（3つの64ビット整数）を記号的な値にする。占有は、盤外に当たるビットを落とすので、有効な144升の全部の組合せを取る。利き線の表だけを本番の`build_ray_table`で構築し、ほかの表は空にした`AttackTables`を使う。歩行は、1升の移動とビットの設定に本番の`step_square`と`Bitboard::set`を使う。 |
| `attacks::tables::kani_proofs::sliding_production_tables` | 本番と同じ入口`attack_tables()`で全部の表を構築し、2つ目と同じ性質を検査する。 |
| `attacks::tables::kani_proofs::sliding_enumerated_origin` | 始点と方向を具体的な値の二重のループ（144×8）で列挙し、占有だけを記号的な値にして、2つ目と同じ性質を検査する。表は2つ目のハーネスと同じである。 |

## エンジン

検査の対象は、masterのコミット`767c36fe`の`crates/minase-core`である（このcrateの最終変更は`eaf49464`）。
Kaniは0.68.0、同梱のCBMCは6.11.0、SATソルバはCaDiCaL 3.0.0であり、Kaniが使うRustのコンパイラは`nightly-2026-08-21`である。

Kani 0.68.0には、CBMCが結果の書き出しの途中でメモリ不足になると、途中までの結果から検証の成功を報告し得る不具合がある（[issue #4905](https://github.com/model-checking/kani/issues/4905)）。
この測定で成功と判定した実行は、メモリの使用量が小さい1つ目のハーネスと、Kaniを通さないCBMCの直接実行だけなので、この不具合の影響を受けない。

## 環境

作業機（Intel Core Ultra 7 265KF、物理20コア、論理20コア、メモリ30.9 GiB）で測った。
カーネルは`6.18.63-3-lts`である。
CBMCは1スレッドで動く。
測定の間、棋力測定は走っていなかったが、同じ機械でオンライン対局サイトlishogiのBotの対局が2局（それぞれ4スレッドの探索）動いていた。
この負荷が所要時間に与えた影響は測っていない。

## 結果

| ハーネス | 実行の方法 | 判定 | 所要時間 | CBMCの常駐メモリの最大 |
|---|---|---|---|---|
| `standard_promotion_choice` | Kani | 成功。73件の検査がすべて通った。 | 検証0.14秒、起動から終了まで約1秒 | 記録なし（30秒の間隔より短い） |
| `sliding_symbolic_origin` | Kani | 判定なし。機械のメモリが尽きかけたので打ち切った。 | 13分16秒で打ち切り（SATの求解は8分13秒で完了） | 14.3 GiB |
| `sliding_symbolic_origin` | CBMCを直接実行 | 性質の表明が成功した。 | 出力の先頭から判定まで2分10秒、起動から終了まで2分18秒 | 9.8 GiB |
| `sliding_production_tables` | CBMCを直接実行 | 判定なし。記号実行の途中でメモリの制限に達した。 | 4分18秒 | 17.1 GiB（制限は仮想メモリ17.2 GiB） |
| `sliding_enumerated_origin` | CBMCを直接実行 | 判定なし。記号実行の途中でメモリの制限に達した。 | 20分4秒 | 17.1 GiB（制限は仮想メモリ17.2 GiB） |

常駐メモリの最大は、30秒ごとの標本の最大である。

`standard_promotion_choice`の論理式は、変数3,061個、節9,454個だった。

`sliding_symbolic_origin`の論理式は、変数4,773,562個、節20,010,179個だった。
Kaniを通した実行では、記号実行に49.6秒、論理式への変換に9.6秒かかり、SATソルバを5回呼んだ。
5回の求解は0.2秒、1.2秒、2.7秒、2.3秒、および51.1秒で、最後の回が充足不能を返して終わった。
求解と求解の間には、最大で約3分半の間隔があった。
最後の求解の後、CBMCのプロセスは約3分半の間残り、その間に何をしていたかは出力から分からない。
CBMCの終了の後、`ps`と`free`で手動で確かめたところ、Kaniの本体（`cargo-kani`）の常駐メモリが15.4 GiB、機械のメモリの使用量が29 GiBだったので、Kaniを停止した。
この2つの値と打ち切りの時刻は手動の観測であり、`*.rss`には記録がない。

同じ検査対象をCBMCで直接実行すると、504件の検査のうち52件が失敗と表示され、CBMCの全体の表示は検証の失敗だった。
52件はすべて、Kaniが到達できるかどうかを調べるために埋め込む検査（`reachability_check`）である。
この検査は、その地点へ到達できる場合に失敗と表示されるものであり、不具合を表さない。
`sliding_control`の結果と歩行の結果が一致するという性質の表明は、成功と表示された。
同じ表明に対する到達の検査は失敗と表示されたので、表明の地点へ到達する入力があり、表明が一度も評価されないまま成功したのではない。
直接実行には、ループの展開の回数の上限を与えていない。
比較に使う歩行は11歩以内、利き線の表の構築は盤端までで止まるので、CBMCはループをすべて展開する。

`sliding_production_tables`は、`AttackTables::build`が固定の利きの表を構築するループ（`attacks/tables.rs`の73行目）を展開している途中で、メモリの制限に達した。
`sliding_enumerated_origin`は、144×8回のループの中で歩行のループを展開している途中で、メモリの制限に達した。

各実行の出力から、段階ごとの時間、求解の結果、および判定の行を抜き出したものを`*.summary.txt`に、CBMCの常駐メモリの記録を`*.rss`に、スクリプトの標準出力を`stdout.txt`に保存し、[kani-rules-feasibility-20261126/](kani-rules-feasibility-20261126/)へ置いた。
`*.rss`の値の単位はKiBである。

### 2026年11月25日の予備の実行

前日に同じ内容のハーネスを実行したが、各段階の時刻を記録しておらず、走りの遮蔽はいずれも判定が出る前に打ち切った。
最初の3回の実行は、同時対局数19の棋力測定（`nnue-gen3-search-retune-ltc`）が進行している間に行ったので、所要時間は参考にならない。
成りの可否は、この日も検証0.14秒で成功した。
前日の実行からは、走りの遮蔽がKaniで検査できるかどうかを結論できなかった。

## 結論

標準規則の成りの可否は、Kaniで全入力を1秒以内に検査できた。
走りの遮蔽は、利き線の表だけを構築する形にしてCBMCを直接実行すると、全部の始点、8方向、および有効な升の全部の占有について、本番の座標とビットの表現を共有する歩行との一致を、2分10秒と9.8 GiBで検査できた。
Kani 0.68.0を通した実行は、30.9 GiBの機械のメモリを使い切りかけて判定を得られなかった。
本番と同じ入口で全部の表を構築する形は、仮想メモリ17.2 GiBの制限の中で記号実行が終わらなかった。

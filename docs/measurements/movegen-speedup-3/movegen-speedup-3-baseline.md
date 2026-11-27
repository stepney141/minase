# 第3期の高速化の準備：基準の出力と、対比較の組数

## 目的

[第3期の設計書](../../plans/movegen-speedup-3.md)の準備のフェーズとして、基準のコミットの探索の結果を記録し、同じバイナリどうしのサイクル数の対比較から、測定単位の判定と最終測定に使う組数を定める。

## コマンドライン

基準はmasterのコミット`6cd40868`である。基準の作業ツリーで`cargo build --release --locked -p minase --bin minase`を実行し、バイナリを`data/ms3/bin/base-6cd40868`へ写した。
教師生成は作業ディレクトリのコミットを教師のファイルに書くので、すべての実行で、基準のコミットのdetachedなworktree（`data/ms3/cwd`）を作業ディレクトリにした。
ネットは標準の第3世代のA型のネット（SHA-256は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）をNASから`data/ms3/net.bin`へ写して使った。

探索の結果の記録は、[付属のスクリプト](movegen-speedup-3-baseline/verify_identical.py)で行った。
記録は[付属の結果](movegen-speedup-3-baseline/baseline-outputs.json)にあり、同じバイナリで`check`を実行して一致することも確かめた。

```console
python3 verify_identical.py record --binary data/ms3/bin/base-6cd40868 --net data/ms3/net.bin \
  --cwd data/ms3/cwd --out baseline-outputs.json
python3 verify_identical.py check --binary <候補> --net data/ms3/net.bin \
  --cwd data/ms3/cwd --baseline baseline-outputs.json
```

スクリプトは設計書の「検証」の節の4種類の出力を比べる。
1つ目は、`bench`の深さ16（規則セット`JCSA`）の15局面の行の深さ、ノード数、最善手、および評価値である。
2つ目は、教師生成（規則セット`lishogi`、各局面2万ノード、`--max-ply`は既定の4,000）の、シード`9010200000`の2局とシード`424242`の4局のファイルのSHA-256、対局数、および探索ノード数の合計である。
3つ目は、7つの規則セットのそれぞれで`bench`の15局面を`go nodes 60000`で探索したときの、最後の`info depth`の行（`time`、`nps`、および`hashfull`の欄を除く）と`bestmove`の行である。局面ごとに`usinewgame`を送り、`bestmove`を受け取ってから次の局面を送る。
4つ目は、同じ7つの規則セットのそれぞれで`bench`の15局面の`perft 2 divide`の、最初の手ごとの行、深さごとの経路数の行、および総数の行である。
どの検査も探索ワーカーは1であり、所定の行数がそろわない場合は失敗とする。

サイクル数の対比較は、[付属のスクリプト](movegen-speedup-3-baseline/paired.py)を、Botを止める[付属のスクリプト](movegen-speedup-3-baseline/measure.sh)から呼んで行った。
`measure.sh`は、対局の測定、SPSA、教師生成、およびcargoのビルドが動いていないことを確かめ、lishogiのBotのエンジンが対局していない時点を待ってBotのコンテナを止め、終了時に起動し直す。
各組は親と候補を1回ずつ実行し、奇数番目の組は親から、偶数番目の組は候補から実行する。
各実行は性能コアの論理CPU 3に固定し、`perf stat -x, -e cpu_core/cycles/u,cpu_core/instructions/u,cpu_core/branch-misses/u`で数える。
`bench`は`setoption name RuleSet value JCSA`、`setoption name EvalFile value <ネット>`、`isready`、`bench depth 16 threads 1 repetitions 1`、`quit`の5行を標準入力から与える。
教師生成は`data selfplay generate --eval-file <ネット> --output-dir <空の出力先> --chunk-games 2 --games 2 --seed 9010200000 --nodes 20000 --random-moves 0 --concurrency 1 --hash-mb 16`である。
組の中の2つの実行で、`bench`は局面別の行（経過時間を除く）が、教師生成はファイルのSHA-256と探索ノード数の合計が一致することも確かめる。

```console
measure.sh <親> <候補> <benchの組数> <教師生成の組数> <結果.jsonl>
python3 paired.py pairs --results <結果.jsonl>
python3 paired.py judge --results <結果.jsonl>
```

同じバイナリどうしの対比較は、親と候補の両方に`data/ms3/bin/base-6cd40868`を渡し、`bench`と教師生成をそれぞれ20組行った。

組数の規則は設計書のとおりである。
同じバイナリどうしの対比較の組ごとの対数比の標準偏差をsとし、組数をn、自由度n−1のt分布の臨界値をtとして、`t × s ÷ √n`が、`bench`の両側95%の区間で1.001の自然対数以下、教師生成の片側95%の区間で1.003の自然対数以下になる最小の偶数のnを求める。上限は`bench`が40組、教師生成が60組である。

## エンジン

基準はmasterのコミット`6cd40868`である。規則セットは`bench`が`JCSA`、教師生成が`lishogi`である。
ビルドはリポジトリのrelease設定（`.cargo/config.toml`の`-C target-cpu=native -C llvm-args=-inline-threshold=1000`、LTO、`codegen-units = 1`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（性能コア8、高効率コア12、スレッドは計20）、メモリ32 GB、Linux `6.18.65-1-lts`、Rust `1.100.0`、perf `7.3.6`である。
探索ワーカーは1、`USI_Hash`は`bench`の既定値、教師生成の置換表は16 MBであり、同時に実行するのは1つだけである。

2026年11月27日17時40分から17時56分に、Botのコンテナを止めて同じバイナリどうしの対比較を測った。同じ機械で、対局の測定、SPSA、教師生成、およびビルドは動いていなかった。

## 結果

基準の出力は記録どおりに再現した。`bench`の深さ16は15局面で3,371,474ノード、教師生成はシード`9010200000`の2局が16,400,000ノード（SHA-256は`120dd8dc…`）、シード`424242`の4局が34,520,000ノード（同`aed39be4…`）である。

同じバイナリどうしの対比較は、組ごとの結果が[付属の結果](movegen-speedup-3-baseline/self-pairs.jsonl)にある。
全組で、組の中の2つの実行の探索の結果は一致した。

| 負荷 | 組数 | 対数比の平均 | 対数比の標準偏差s | サイクル数の範囲 | 命令数 |
|---|---:|---:|---:|---:|---:|
| bench深さ16 | 20 | −0.00003 | 0.00311 | 218.7億〜220.6億 | 約700.5億 |
| 教師生成2局 | 20 | +0.00062 | 0.00194 | 1,010.1億〜1,017.1億 | 約3,338.8億 |

同じ機械で2026年11月27日の昼に、Botの探索と重ねて行った試作Eの対比較の標準偏差（bench深さ16で0.0103、教師生成2局で0.1154）に比べ、Botを止めた状態の揺れは、bench深さ16で約3分の1、教師生成2局で約60分の1だった。

組数の規則を当てると、bench深さ16は、40組で区間の端から平均までの距離が0.00099となり、目標の0.00100（1.001の自然対数）に上限の40組でちょうど届く。教師生成2局は、4組で片側の距離が0.00228となり、目標の0.00300（1.003の自然対数）に届く最小の偶数である。

## 結論

測定単位の判定と最終測定は、bench深さ16を40組、教師生成2局を4組で行う。

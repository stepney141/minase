# ネットを外部のファイルから読む前後のbenchの一致

## 目的

ネットを埋め込んだmasterのコミットと、同じネットを`EvalFile`で渡した本書のブランチの先頭のコミットで、深さ5の`bench`の各局面の深さ、ノード数、最善手、および評価値が完全に一致することを確かめる。

## コマンドライン

`<NAS>`は`/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin`であり、masterが埋め込んでいた`crates/minase/nets/nnue.bin`を同じバイト列のまま写したものである。

```console
taskset -c 12 python3 scripts/bench_compare.py --reference 57eb3d25 --baseline 57eb3d25 --parent 57eb3d25 \
  --depth 5 --rounds 1 --eval-file <NAS>
```

`bench_compare.py`は、両方の版に`setoption name RuleSet value JCSA`、`setoption name EvalFile value <NAS>`、`isready`、`bench depth 5 threads 1 repetitions 3`を送る。
masterの版は`EvalFile`を宣言しないのでこの`setoption`を無視し、埋め込んだネットで探索する。
局面ごとの行は、同じ命令列をそれぞれの版へ送って保存し、経過時間の欄を除いて[生データ](nnue-external-network-file-bench-equivalence/)に置いた。

## エンジン

比べる元はmasterのコミット`57eb3d25`（ネットを埋め込んだ版）、比べる先は[外部ファイル化の設計書](../../plans/archived/nnue-external-network-file.md)のブランチの先頭のコミット`48956be1`（フェーズ3の後）であり、`bench_compare.py`が作業ツリーをビルドしたものである。
`bench`の規則は`bench`専用の`JCSA`である。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア、20スレッド）、メモリ30 GiBであり、`bench`は論理CPU 12（効率コア）に固定し、1スレッドで実行した。
測定の間、lishogi Botの2局（各4スレッド）が同じ機械で動いていたので、この記録の速度の値は比較に使わない。
速さの比較は、性能コアに固定して別に測った（[速さの記録](nnue-external-network-file-bench-speed.md)）。

## 結果

`bench_compare.py`は`MATCH: 15 positions agree with reference 57eb3d2 in depth, nodes, best move, and score`を出して終了コード0で終わった。
15局面の総ノード数は、どちらの版も33,615だった。
保存した局面ごとの15行（`master-57eb3d25.txt`と`branch-48956be1.txt`）は、経過時間の欄を除いて全行一致した。

## 結論

ネットを外部のファイルから読む変更は、同じネットを与えたときの探索の結果を変えない。

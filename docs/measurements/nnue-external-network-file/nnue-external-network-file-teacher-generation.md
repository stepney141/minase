# 教師生成のネットのチェックサムと再開の照合

## 目的

`minase data selfplay generate`に`--eval-file`でネットを渡して2局を生成し、生成条件`config.json`とMNSDのヘッダの`network_checksum`が渡したネットのファイル全体のSHA-256と一致すること、および別のネットでの再開が止まることを確かめる。

## コマンドライン

生成器は作業ツリーの未コミットの変更を検査するので、コミット`e91c4f5e`を展開した変更のない一時のworktreeを作業ディレクトリにし、同じコミットからビルドした`minase`で生成した。
`<NAS>`は`/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin`である。

```console
minase data selfplay generate --eval-file <NAS> --output-dir <出力先> --games 2 --chunk-games 1 \
  --seed 20261117000 --nodes 2000 --random-moves 0 --max-ply 400 --hash-mb 16 --concurrency 2
minase data selfplay inspect <出力先>/teacher.mnsd
```

続けて、確定済みの塊がある出力先での再開の照合を確かめるため、同じ出力先と同じ引数で、`--eval-file`だけを固定ネット`crates/minase/nets/nnue-epoch0.bin`に替えて起動し、最後に`<NAS>`に戻して起動した。

## エンジン

生成器はコミット`e91c4f5e`（[外部ファイル化の設計書](../../plans/archived/nnue-external-network-file.md)のフェーズ2の後）であり、規則は生成器の既定の`lishogi`である。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア、20スレッド）、メモリ30 GiBであり、同時対局数は2、置換表は16 MiBである。

## 結果

2局のうち1局が手数の上限400手で破棄され、1局を保存して0.74秒で終わった。
`config.json`の`network_checksum`と、`inspect`が表示したMNSDのヘッダの`network_checksum`は、どちらも`987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd`であり、`<NAS>`のファイル全体のSHA-256と一致した。
`config.json`の`generation_commit`は`e91c4f5eaac453094d928fa54b1a3f128c1b9549`、`dirty`は`false`だった。

別のネット（SHA-256 `cc61415498636b74…`）での再開は、`error: generation conditions differ: network_checksum`を出して終了コード1で終わった。
元のネットでの再開は、確定済みの2つの塊を再利用して（`chunks_reused: 2`）終了コード0で終わった。

## 結論

教師生成は`--eval-file`のファイル全体のSHA-256をネットの識別子として記録し、既存の生成条件の照合で別のネットによる再開を拒否する。

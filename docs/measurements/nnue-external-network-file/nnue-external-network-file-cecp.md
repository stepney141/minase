# CECPでのネットの受け渡し

## 目的

CECPで動かすminaseが、`option EvalFile=`を受けずに`new`を受けるとエラーで止まること、および`scripts/hachu_replay.py`がminaseにネットを渡し、外部の中将棋エンジンHaChuの自己対局の1局を1手ずつ送って全手を受理させられることを確かめる。

## コマンドライン

`<NAS>`は`/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin`である。

```console
printf 'xboard\nprotover 2\noption RuleSet=JCSA\nnew\nping 1\n' | target/release/minase --protocol cecp
python3 scripts/hachu_selfplay.py --hachu ../hachu-debian/hachu --out <棋譜> --games 1 --time-cs 3000 --max-plies 300
python3 scripts/hachu_replay.py --games <棋譜> --eval-file <NAS> --minase target/release/minase \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --out <結果> --log <送受信の記録> --include-abnormal
```

## エンジン

minaseは[外部ファイル化の設計書](../../plans/archived/nnue-external-network-file.md)のブランチの作業ツリーでビルドしたものであり、フェーズ3はPythonのスクリプトだけを変えたので、Rustのコードはフェーズ2の後のコミット`e91c4f5e`と同じである。`hachu_replay.py`はフェーズ3の後のコミット`48956be1`の版である。
HaChuは手元の複製`../hachu-debian`（コミット`822d5121`）でビルドした`hachu`であり、規則はHaChuの既定の設定に対応する`L1,L3,P0,P5,P6,R2,E1,E2`である。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア、20スレッド）、メモリ30 GiBである。

## 結果

`option EvalFile=`を送らずに`new`を送ると、minaseは`tellusererror required option EvalFile is not set`を出し、標準エラーに`error: required option EvalFile is not set`を出して終了コード1で終わった。
`option EvalFile=<NAS>`を足すと、同じ命令列に`pong 1`を返して終了コード0で終わった。

HaChuの自己対局は、両側とも深さ4で300手に達して打ち切られた。
`hachu_replay.py`は、`option RuleSet=`の後、`new`の前に`option EvalFile=<NAS>`を送り、300手をすべて受理させた（`all_legal_no_result`、受理300/300手）。
打ち切った対局なので、終局の裁定は比べていない。

## 結論

CECPで動かすminaseは、ネットの指定がなければ対局を始めずに止まり、`hachu_replay.py`はネットを渡して再生できる。

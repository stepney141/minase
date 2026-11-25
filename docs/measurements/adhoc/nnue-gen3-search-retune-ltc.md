# 第3世代のネットに合わせた探索係数の再調整の候補のLTC

## 目的

[STC](nnue-gen3-search-retune-stc.md)で`H1`となった、[SPSAの最終値](nnue-gen3-search-retune.md)を反映した候補の採否を、[SPRTによる棋力測定の手引き](../../guides/sprt.md)の段階ゲートのLTCで判定する。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/nnue-gen3-search-retune-ltc --seed 7600000000 \
  --candidate commit:2b4bbbcb --baseline commit:b74f11c4 \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --rules lishogi --each time=60000+200 gsprt
```

GSPRTの仮説はelo0 = 0とelo1 = 10、誤り率はα = β = 0.05である。
シード7600000000は、STCの基本シード7500000000から、STCのペア数より十分に離した未使用の値である。

## エンジン

候補、起点、ネット、規則セット、およびrunnerは[STC](nnue-gen3-search-retune-stc.md)と同じである。
候補はコミット`2b4bbbcbfe3fc8522df70c4dd8cc2829ed825923`、起点はmasterのコミット`b74f11c45e7ba6cb43062014c4c34508398bf6a7`、ネットは標準の第3世代のA型のネット（SHA-256は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）、規則セットは`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）である。

## 環境

手元の測定機（Intel Core Ultra 7 265KF、物理20コア、実メモリ約33 GB）で、既定の同時対局数19で実行した。
lishogiのBotは止めずに動かした。
起動は1回であり、経過時間は4,844秒である。

## 結果

213ペアで`decision: H1`となった。対数尤度比LLRは2.957である。
有効ペア211、手数上限による破棄ペア2であり、ペンタノミアルは`[22, 9, 91, 28, 61]`である。
有効ペアの得点率は0.615である。
不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手はすべて0件である。

## 結論

`H1`であり、エンジン異常、時間切れ、および拒否着手が0件なので、候補を採用してmasterへマージする。
採用した係数の現行の値は、`crates/minase/src/search/params.rs`の表の既定値である。

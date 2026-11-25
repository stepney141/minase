# 第3世代のネットに合わせた探索係数の再調整の候補のSTC

## 目的

[SPSAの最終値](nnue-gen3-search-retune.md)を反映した候補を、[SPRTによる棋力測定の手引き](../../guides/sprt.md)の段階ゲートのSTCで選別する。

## コマンドライン

```console
target/release/minase match run \
  --run-dir data/matches/nnue-gen3-search-retune-stc --seed 7500000000 \
  --candidate commit:2b4bbbcb --baseline commit:b74f11c4 \
  --candidate-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin \
  --rules lishogi --each time=10000+100 gsprt --max-pairs 3000
```

GSPRTの仮説はelo0 = 0とelo1 = 10、誤り率はα = β = 0.05である。

## エンジン

候補は、SPSAの最終値を係数の表へ反映したコミット`2b4bbbcbfe3fc8522df70c4dd8cc2829ed825923`である。
起点は、探索スレッドのスタック容量の修正を含むmasterのコミット`b74f11c45e7ba6cb43062014c4c34508398bf6a7`であり、候補の親である。
ネットは両側とも標準の第3世代のA型のネット（SHA-256は`415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9`）である。
規則セットは`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）である。
SPSAを`lishogi`で行ったので、採否の測定も同じ規則セットで行うと2026年11月24日に利用者が決めた。手引きの標準コマンドの規則セットは`JCSA`である。
runnerは、masterのコミット`b74f11c4`をビルドした通常のminase（版3.0.1、SHA-256は`6d5afbd93ef168edf34a9b7e0fcc80d5678da4289a4955c893c145442690d4aa`）である。

## 環境

手元の測定機（Intel Core Ultra 7 265KF、物理20コア、実メモリ約33 GB）で、既定の同時対局数19で実行した。
lishogiのBotは止めずに動かした。
起動は1回であり、経過時間は1,105秒である。

## 結果

157ペアで`decision: H1`となった。対数尤度比LLRは2.957である。
有効ペア156、手数上限による破棄ペア1であり、ペンタノミアルは`[7, 12, 76, 20, 41]`である。
有効ペアの得点率は0.622である。
不正着手、クラッシュ、応答タイムアウト、時間切れ（`time_forfeits`）、および拒否着手はすべて0件である。

## 結論

振分け規則により、候補をLTCへ進める。
採否はLTCの判定で決める。

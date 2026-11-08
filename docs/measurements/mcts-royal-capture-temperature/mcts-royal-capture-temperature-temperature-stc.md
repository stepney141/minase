# 方策の温度1.4のSTC

## 目的

[設計書](../../plans/mcts-royal-capture-temperature.md)のフェーズ2の候補（`PolicyTemperature`の既定値を1.4にしたMCTS）を、既定値1.0のMCTSとSTCのGSPRTで比べ、採否を判定する。
2026年11月8日の利用者の決定により、この測定が`H1`なら、LTCを省いて採用とする。

## コマンドライン

本記録と同じ名前のディレクトリの`run-gate.sh`を、作業機で次のように実行した。

```console
W=data/worktrees/mcts-royal-capture-temperature
bash $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-temperature-stc/run-gate.sh \
    $W/target/release/minase 6258790a2df64e1cce4a9a88bda161e481eceb29 db685ca92ab8f81f361660d74e1cbed2fb66a680 \
    mcts-royal-capture-temperature-temperature-stc 20261108600000 time=10000+100 gsprt --max-pairs 3000
```

`run-gate.sh`が実行する`minase match run`は次のとおりである。

```console
minase match run --run-dir data/matches/mcts-royal-capture-temperature-temperature-stc --seed 20261108600000 \
    --candidate commit:6258790a2df64e1cce4a9a88bda161e481eceb29 --baseline commit:db685ca92ab8f81f361660d74e1cbed2fb66a680 \
    --candidate-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --baseline-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
    --each time=10000+100 --concurrency 2 gsprt --max-pairs 3000
```

## エンジン

候補はブランチ`mcts-policy-temperature-140`のコミット6258790aであり、基準のコミットからの変更は`PolicyTemperature`の既定値を100から140にしたことだけである。
基準はブランチ`mcts-royal-capture-temperature`のコミットdb685ca9であり、葉での王駒の捕獲の判定と、既定値100の`PolicyTemperature`を含む。
両側ともMCTS（`--search mcts`）で、NASの`nets/cycle-0032.onnx`を与えた。
推論はTensorRTのFP16、`NnBatchSize`は16である。
規則は`engine-default`（`L0,P0,R1,E0`）である。
対局のハーネスは、worktreeでビルドしたコミットeb7e3081のバイナリである。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月8日21時41分から11月9日0時14分に行った。
両エンジンの`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は2である。
同じ機械でlishogiのBotが動いていた。

本測定の前に、同じ条件で2ペアのスモークテストを行い、時間切れとエンジンの異常がないことを確かめた（[smoke.out](mcts-royal-capture-temperature-temperature-stc/smoke.out)）。

## 結果

111ペアで`decision: H1`となった（[集計](mcts-royal-capture-temperature-temperature-stc/results.out)）。
ペンタノミアル度数は、先後を入れ替えた2局の候補の得点が0、0.5、1、1.5、および2であるペアの数である。

| 項目 | 値 |
|---|---|
| 有効ペア | 110 |
| 破棄ペア | 1 |
| ペンタノミアル度数 | [1, 8, 36, 27, 38] |
| LLR | +2.962 |
| 判定 | `H1` |
| エンジンの異常、時間切れ、拒否着手 | 0件 |
| 経過時間 | 9,151秒 |

破棄した1ペアは、ペア70の2局目が手数の上限4,096手に達して打ち切られたものである。
有効ペアの候補の得点率は71.1%（220局で156.5点）であり、ロジスティックのEloに換算すると約+157である。
この換算は記録者の概算であり、GSPRTの判定には使っていない。
有効な対局と打ち切りを含む222局の終局は、詰みが150局、反復が50局、王駒の捕獲が21局、手数の上限が1局であった。
開始局面の乱択の手順を含む1局の手数は、打ち切りを除く221局で中央値552手、最長2,229手であった。

## 結論

候補はSTCで`H1`となったので、2026年11月8日の利用者の決定により、LTCを省いて採用とする。

# 方策の温度1.4のMCTSとαβ探索の固定ノードの対局

## 目的

[設計書](../../plans/mcts-royal-capture-temperature.md)のフェーズ2で、方策の温度（`PolicyTemperature`）の候補の値を1つに絞るため、温度1.4のMCTS 800回とαβ探索 500ノードの対局のEloを、温度1.0、1.4、および1.74の3条件で比べる。
固定ノードの対局は段階ゲートの前の探索的な選別であり、採否は段階ゲートで決める。

## コマンドライン

[固定ノードの対局のスクリプト](mcts-royal-capture-temperature-royal-ladder/run-ladder.sh)を、作業機で次のように実行した。
3条件は同じシードで、温度1.0、1.4、1.74の順に続けて実行した。

```console
W=data/worktrees/mcts-royal-capture-temperature
bash $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-ladder/run-ladder.sh \
    $W/target/release/minase 0b7c38000ceb154c5e33760763cbb79ebc129e4f \
    mcts-royal-capture-temperature-t140-ladder 20261103300000 100 PolicyTemperature=140
```

`run-ladder.sh`が実行する`minase match run`は次のとおりである。

```console
minase match run --run-dir data/matches/mcts-royal-capture-temperature-t140-ladder --seed 20261103300000 \
    --candidate commit:0b7c38000ceb154c5e33760763cbb79ebc129e4f --baseline commit:45d5db5498e1739b2307b3719891a1eec24f7fbd \
    --candidate-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --candidate-option NnBatchSize=16 --candidate-option PolicyTemperature=140 \
    --each nodes=800 --baseline-limit nodes=500 --concurrency 4 elo --pairs 100
```

## エンジン

候補は、ブランチ`mcts-royal-capture-temperature`で、葉での王駒の捕獲の判定の後に`PolicyTemperature`を既定値100で足したコミット0b7c3800のMCTSであり、NASの`nets/cycle-0032.onnx`を与えた。
基準は、[固定ノードの対局](../adhoc/alphazero-rl-c0032-ab-ladder.md)と同じコミット45d5db54のαβ探索と、そのコミットに埋め込まれたNNUEである。
規則は`engine-default`（`L0,P0,R1,E0`）である。
対局のハーネスは、worktreeでビルドしたコミットeb7e3081のバイナリである。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月8日20時9分から20時35分に行った。
両エンジンの`Threads`は1、同時対局数は4、MCTSの推論はTensorRTのFP16で`NnBatchSize`は16である。
対局はノード数の制限なので、同じ機械で動いていたlishogiのBotは、時間には影響するが結果の条件には影響しない。

## 結果

破棄ペア、エンジンの異常、および時間切れは0件で、経過時間は1,528秒であった（[集計](mcts-royal-capture-temperature-t140-ladder/results.out)）。
ペンタノミアル度数は、先後を入れ替えた2局の候補の得点が0、0.5、1、1.5、および2であるペアの数である。

| 温度 | ペンタノミアル度数 | Eloの点推定 | 95%信頼区間 |
|---|---|---|---|
| 1.0 | [24, 1, 49, 1, 25] | +3.5 | [−44.7, +51.8] |
| 1.4 | [18, 0, 48, 0, 34] | +56.1 | [+7.7, +106.7] |
| 1.74 | [20, 0, 45, 1, 34] | +50.7 | [+1.2, +102.4] |

表の3行は、本記録と、同じ選別の残る2条件の記録（`mcts-royal-capture-temperature-t100-ladder`、`-t140-ladder`、および`-t174-ladder`）の値である。
温度1.0の行は、コミットeb7e3081で指した[葉での王駒の捕獲の判定の固定ノードの対局](mcts-royal-capture-temperature-royal-ladder.md)とペンタノミアル度数まで一致した。

## 結論

温度1.4は3条件のうちEloの点推定が最も高く、段階ゲートの候補とした。
100ペアの選別なので、1.74との差は順位づけられない。

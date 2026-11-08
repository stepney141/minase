# 葉での王駒の捕獲の判定を入れたMCTSとαβ探索の固定ノードの対局

## 目的

[設計書](../../plans/mcts-royal-capture-temperature.md)のフェーズ1の候補の効果の大きさの目安として、候補のMCTS 800回とαβ探索 500ノードの対局のEloを、分岐元と同じMCTSで指した同じ条件の[固定ノードの対局](../adhoc/alphazero-rl-c0032-ab-ladder.md)（Eloの点推定−70）と比べる。
固定ノードの対局は診断であり、採否は段階ゲートで決める。

## コマンドライン

本記録と同じ名前のディレクトリの`run-ladder.sh`を、作業機で次のように実行した。

```console
W=data/worktrees/mcts-royal-capture-temperature
bash $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-ladder/run-ladder.sh \
    $W/target/release/minase eb7e3081919af5d64facadd7b935c600bba042a8 \
    mcts-royal-capture-temperature-royal-ladder 20261103300000 100
```

`run-ladder.sh`が実行する`minase match run`は次のとおりである。

```console
minase match run --run-dir data/matches/mcts-royal-capture-temperature-royal-ladder --seed 20261103300000 \
    --candidate commit:eb7e3081919af5d64facadd7b935c600bba042a8 --baseline commit:45d5db5498e1739b2307b3719891a1eec24f7fbd \
    --candidate-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --candidate-option NnBatchSize=16 --each nodes=800 --baseline-limit nodes=500 --concurrency 4 elo --pairs 100
```

シードは比べる対局と同じであり、100ペアの開始局面は同じである。

## エンジン

候補はブランチ`mcts-royal-capture-temperature`のコミットeb7e3081のMCTSで、NASの`nets/cycle-0032.onnx`を与えた。
基準はコミット45d5db54のαβ探索と、そのコミットに埋め込まれたNNUEであり、比べる対局と同じである。
比べる対局の候補もコミット45d5db54のMCTSであり、そのMCTSのコードは分岐元のmaster（4885ccb7）と同じである。
規則は`engine-default`（`L0,P0,R1,E0`）である。
対局のハーネスは、worktreeでビルドしたコミットeb7e3081のバイナリである。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月8日12時59分から13時28分に行った。
両エンジンの`Threads`は1、同時対局数は4、MCTSの推論はTensorRTのFP16で`NnBatchSize`は16である。
対局はノード数の制限なので、同じ機械で動いていたlishogiのBotは、時間には影響するが結果の条件には影響しない。

## 結果

破棄ペア、エンジンの異常、および時間切れは0件で、経過時間は1,631秒であった（[集計](mcts-royal-capture-temperature-royal-ladder/results.out)）。
ペンタノミアル度数は、先後を入れ替えた2局の候補の得点が0、0.5、1、1.5、および2であるペアの数である。

| MCTS | ペンタノミアル度数 | Eloの点推定 | 95%信頼区間 |
|---|---|---|---|
| 分岐元（比べる対局） | [39, 1, 39, 3, 18] | −70 | [−125, −19] |
| 候補 | [24, 1, 49, 1, 25] | +3.5 | [−44.7, +51.8] |

終局の内訳は次のとおりである（[集計](mcts-royal-capture-temperature-royal-ladder/summarize.out)）。

| MCTS | MCTSの勝ち（詰み） | MCTSの勝ち（王駒の捕獲） | MCTSの負け（詰み） | MCTSの負け（王駒の捕獲） | MCTSの負け（反復） | 手数の中央値 |
|---|---|---|---|---|---|---|
| 分岐元 | 77 | 1 | 26 | 92 | 4 | 362.5 |
| 候補 | 99 | 1 | 68 | 30 | 2 | 382.5 |

候補が王駒の捕獲で負けた30局では、MCTSの最後の手番の評価値がすべて負けの確定であった。
つまり、取られる手を選んだ時点で、探索は負けを読み切っていた。
最後の手番の駒の損得の中央値は−73であり、歩を1とする概算である。

## 結論

候補のMCTSは同じ条件でEloの点推定が−70から+3.5へ上がり、王駒の捕獲による負けは92局から30局へ減って、残りはすべて負けを読み切った後の手であった。

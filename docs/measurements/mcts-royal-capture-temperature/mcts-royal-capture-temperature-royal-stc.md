# 葉での王駒の捕獲の判定のSTC

## 目的

[設計書](../../plans/archived/mcts-royal-capture-temperature.md)のフェーズ1の候補（新しい節点の手番側が相手の最後の王駒を取れるなら1手の勝ちに確定させるMCTS）を、分岐元のMCTSとSTCのGSPRTで比べ、LTCへ進めるかを判定する。

## コマンドライン

本記録と同じ名前のディレクトリの`run-gate.sh`を、作業機で次のように実行した。

```console
W=data/worktrees/mcts-royal-capture-temperature
bash $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-stc/run-gate.sh \
    $W/target/release/minase eb7e3081919af5d64facadd7b935c600bba042a8 4885ccb76ee79fde500f7a5055b771621c4734cf \
    mcts-royal-capture-temperature-royal-stc 20261108200000 time=10000+100 gsprt --max-pairs 3000
```

`run-gate.sh`が実行する`minase match run`は次のとおりである。

```console
minase match run --run-dir data/matches/mcts-royal-capture-temperature-royal-stc --seed 20261108200000 \
    --candidate commit:eb7e3081919af5d64facadd7b935c600bba042a8 --baseline commit:4885ccb76ee79fde500f7a5055b771621c4734cf \
    --candidate-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --baseline-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
    --each time=10000+100 --concurrency 2 gsprt --max-pairs 3000
```

## エンジン

候補はブランチ`mcts-royal-capture-temperature`のコミットeb7e3081、基準は分岐元のmasterのコミット4885ccb7である。
両側ともMCTS（`--search mcts`）で、NASの`nets/cycle-0032.onnx`を与えた。
推論はTensorRTのFP16、`NnBatchSize`は16である。
規則は`engine-default`（`L0,P0,R1,E0`）である。
対局のハーネスは、worktreeでビルドしたコミットeb7e3081のバイナリである。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月8日15時10分から18時2分に行った。
両エンジンの`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は2である。
同じ機械でlishogiのBotが動いていた。

本測定の前に、同じ条件で3ペアのスモークテストを行った（[smoke.out](mcts-royal-capture-temperature-royal-stc/smoke.out)と[smoke-contention.out](mcts-royal-capture-temperature-royal-stc/smoke-contention.out)）。
時間切れとエンジンの異常はなく、GPUのメモリの使用量は最大832 MiB、使用率は93〜96%であった。
2局を同時に指している最中に別のプロセスから同じ局面を探索すると、毎秒の探索回数は11,836回であり、単独のときの約14,500回より約18%少なかった（[simulation-rate-under-contention.out](mcts-royal-capture-temperature-royal-stc/simulation-rate-under-contention.out)）。

## 結果

145ペアで`decision: H1`となった（[集計](mcts-royal-capture-temperature-royal-stc/results.out)）。
ペンタノミアル度数は、先後を入れ替えた2局の候補の得点が0、0.5、1、1.5、および2であるペアの数である。

| 項目 | 値 |
|---|---|
| 有効ペア | 144 |
| 破棄ペア | 1 |
| ペンタノミアル度数 | [4, 13, 71, 28, 28] |
| LLR | +2.956 |
| 判定 | `H1` |
| エンジンの異常、時間切れ、拒否着手 | 0件 |
| 経過時間 | 10,325秒 |

破棄した1ペアは、ペア142の1局目が手数の上限4,096手に達して打ち切られたものである。
有効ペアの候補の得点率は60.9%（288局で175.5点）であり、ロジスティックのEloに換算すると約+77である。
この換算は記録者の概算であり、GSPRTの判定には使っていない。
有効な対局と打ち切りを含む290局の終局は、王駒の捕獲が109局、詰みが108局、反復が72局、手数の上限が1局であった。
開始局面の乱択の手順を含む1局の手数は、打ち切りを除く289局で中央値462手、最長2,701手であった。

## 結論

候補はSTCで`H1`となったので、LTCへ進める。

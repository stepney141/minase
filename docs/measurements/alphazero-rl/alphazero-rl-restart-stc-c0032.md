# 学び直した本線のサイクル32とαβ探索のSTCの対局

## 目的

2026年10月31日に学び直した強化学習の本線のサイクル32のネットを使うMCTSが、αβ探索とNNUEの組合せにどれだけ離れているかを、STCの対局で確かめる。
この対局は統括計画の到達の判定ではなく、進め方を決めるための材料である。
設計書はサイクル50で最初に行うとしていたが、2026年11月2日の利用者の決定で前倒しした。

## コマンドライン

本記録と同じ名前のディレクトリの`run-stc.sh`を作業機で実行した。

```console
target/release/minase match run --run-dir data/matches/alphazero-rl-restart-stc-c0032 --seed 2026110232 \
    --candidate commit:45d5db5498e1739b2307b3719891a1eec24f7fbd --baseline commit:45d5db5498e1739b2307b3719891a1eec24f7fbd \
    --candidate-args "--search mcts --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx" \
    --candidate-option NnBatchSize=16 --each time=10000+100 --concurrency 1 elo --pairs 200
```

固定200ペアの予定であったが、結果が明らかになったので、利用者の指示で97ペアの時点で止めた。
ハーネスの最終集計は出力されていないので、結果は実行ディレクトリの`pairs/`から求めた。
信頼区間は、ハーネスと同じく正規化ペア得点の平均と分散から求めた。

## エンジン

候補と基準はどちらもコミット45d5db54である。
候補はMCTS（`--search mcts`）にサイクル32のネット（NASの`nets/cycle-0032.onnx`）を与え、基準はαβ探索とそのコミットに埋め込まれたNNUEである。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月2日12時3分から13時42分に行った。
両エンジンの`Threads`は1、`USI_Hash`は256 MB、同時対局数は1、時間制御は`time=10000+100`である。
MCTSの推論はTensorRTのFP16で、`NnBatchSize`は16である。
同じ機械でlishogiのBotが動いていた。

## 結果

有効な97ペアのペンタノミアル度数は[95, 0, 2, 0, 0]であり、194局でMCTSの2勝192敗、引き分けは0であった。
MCTSの1局あたりの得点は0.0103、Eloの点推定は−793で、95%信頼区間の上限は−640である（全敗に近いので下限は意味を持たない）。
エンジンの異常と時間切れは0件であった。
終局は王駒の捕獲が127局、詰みが67局で、対局の長さの中央値は237.5手、最長は476手であった。
最初の91ペアの1手の思考時間の中央値は、MCTSが134ミリ秒、αβ探索が82ミリ秒であった。

## 結論

サイクル32のMCTSは、STCでαβ探索とNNUEの組合せに大差で負ける。
到達の判定はLTCで行うが、この差が持ち時間だけで埋まるとは考えにくいので、学習を続けてサイクル50で同じ条件の対局をやり直す。

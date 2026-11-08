# 葉での王駒の捕獲の判定の費用

## 目的

[設計書](../../plans/mcts-royal-capture-temperature.md)のフェーズ1の候補が、MCTSの毎秒の探索回数を下げるかを、基準と候補のバイナリで同じ局面を交互に探索して確かめる。

## コマンドライン

本記録と同じ名前のディレクトリの`run.sh`を、作業機で次のように実行した。
`run.sh`は、[`simulation_rate.py`](../adhoc/alphazero-rl-c0032-value-bias/simulation_rate.py)をバッチ16で、基準と候補の順に3回ずつ交互に実行する。

```console
W=data/worktrees/mcts-royal-capture-temperature
bash $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-cost/run.sh \
    data/worktrees/mcts-royal-capture-base/target/release/minase $W/target/release/minase 3 \
    $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-cost/simulation-rate.out
```

`simulation_rate.py`は、STCの実行ディレクトリ`data/matches/alphazero-rl-restart-stc-c0032`の最初のペアの1局目から、20手目から200手目まで20手ごとの10局面を取り出す。
各局面を、残り8秒と加算0.1秒の時計（`clock`）と、800回の固定（`nodes800`）で探索し、探索回数、思考時間、および毎秒の探索回数の中央値を出力する。

## エンジン

基準は分岐元のmasterのコミット4885ccb7、候補はブランチ`mcts-royal-capture-temperature`のコミットeb7e3081である。
どちらも同じビルド設定で、それぞれのworktreeで`cargo build --release --bin minase`によりビルドした。
基準のworktree`data/worktrees/mcts-royal-capture-base`は、測定の後に削除した。
MCTSにはNASの`nets/cycle-0032.onnx`を与え、推論はTensorRTのFP16である。
規則は`engine-default`（`L0,P0,R1,E0`）である。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月8日に、ほかの測定を走らせずに行った。
同じ機械でlishogiのBotが動いていた。
対局ではないので、同時対局数、ペンタノミアル度数、および時間切れの項目はない。

## 結果

出力は[simulation-rate.out](mcts-royal-capture-temperature-royal-cost/simulation-rate.out)にある。
値は10局面の毎秒の探索回数の中央値である。

| 回 | 基準 `clock` | 候補 `clock` | 基準 `nodes800` | 候補 `nodes800` |
|---|---|---|---|---|
| 1 | 14,509 | 14,637 | 13,793 | 13,797 |
| 2 | 14,648 | 14,631 | 13,676 | 13,914 |
| 3 | 14,493 | 14,493 | 13,559 | 13,914 |

基準と候補の値の範囲は、`clock`では14,493〜14,648と14,493〜14,637で重なり、`nodes800`では候補のほうがわずかに高い。

## 結論

判定による毎秒の探索回数の低下は、3回の測定の揺れの中に収まり、検出できなかった。

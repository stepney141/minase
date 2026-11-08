# 葉での王駒の捕獲の判定の発動の確認

## 目的

[設計書](../../plans/archived/mcts-royal-capture-temperature.md)のフェーズ1の候補（新しい節点の手番側が相手の最後の王駒を取れるなら1手の勝ちに確定させる判定）が、王駒を取られる手を選ぶ局面で実際に働くかを、[サイクル32の価値の診断](../adhoc/alphazero-rl-c0032-value-bias.md)と同じ123局面で確かめる。

## コマンドライン

本記録と同じ名前のディレクトリの`run.sh`を、作業機で次のように実行した。
`run.sh`は、診断のスクリプト[`proofs_and_hanging.py`](../adhoc/alphazero-rl-c0032-value-bias/proofs_and_hanging.py)を、候補のバイナリ、サイクル32のネット、およびSTCの実行ディレクトリ`data/matches/alphazero-rl-restart-stc-c0032`を引数として実行する。

```console
W=data/worktrees/mcts-royal-capture-temperature
bash $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-activation/run.sh \
    $W/target/release/minase \
    $W/docs/measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-royal-activation/proofs-and-hanging.out
```

スクリプトは、STCの対局のうちMCTSが王駒の捕獲で負けた局から、開始局面の1手負けを除く123局について、MCTSが最後に手を選んだ局面を取り出す。
その局面を、前の探索木を持ち越さずに2,000回と20,000回で探索し直し、選んだ手の直後に相手が最後の王駒を取れるかを、同じバイナリのαβ探索の深さ2で調べる。
あわせて、αβ探索の深さ8で強制負けを読むかを調べる。

## エンジン

候補はブランチ`mcts-royal-capture-temperature`のコミットeb7e3081であり、worktreeで`cargo build --release --bin minase`によりビルドした。
MCTSにはNASの`nets/cycle-0032.onnx`を与え、推論はTensorRTのFP16、`NnBatchSize`は16である。
規則は`engine-default`（`L0,P0,R1,E0`）である。
王駒を取れるかの判定と強制負けの判定に使うαβ探索は、同じバイナリに埋め込まれたNNUEを使う。
このNNUEは、元の診断のコミット45d5db54のものと異なる（2026年11月4日の[NNUEの構成の見直し](../../plans/archived/nnue-architecture-revision.md)の統合による）。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB）で、2026年11月8日12時56分から12時58分に行った。
同じ機械でlishogiのBotが動いていた。
対局ではないので、同時対局数、ペンタノミアル度数、および時間切れの項目はない。

## 結果

出力は[proofs-and-hanging.out](mcts-royal-capture-temperature-royal-activation/proofs-and-hanging.out)にある。
検出の対照として実際に指された最後の手を調べると、123局すべてで王駒を取れると判定された。

| 探索回数 | 王駒を取られる手を選んだ局面（全123局面） | 同（αβの深さ8が強制負けを読まなかった局面） | 根が負けに確定した局面 |
|---|---|---|---|
| 基準 2,000 | 121 | 65/67 | 4 |
| 基準 20,000 | 106 | 54/67 | 15 |
| 候補 2,000 | 34 | 0/68 | 41 |
| 候補 20,000 | 35 | 0/68 | 47 |

基準の行は、元の診断の記録（コミット45d5db54）の値である。
αβの深さ8が強制負けを読んだ局面は、元の診断の56局面に対して55局面であった。
αβ探索のNNUEが異なるためであり、MCTSの比較には影響しない。

## 結論

判定は発動し、αβの深さ8が強制負けを読まなかった68局面では、王駒を取られる手を選ぶ局面がなくなった。
残る34〜35局面は、いずれもαβの深さ8が強制負けを読んだ55局面に含まれる。

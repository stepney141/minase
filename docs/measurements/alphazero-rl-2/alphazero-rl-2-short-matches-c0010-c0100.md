# 2回目の強化学習のサイクル10と100のネットと、固定の相手との短い対局

## 目的

2回目の強化学習（[設計書](../../plans/alphazero-rl-2.md)）のサイクル10と100のネットが、強さを測る固定の2つの相手にどのくらい勝てるかの手がかりを得て、定例の測定にかかる時間を見積もる。
本書の対局は各8ペア（16局）の短い対局であり、Eloの信頼区間は広い。
設計書は10サイクルごとに、1手500ノードのαβ探索との400ペアと、深さ3のHaChuとの100ペアの定例の測定を定めているが、サイクル10と100ではどちらも行わなかった。
サイクル10では、この短い対局で全敗したことを受けて、利用者が定例の測定を行わずにサイクルを進めると決めた。
サイクル11〜100も、利用者の決定で、10サイクルごとの定例の測定を行わずに進めた。

## コマンドライン

本体のリポジトリの根で、bashで次のように実行した。
`$R`は、定例の測定用にMCTSを固定した作業ツリー`data/worktrees/alphazero-rl-2-routine`（コミット`8e419486`）、`$A`はNASの`/mnt/synology_nas/minase/alphazero`、`NNNN`は`0010`か`0100`である。

```bash
source $R/tools/train/zero/env.sh
export XDG_CACHE_HOME=$HOME/.cache
M=8e419486e7cf5c93bbefb4f856bc203f2213cd21
F=$R/docs/measurements/mcts-fixed-opponents/mcts-fixed-opponents-scripts
N=$A/nets/cycles-rl2-az/cycle-NNNN.onnx
python3 -I $F/run_match.py alphabeta --harness $R/target/release/minase --mcts-commit $M --net $N --nodes 500 \
  --name alphazero-rl-2-progress-cNNNN-timing-ab500 --seed <シード> --pairs 8 --concurrency 4
python3 -I $F/run_match.py hachu --harness $R/target/release/minase --mcts-commit $M --net $N \
  --hachu ../hachu-debian/hachu --depth 3 --name alphazero-rl-2-progress-cNNNN-timing-hachu3 --seed <シード> --pairs 8 --concurrency 4
python3 -I $F/summarize.py <4つの実行ディレクトリ> > alphazero-rl-2-short-matches-c0010-c0100/summary.json
```

シードは、サイクル10のαβ探索との対局が2026111621、HaChuとの対局が2026111622、サイクル100がそれぞれ2026111731と2026111732である。
`--name`で指定した測定名の4つの実行ディレクトリは、本体のリポジトリの`data/matches/<測定名>.tar.zst`へ圧縮して残した。

## エンジン

MCTSは、作業ツリー`$R`の`minase`に`--search mcts`を指定したものであり、1手800シミュレーション、`NnBatchSize`は16、推論はTensorRTのFP16である。
ネットは、サイクル10が`nets/cycles-rl2-az/cycle-0010.onnx`（SHA-256 `ba89c56c…`）、サイクル100が`nets/cycles-rl2-az/cycle-0100.onnx`（SHA-256 `8ab8574a…`）である。
相手と対局の条件は[強さを測る固定の相手の設計書](../../plans/archived/mcts-fixed-opponents.md)のとおりである。
αβ探索はminaseのコミット`45d5db54`とそのNNUEを1手500ノードで使い、規則セット`lishogi`、手数の上限4,096手で対局させた。
HaChuは、同じ設計書で基準を測ったときと同じバイナリ（SHA-256 `10db1299…`）を深さ3で使い、HaChuの既定の設定に対応する規則`L1,L3,P0,P5,P6,R2,E1,E2`、手数の上限1,900手で対局させた。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti 8 GB）で、サイクル10の対局を2026年11月16日に、サイクル100の対局を2026年11月17日に行った。
1手の探索量をノード数、深さ、およびシミュレーション回数で制限したので、対局の指し手と勝敗は負荷の影響を受けない。

## 結果

| サイクル | 相手 | ペンタノミアル度数 | 得点率 | Elo（95%信頼区間） | 手数の中央値（最小〜最大） | 実行時間 |
|---|---|---|---|---|---|---|
| 10 | αβ探索 500ノード | [8, 0, 0, 0, 0] | 0% | −∞ | 143.5手（87〜197手） | 125秒 |
| 10 | HaChu 深さ3 | [8, 0, 0, 0, 0] | 0% | −∞ | 122手（55〜172手） | 104秒 |
| 100 | αβ探索 500ノード | [7, 0, 1, 0, 0] | 6.25% | −470 [−∞, −267] | 265.5手（104〜799手） | 284秒 |
| 100 | HaChu 深さ3 | [8, 0, 0, 0, 0] | 0% | −∞ | 276.5手（153〜741手） | 280秒 |

ペンタノミアル度数は、MCTSから見たペアの得点（0、0.5、1、1.5、2）ごとのペア数である。
相手を基準としたMCTSのEloと手数は、集計スクリプト`summarize.py`が出力した値である。
どの対局も、手数の上限で破棄したペアはなく、エンジンの異常終了、時間切れ、反則、HaChuの拒否着手、および不正着手は0件であった。
実行時間は、ハーネスが各実行ディレクトリに書いた`summary.json`の`active_wall_time_ns`である。

## 結論

サイクル10のネットは、どちらの相手にも16局すべて負けた。
サイクル100のネットは、αβ探索に16局中1局勝ち、HaChuには16局すべて負けた。
同じ相手と条件で基準として測った1回目の強化学習のネットは、αβ探索に対するEloがサイクル10で−576（得点率3.5%）、サイクル32で+17であり（[基準の記録](../mcts-fixed-opponents/mcts-fixed-opponents-alphabeta-baseline.md)）、2回目のサイクル100は、1回目のサイクル10前後の水準にあるとみられる（推論、16局の結果）。

サイクル100のネットの対局は、サイクル10より手数が増えて実行時間が長くなった。
サイクル100の1ペアあたりの実行時間（約35秒）から、定例の測定のαβ探索との400ペアには約4時間、HaChuとの100ペアには約1時間かかると見込む（推論）。

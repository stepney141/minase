# サイクル25の重みの移動平均のネットと平均でないネットの比較

## 目的

[強化学習の実行の設計書](../../plans/alphazero-rl.md)の「学習が動いてから決める技法」の節に従い、サイクル25の学習で得た重みの移動平均（EMA）のネットが、同じ学習の平均でないネットより強いかをGSPRTで判定し、以後の自己対局に平均のネットを使うかを決める。

## コマンドライン

[ema-gsprt.sh](alphazero-rl-ema-c0025/ema-gsprt.sh)で次を実行した。

```console
minase match run --run-dir ema-c0025 --seed 30025 \
  --candidate "minase --protocol usi --rules engine-default --search mcts --nn-model nets/cycle-0025-ema.onnx" \
  --baseline "minase --protocol usi --rules engine-default --search mcts --nn-model nets/cycle-0025.onnx" \
  --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
  --each nodes=800 --concurrency 2 gsprt --max-pairs 3000
```

仮説は既定（H0はelo=0、H1はelo=10、α=β=0.05）である。
上限3,000ペアに達した判定保留は採らないと、測定の前に決めていた。

## エンジン

`minase`は、ブランチ`alphazero-rl`のworktreeでmasterのコミットd786c5e1と同じ`crates/`からリリースビルドしたものである。
候補は`nets/cycle-0025-ema.onnx`（SHA-256 `d1fedcac99c7de7bd9c53ef92894ec9e9276f5387370d27952269b94474a96c4`）、基準は`nets/cycle-0025.onnx`（SHA-256 `d716570c2445825455def96d4f58edc4183cb8769f570790897e21a05a59d397`）である。
どちらもコミット4ec063c1の学習器がサイクル25で書き出したものであり、平均はサイクル21の学習からサイクル20の重みを初期値として、50万標本ごとに1/8の割合で9回更新した。
規則セットは`engine-default`（L0＋P0＋R1＋E0）である。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはRTX 3060 Ti 8 GB）で、2026年10月30日17時54分から22時23分に行った。
MCTSは`Threads=1`、`USI_Hash`は既定、同時対局数は2である。

## 結果

| 項目 | 値 |
|---|---|
| ペア | 433（破棄0） |
| ペンタノミアルの度数 | [49, 108, 156, 85, 35] |
| LLR | −2.967 |
| 判定 | `H0` |
| 平均のネットの得点率 | 0.471 |
| エンジンの異常 | 0件 |
| 経過時間 | 16,116秒 |

得点率は、ペンタノミアルの度数から求めた候補の1局あたりの得点である（[出力](alphazero-rl-ema-c0025/summary.txt)）。
同じサイクル25の最新の検証集合では、平均のネットの損失のほうが大きかった（方策2.909対2.675、対局結果に対する価値0.682対0.655）。
学習が速く進んでいる間は、平均が過去の重みを引きずって現在の重みより遅れるためとみられる。

## 結論

サイクル25の重みの移動平均のネットは、平均でないネットより10 Elo以上強いとは言えなかった（`H0`）ので、以後の自己対局と進捗測定には平均でないネットを使い続ける。

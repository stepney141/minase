# 段階8の最終構成と段階開始版の固定200ペア測定

## 目的

[棋力向上段階8](../plans/strength-stage8.md)の最終構成を段階開始版と固定200ペアで比較し、段階ごとの強さの積み上がりを進捗指標として記録する。採否の根拠には使わない。

## コマンドライン

対局ハーネスは、コミットを固定したworktreeでビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-elo200 --seed 35000919 \
  --candidate commit:0c1bd76 --baseline commit:dfdbe40 \
  --rules engine-default --each time=10000+100 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --max-ply 4096 --response-timeout 120 \
  elo --pairs 200
target/release/match_report --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-elo200
```

## エンジン

候補は段階8の最終構成であるコミット`0c1bd76722a2a372bb5dff9af03e5ad7b50a0650`（バイナリのSHA-256 `87a4832600df6bdb0960768f5d45790ca2e6738857641d462c204b64333e2418`）であり、段階開始版にSEEによる捕獲手の枝刈り（SEE pruning）と静的評価の補正（correction history）を加えたものである。
基準は段階開始版のコミット`dfdbe40c6abd1193072700beaa05bbda42d1e470`（バイナリのSHA-256 `5e07bdf5280659f073ab1d9b79c6e4bbb8e502bd871fafc15e252c82b060ec3e`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。2026年9月21日に実施した。
同じ機械でlishogi Botのエンジンが対局しており、測定中にProbCutの対象率の診断（1スレッド、最低優先度）が並行した。

## 結果

値は`match_report`が保存記録から再計算したものである。

| 項目 | 値 |
|---|---|
| ペア数 | 200（有効193、手数上限到達による破棄7） |
| ペンタノミアル度数 | [27, 6, 88, 12, 60] |
| Elo | +65.57、95%信頼区間[+32.41, +99.97] |
| 正規化ペア得点の平均 | 0.5933（標準誤差0.0239） |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 局時間 | 中央値57.1秒、95%点136.7秒 |
| 最大常駐メモリ | 1局あたり最大613 MB、同時対局の保守的な合計9.8 GB（実メモリの29.5%） |
| 経過時間 | 1,789.8秒（`summary.json`の`active_wall_time_ns = 1789762353065`） |

## 結論

段階8の最終構成は、段階開始版に対して短時間条件の固定200ペアで+65.57 Elo、95%信頼区間[+32.41, +99.97]であり、異常は0件であった。

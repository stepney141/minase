# 棋力向上段階8の静的評価の補正のSPRT（LTC）

## 目的

[棋力向上段階8](../plans/strength-stage8.md)のフェーズ5として、探索値と静的評価の差を駒種別の枚数の鍵ごとに学習し、futility pruningの判定に使う静的評価を補正する構成（correction history）（コミット`a000948`）が、SEEによる捕獲手の枝刈りの採用構成（コミット`6689f27`、探索は`c1c7227`と同一）より長時間条件で有意に強いかをGSPRTで判定し、採否を決める。[STC](strength-stage8-correction-stc.md)は`H1`であった。

## コマンドライン

対局ハーネスは、コミットを固定したworktreeでビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-correction-ltc --seed 34000919 \
  --candidate commit:a000948 --baseline commit:6689f27 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=60000+200 gsprt
```

## エンジン

候補はコミット`a0009489428b91005f34345ec04ce7083b0b6e43`（バイナリのSHA-256 `9fb864fb0251b0b1841001b108a98bd67ddb28e0a9787d4a8a976dbc96a5bfe2`）である。補正は[再診断](strength-stage8-correction-bench.md)で選んだ構成であり、最善手が捕獲手でなく手番側の王駒に相手の利きが届かないノードでだけ更新し、鍵は双方の駒種別の枚数の組と手番側、重みは`min(depth, 8) / 32`、補正値の上限は歩兵2枚分である。`bench`深さ6の総ノード数は基準の1,556,722から1,813,911へ16.5%増え、NPSの中央値は2.3%高い。
基準はコミット`6689f2794ae11d3ccfddc7245c08026949f68656`（バイナリのSHA-256 `7796a42758926e54e1df7003b93132388b70b209f0910fee2b6e0d131d99ff51`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4,096、応答タイムアウトは120秒、H1は10 Eloで、2026年9月20日から21日にかけて実施した。
同じ機械でlishogi Botのエンジンが対局していた。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 722（有効679、手数上限到達による破棄43） |
| ペンタノミアル度数 | [107, 28, 351, 40, 153] |
| LLR | 2.9846 |
| 判定 | `H1` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 19,912.7秒（`summary.json`の`active_wall_time_ns = 19912719781326`） |

## 結論

`H1`であり、エンジン異常、時間切れ、および拒否着手が0件なので、静的評価の補正（correction history）を採用する。
LLRは100ペアあたり約+0.25の一定の傾きで上がり、679ペアで境界を越えた。

# 棋力向上段階8のSEEによる捕獲手の枝刈りのSPRT（STC）

## 目的

[棋力向上段階8](../plans/strength-stage8.md)のフェーズ2として、SEEが余裕値を下回る捕獲手を浅い非PVノードで展開しない構成（コミット`c1c7227`）が、直前のコミット（`1e62031`、探索は段階開始版`dfdbe40`と同一）より短時間条件で有意に強いかをGSPRTで判定し、通過ならLTCへ進める。

## コマンドライン

対局ハーネスは、候補のコミットを固定したworktreeでビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-see-capture-stc --seed 30000919 \
  --candidate commit:c1c7227 --baseline commit:1e62031 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`c1c7227f1fc6375a1b201db3e51e2114da637819`（バイナリのSHA-256 `1580f472640feeae43c5015adac044e7f09845557372f089bc15c6dfc9b35d9a`）であり、残り深さ1から3の非PVノードで、SEEが`−m`未満の捕獲手（`m`は残り深さ順に0、200、0）を展開しない。`bench`深さ6の総ノード数は基準の2,040,262から1,556,722へ23.7%減る。
基準はコミット`1e62031dd61ce027e92c6ec43bf1c88113855a2f`（バイナリのSHA-256 `49dc514335547e6825cd55fcf06c79ca3a68b4d6252769bf754c226c50c0ca51`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4,096、応答タイムアウトは120秒、H1は10 Eloで、2026年9月19日から20日にかけて実施した。
同じ機械でlishogi Botのエンジンが対局しており、測定の後半には再診断の道具の動作確認（2並列のビルドと1スレッドのbench、いずれも最低優先度）が並行した。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 496（有効470、手数上限到達による破棄26） |
| ペンタノミアル度数 | [76, 15, 233, 19, 127] |
| LLR | 2.9685 |
| 判定 | `H1` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 4,696.8秒（`summary.json`の`active_wall_time_ns = 4696787777293`） |

## 結論

`H1`かつ異常0件なので、SEEによる捕獲手の枝刈りをLTCへ進める。

# 棋力向上段階8のimprovingフラグのSPRT（STC）

## 目的

[棋力向上段階8](../plans/strength-stage8.md)のフェーズ3として、静的評価が2手前より上がっていないノードでfutility pruningの余裕値を縮める構成（コミット`189f874`）が、SEEによる捕獲手の枝刈りの採用構成（コミット`639d7ed`、探索は`c1c7227`と同一）より短時間条件で有意に強いかをGSPRTで判定し、通過ならLTCへ進める。

## コマンドライン

対局ハーネスは、コミットを固定したworktreeでビルドしたバイナリから起動した。

```console
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/strength-stage8-improving-stc --seed 32000919 \
  --candidate commit:189f874 --baseline commit:639d7ed \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`189f8744b6e60175558e971d3e951723dd448447`（バイナリのSHA-256 `d8dad5bb1ec65047cb7ad2b865a1a4db05706b978142210fd4f01c3a9ead54be`）であり、improvingでないノードのfutility余裕値を残り深さ順に現行の1/4、1/2、1/2（12、75、75）へ縮める。LMRの追加減深は[再診断](strength-stage8-improving-bench.md)で外したので含まない。`bench`深さ6の総ノード数は基準の1,556,722から1,356,811へ12.8%減る。
基準はコミット`639d7edc9830c052c1f4aaa6d022196df086d187`（バイナリのSHA-256 `f142bd5411be67adbf0681ba3ace4fb5a8da85d519303ef2eafc41e3e7d24eaa`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4,096、応答タイムアウトは120秒、H1は10 Eloで、2026年9月20日に実施した。
同じ機械でlishogi Botのエンジンが対局していた。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 726（有効694、手数上限到達による破棄32） |
| ペンタノミアル度数 | [170, 25, 344, 20, 135] |
| LLR | −2.9907 |
| 判定 | `H0` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 6,678.3秒（`summary.json`の`active_wall_time_ns = 6678256769368`） |

## 結論

`H0`なので、improvingフラグは採用せず、LTCへ進めない。
候補の2連敗のペアが170、2連勝のペアが135で、候補は負け越した。
診断で選んだ倍率は失う良い結果の割合を10%以下に抑えていたが、探索するノードを12.8%減らした利得は、失った良い結果の損失を上回らなかった。

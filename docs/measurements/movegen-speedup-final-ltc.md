# 合法手生成高速化の採用版と基準の最終SPRT（LTC）

## 目的

合法手生成と利き計算の高速化の採用版（単位AからDまでを含み、PGOを適用したコミット`5a67d41`）が、基準コミット`0e3dfa6`より長時間条件で有意に強いかをGSPRTで判定し、マイルストーンの完了を決める。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir data/matches/movegen-speedup-final-ltc --seed 20270915 \
  --candidate commit:5a67d41 --baseline commit:0e3dfa6 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=60000+200 gsprt
```

シードはSTCの基本シード20260915からペア数上限3,000以上離れた未使用の値である。

## エンジン

候補はコミット`5a67d41e7c50dfd31b13d109c9f984c9c7e31609`（バイナリのSHA-256 `6c77792d05aa97c43b3448262a5df6f68d841744fcf01102706ebd4b6598e854`）であり、探索のソースは単位Dの採用版`d961adf`と同じで、`git archive`からの検証つきPGOビルドである。
基準はコミット`0e3dfa6b59c489049c15e2cd00455e190a3fbf52`（バイナリのSHA-256 `24d3cc6d5e6b86e571d84c0d3f565b1579a60f893d424d47c8ca22c60e44a302`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4096、応答タイムアウトは120秒で、2026年9月15日に実施した。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 164（有効157、手数上限到達による破棄7） |
| ペンタノミアル度数 | [12, 4, 71, 7, 63] |
| LLR | 2.9556 |
| 判定 | `H1` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 5,001.9秒（`summary.json`の`active_wall_time_ns = 5001858379159`） |

## 結論

候補は長時間条件で`H1`かつエンジン異常、時間切れ、拒否着手が0件なので採用し、マイルストーンを完了とする。

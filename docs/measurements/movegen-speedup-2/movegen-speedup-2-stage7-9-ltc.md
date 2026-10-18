# 第2期高速化の段階7から段階9と段階6までの採用版のSPRT（LTC）

## 目的

[STC](movegen-speedup-2-stage7-9-stc.md)を通過した段階7から段階9の構成（コミット`24ddf4f`）が、段階6までの採用版（コミット`c6e2539`）より長時間条件で有意に強いかをGSPRTで判定し、`H1`かつ異常0件なら採用する。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir /home/stepney141/board-games/minase/data/matches/movegen-speedup-2-stage7-9-ltc --seed 20290916 \
  --candidate commit:24ddf4f --baseline commit:c6e2539 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=60000+200 gsprt
```

シードはSTCの基本シード20280916からペア数上限3,000以上離した未使用の値である。

## エンジン

候補はコミット`24ddf4f9502642706600c26b6827546f7e4b54be`（バイナリのSHA-256 `4c01ba076f94d4ea5595d214e16bba532d57dc02763e29e9906a3a9eb2d25d76`）、基準はコミット`c6e253924587424bd019e4feaee7c59fb5652e84`（バイナリのSHA-256 `6606e2a7ae45221c5ada1aa66fbab7c353ad2f5076cad0d52ab571eee04a58cd`）であり、STCと同じバイナリである。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4,096、応答タイムアウトは120秒、H1は10 Eloで、2026年9月16日から17日にかけて実施した。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 1,581（有効1,515、手数上限到達による破棄66） |
| ペンタノミアル度数 | [276, 42, 804, 62, 331] |
| LLR | 2.9778 |
| 判定 | `H1` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 42,843.3秒（`summary.json`の`active_wall_time_ns = 42843335502091`） |

## 結論

`H1`かつエンジン異常、時間切れ、拒否着手が0件なので、段階7から段階9の構成を採用する。判定に1,515ペア（約11.9時間）を要したので、効果はH1の10 Eloに近い。段階8は段階5の獅子候補の圧縮を、対象が居喰いだけになるため取り除いており、圧縮の再評価はこの除去で完了している。

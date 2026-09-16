# 第2期高速化の段階7から段階9と段階6までの採用版のSPRT（STC）

## 目的

事前選別を通過した段階7、段階8、段階9を1コミットに固定した構成（コミット`24ddf4f`）が、段階6までの採用版（コミット`c6e2539`）より短時間条件で有意に強いかをGSPRTで判定し、`H1`ならLTCへ進める。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir /home/stepney141/board-games/minase/data/matches/movegen-speedup-2-stage7-9-stc --seed 20280916 \
  --candidate commit:24ddf4f --baseline commit:c6e2539 \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`24ddf4f9502642706600c26b6827546f7e4b54be`（バイナリのSHA-256 `4c01ba076f94d4ea5595d214e16bba532d57dc02763e29e9906a3a9eb2d25d76`）であり、段階6までの採用版に段階7（静的評価の先行打ち切り）、段階8（静止探索の獅子候補の絞り込み）、段階9（捕獲対象が空のノードでの置換表アクセスの省略）を加えたものである。`bench`（深さ5）の固定深さの経過時間は段階6までの採用版の0.880倍、総ノード数は0.947倍である（[事前選別](movegen-speedup-2-stage7-9-prescreen-depth5.md)）。
基準はコミット`c6e253924587424bd019e4feaee7c59fb5652e84`（バイナリのSHA-256 `6606e2a7ae45221c5ada1aa66fbab7c353ad2f5076cad0d52ab571eee04a58cd`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4,096、応答タイムアウトは120秒、H1は10 Eloで、2026年9月16日に実施した。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 1,074（有効1,012、手数上限到達による破棄62） |
| ペンタノミアル度数 | [184, 30, 521, 38, 239] |
| LLR | 2.9461 |
| 判定 | `H1` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 10,384.9秒（`summary.json`の`active_wall_time_ns = 10384855774499`） |

## 結論

`H1`かつ異常0件なので、段階7から段階9の構成をLTCへ進める。判定に1,012ペアを要したので、効果はH1の10 Eloに近い小さな値と見込まれ、LTCは長時間になり得る。

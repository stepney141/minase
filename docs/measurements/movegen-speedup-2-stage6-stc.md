# 第2期高速化の段階6までの採用版と第1期の採用版のSPRT（STC）

## 目的

第2期の段階6までの採用版（コミット`c6e2539`）が、第1期の採用版（新しい基準、コミット`7a41b2c`）より短時間条件で有意に強いかをGSPRTで判定し、`H1`ならLTCへ進める。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir /home/stepney141/board-games/minase/data/matches/movegen-speedup-2-stage6-stc --seed 20260916 \
  --candidate commit:c6e2539 --baseline commit:7a41b2c \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

実行ディレクトリは本体リポジトリの`data/matches/`に置き、worktree（`data/worktrees/movegen-speedup-2`）から起動した。

## エンジン

候補はコミット`c6e253924587424bd019e4feaee7c59fb5652e84`（バイナリのSHA-256 `6606e2a7ae45221c5ada1aa66fbab7c353ad2f5076cad0d52ab571eee04a58cd`）であり、段階1（`codegen-units = 1`）、段階2、段階4、段階5、段階6の採用版を含む。`bench`（深さ5）のNPSは基準比1.3854倍で、探索木は基準と同一である。
基準はコミット`7a41b2cbc7e10ab5a597156ec00219b657b764ff`（バイナリのSHA-256 `ec01ead0ca88e5317b15675fd2337faa03616069ab0278be2f45b6034386b216`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16である。手数上限は4,096、応答タイムアウトは120秒、H1は10 Eloで、2026年9月16日に実施した。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 270（有効252、手数上限到達による破棄18） |
| ペンタノミアル度数 | [32, 5, 125, 9, 81] |
| LLR | 2.9653 |
| 判定 | `H1` |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間 | 2,673.2秒（`summary.json`の`active_wall_time_ns = 2673198851623`） |

## 結論

`H1`かつ異常0件なので、段階6までの採用版をLTCへ進める。

# 第2期高速化の最終の採用版と第1期の採用版の固定200ペアのElo

## 目的

第2期の最終の採用版（段階9まで、コミット`24ddf4f`）が第1期の採用版（コミット`7a41b2c`）に対してどれだけ強くなったかを、固定200ペアのEloと95%信頼区間で進捗記録として測る。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir /home/stepney141/board-games/minase/data/matches/movegen-speedup-2-elo200 --seed 20300916 \
  --candidate commit:24ddf4f --baseline commit:7a41b2c \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 elo --pairs 200
cargo run --release --bin match_report -- \
  --run-dir /home/stepney141/board-games/minase/data/matches/movegen-speedup-2-elo200
```

## エンジン

候補はコミット`24ddf4f9502642706600c26b6827546f7e4b54be`（バイナリのSHA-256 `4c01ba076f94d4ea5595d214e16bba532d57dc02763e29e9906a3a9eb2d25d76`）であり、段階1、2、4、5、6、7、8、9の採用版を含む。`bench`（深さ5）の固定深さの経過時間は第1期の採用版の0.640倍である。
基準はコミット`7a41b2cbc7e10ab5a597156ec00219b657b764ff`（バイナリのSHA-256 `ec01ead0ca88e5317b15675fd2337faa03616069ab0278be2f45b6034386b216`）である。
規則セットは`L0,P0,R1,E0`（engine-default）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）であり、候補と基準はいずれも`Threads=1`、`USI_Hash`は各256 MB、同時対局数は16、時間制御は`time=10000+100`である。手数上限は4,096、応答タイムアウトは120秒で、2026年9月17日に実施した。

## 結果

`match_report`が保存記録から再計算した値は次のとおりである。

| 項目 | 値 |
|---|---|
| ペア数 | 200（有効191、手数上限到達による破棄9） |
| ペンタノミアル度数 | [17, 3, 85, 11, 75] |
| Elo | +117.01 |
| 95%信頼区間 | [+83.39, +152.89] |
| 正規化ペア得点の平均 | 0.6623 |
| 異常件数 | illegal_moves=0 crashes=0 timeouts=0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 総CPU時間 | 26,937.1秒 |
| 局時間の中央値 | 55.2秒（95パーセンタイル173.1秒） |
| 最大常駐メモリの最大値 | 610.6 MB（1局あたり） |
| 経過時間 | 2,216.3秒（`summary.json`の`active_wall_time_ns = 2216338402321`） |

## 結論

最終の採用版は第1期の採用版に対して短時間条件で+117 Elo（95%信頼区間[+83, +153]）であり、固定深さの経過時間0.640倍（約1.56倍の速度）から[持ち時間2倍の感度測定](sensitivity-time2x.md)の目安（2倍で約216 Elo）で見込まれる約139 Eloと同じ規模である。

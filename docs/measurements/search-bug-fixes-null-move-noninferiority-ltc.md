# search-bug-fixes-null-move-noninferiority-ltc

## 目的

[探索部の不具合修正](../plans/search-bug-fixes.md)のnull moveをまたぐ反復の修正が、masterより10 Elo規模で弱くなっていないことを、候補と基準を入れ替えたGSPRTで長時間条件（LTC）について判定する。
候補をmaster、基準を修正のコミットとし、`H0`（masterが10 Elo強いとは言えない）なら採用、`H1`（masterが有意に強い）なら不合格とする。

## コマンドライン

```console
data/worktrees/search-bug-fixes-runner/target/release/match_runner \
  --run-dir data/matches/search-bug-fixes-null-move-noninferiority-ltc --seed 83300000 \
  --candidate commit:b59e616720757a17228a689831e3a6acea20cf14 \
  --baseline commit:24c08011f5c96c778e0a98b15493be735829b0e0 \
  --concurrency 16 --each time=60000+200 gsprt
```

## エンジン

候補はコミット`b59e616720757a17228a689831e3a6acea20cf14`（master、監査の基準コミット）、基準はコミット`24c08011f5c96c778e0a98b15493be735829b0e0`（null moveの修正を加えたもの）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、両側の`Threads`は1、`USI_Hash`は256 MBである。
仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は明示した16である。
他の測定はなかったが、lishogiのBotとして動くminaseが約3.7コアを使っていた。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 850（うち37は手数上限4,096手で破棄） |
| 有効ペア数 | 813 |
| ペンタノミアル度数（候補=master側） | [181, 37, 413, 30, 152] |
| 候補（master）の得点率 | 48.0%（修正から見て約+14 Elo） |
| LLR | −2.949 |
| 判定 | `H0` |
| 不正着手・クラッシュ・応答タイムアウト・時間切れ・拒否着手 | すべて0件 |
| 経過時間 | 23,215秒（`summary.json`の`active_wall_time_ns`） |

## 結論

判定は`H0`であり、異常は0件なので、null moveの修正はLTCでも非劣性を通過し、採用する。
masterの得点率48.0%はSTC（45.7%）と同じく修正の側が強い方向を示すが、本測定は非劣性の判定であり、向上の大きさは主張しない。

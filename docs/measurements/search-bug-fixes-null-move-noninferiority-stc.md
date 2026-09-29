# search-bug-fixes-null-move-noninferiority-stc

## 目的

[探索部の不具合修正](../plans/search-bug-fixes.md)のnull moveをまたぐ反復の修正が、masterより10 Elo規模で弱くなっていないことを、候補と基準を入れ替えたGSPRTで短時間条件（STC）について判定する。
候補をmaster、基準を修正のコミットとし、`H0`（masterが10 Elo強いとは言えない）なら非劣性を通過してLTCへ進み、`H1`（masterが有意に強い）なら不合格とする。

## コマンドライン

```console
data/worktrees/search-bug-fixes-runner/target/release/match_runner \
  --run-dir data/matches/search-bug-fixes-null-move-noninferiority-stc --seed 83200000 \
  --candidate commit:b59e616720757a17228a689831e3a6acea20cf14 \
  --baseline commit:24c08011f5c96c778e0a98b15493be735829b0e0 \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`b59e616720757a17228a689831e3a6acea20cf14`（master、監査の基準コミット）、基準はコミット`24c08011f5c96c778e0a98b15493be735829b0e0`（null moveの修正を加えたもの）である。
ブランチ`search-bug-fixes`の先頭のソースはこのコミットと同一である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、両側の`Threads`は1、`USI_Hash`は256 MBである。
仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は明示した16である。
他の測定はなかったが、lishogiのBotとして動くminaseが約3.7コアを使っていた。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 505（うち25は手数上限4,096手で破棄） |
| 有効ペア数 | 480 |
| ペンタノミアル度数（候補=master側） | [128, 15, 237, 11, 89] |
| 候補（master）の得点率 | 45.7%（修正から見て約+30 Elo） |
| LLR | −2.950 |
| 判定 | `H0` |
| 不正着手・クラッシュ・応答タイムアウト・時間切れ・拒否着手 | すべて0件 |
| 経過時間 | 4,793秒（`summary.json`の`active_wall_time_ns`） |

## 結論

判定は`H0`であり、null moveの修正はSTCで非劣性を通過した。
masterの得点率45.7%は、修正の側がむしろ強い方向を示すが、本測定は非劣性の判定であり、向上の大きさは主張しない。
設計書のフェーズ2に従い、利用者の確認を経てLTCへ進む。

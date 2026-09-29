# search-bug-fixes-qsearch-evasion-diag-stc

## 目的

[探索部の不具合修正](../plans/search-bug-fixes.md)の非劣性のSTC（[search-bug-fixes-noninferiority-stc](search-bug-fixes-noninferiority-stc.md)）が不合格となったので、原因の診断として、第3の修正（静止探索の逃げる手）のコミットがその親コミットより10 Elo規模で弱いかを、候補と基準を入れ替えたGSPRTで調べる。
この測定は診断であり、採否を決めない。

## コマンドライン

```console
data/worktrees/search-bug-fixes-runner/target/release/match_runner \
  --run-dir data/matches/search-bug-fixes-qsearch-evasion-diag-stc --seed 83030000 \
  --candidate commit:602f7e1a3109dd75857f1af2bb57a793cc19e005 \
  --baseline commit:45f43f250eb20a19f2b026cf9fc10588c4789c8d \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`602f7e1a3109dd75857f1af2bb57a793cc19e005`（第1と第2の修正まで）、基準はコミット`45f43f250eb20a19f2b026cf9fc10588c4789c8d`（第3の修正を加えたもの）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、両側の`Threads`は1、`USI_Hash`は256 MBである。
仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は明示した16である。
他の測定はなかったが、lishogiのBotとして動くminaseが約3.7コアを使っていた。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 154（うち4は手数上限4,096手で破棄） |
| 有効ペア数 | 150 |
| ペンタノミアル度数（候補=親コミット側） | [10, 4, 69, 8, 59] |
| 候補（親コミット）の得点率 | 67.0% |
| LLR | 2.963 |
| 判定 | `H1` |
| 不正着手・クラッシュ・応答タイムアウト・時間切れ・拒否着手 | すべて0件 |
| 経過時間 | 1,383秒（`summary.json`の`active_wall_time_ns`） |

## 結論

第3の修正を加えると、親コミットより有意に弱くなる（得点率67.0%は約120 Eloの差に相当する）。
非劣性のSTCで観測した約140 Eloの差の大部分は、この修正による。

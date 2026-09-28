# search-bug-fixes-delta-promotion-diag-stc

## 目的

[探索部の不具合修正](../plans/search-bug-fixes.md)の非劣性のSTC（[search-bug-fixes-noninferiority-stc](search-bug-fixes-noninferiority-stc.md)）が不合格となったので、原因の診断として、delta pruningの成り益の修正のコミットがその親コミットより10 Elo規模で弱いかを、候補と基準を入れ替えたGSPRTで調べる。
この測定は診断であり、採否を決めない。

## コマンドライン

```console
data/worktrees/search-bug-fixes-runner/target/release/match_runner \
  --run-dir data/matches/search-bug-fixes-delta-promotion-diag-stc --seed 83020000 \
  --candidate commit:24c08011f5c96c778e0a98b15493be735829b0e0 \
  --baseline commit:602f7e1a3109dd75857f1af2bb57a793cc19e005 \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`24c08011f5c96c778e0a98b15493be735829b0e0`（null moveの修正まで）、基準はコミット`602f7e1a3109dd75857f1af2bb57a793cc19e005`（delta pruningの修正を加えたもの）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、両側の`Threads`は1、`USI_Hash`は256 MBである。
仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は明示した16である。
他の測定はなかったが、lishogiのBotとして動くminaseが約3.7コアを使っていた。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 3,000（上限。うち136は手数上限4,096手で破棄） |
| 有効ペア数 | 2,864 |
| ペンタノミアル度数（候補=親コミット側） | [560, 92, 1503, 96, 613] |
| 候補（親コミット）の得点率 | 51.0%（約+6.7 Elo） |
| LLR | 0.932 |
| 判定 | `pending`（上限到達） |
| 不正着手・クラッシュ・応答タイムアウト・時間切れ・拒否着手 | すべて0件 |
| 経過時間 | 27,221秒（`summary.json`の`active_wall_time_ns`） |

LLRは有効1,209ペアの時点で1.95まで上がった後、上限到達時に0.93まで下がった。

## 結論

delta pruningの修正が親コミットより10 Elo規模で弱いとは判定されなかったが、LLRは正であり、修正前の版がわずかに強い方向の証拠が残った。
利用者の決定（上限到達で判定が出なかった場合は、null moveの修正の診断へ進む前に報告する）に従い、この結果で測定を止め、delta pruningの修正を組に残すかどうかを利用者が決める。

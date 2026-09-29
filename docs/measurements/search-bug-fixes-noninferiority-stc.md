# search-bug-fixes-noninferiority-stc

## 目的

[探索部の不具合修正](../plans/search-bug-fixes.md)の3件の修正を積んだ最終コミットが、masterより10 Elo規模で弱くなっていないことを、候補と基準を入れ替えたGSPRTで短時間条件（STC）について判定する。
候補をmaster、基準を修正の最終コミットとし、`H0`（masterが10 Elo強いとは言えない）なら非劣性を通過、`H1`（masterが有意に強い）なら不合格とする。

## コマンドライン

```console
data/worktrees/search-bug-fixes-runner/target/release/match_runner \
  --run-dir data/matches/search-bug-fixes-noninferiority-stc --seed 83000000 \
  --candidate commit:b59e616720757a17228a689831e3a6acea20cf14 \
  --baseline commit:45f43f250eb20a19f2b026cf9fc10588c4789c8d \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

`match_runner`は、測定中にmasterが進んでも再開できるよう、監査の基準コミットに固定したworktree `data/worktrees/search-bug-fixes-runner`でビルドしたものである。

## エンジン

候補はコミット`b59e616720757a17228a689831e3a6acea20cf14`（master、監査の基準コミット）、基準はコミット`45f43f250eb20a19f2b026cf9fc10588c4789c8d`（ブランチ`search-bug-fixes`のフェーズ3完了時点）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、両側の`Threads`は1、`USI_Hash`は256 MBである。
仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、実メモリ約31 GiB、同時対局数は明示した16である。
他の測定はなかったが、lishogiのBotとして動くminaseが約3.7コアを使っていた。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 143（うち8は手数上限4,096手で破棄） |
| 有効ペア数 | 135 |
| ペンタノミアル度数（候補=master側） | [7, 1, 66, 4, 57] |
| 候補（master）の得点率 | 69.1% |
| LLR | 2.970 |
| 判定 | `H1` |
| 不正着手・クラッシュ・応答タイムアウト・時間切れ・拒否着手 | すべて0件 |
| 経過時間 | 1,677秒（`summary.json`の`active_wall_time_ns`） |

保存記録の全手番を集計すると、修正版の平均到達深さは9.89（着手98,294手）、masterは10.01（着手98,361手）であり、1手あたりの平均思考時間はどちらも約130ミリ秒であった。
深さ9までの所要時間が39%延びた（[search-bug-fixes-bench](search-bug-fixes-bench.md)）にもかかわらず到達深さはほぼ同じなので、負けの主因は探索の速さではなく、同じ深さで選ぶ着手の質の低下と考えられる。

## 結論

判定は`H1`であり、修正の最終コミットは非劣性を通過せず、不採用である。
得点率69.1%はmasterの約140 Eloの優位に相当し、10 Eloの非劣性の限界を大きく超える。
設計書のフェーズ4に従い、各修正コミットをその親コミットと比べる診断のSTCで原因を調べる。

# 候補B対対照Aの短時間測定（STC）

## 目的

[探索局面を用いた評価関数の学習](../plans/search-aware-evaluation.md)のフェーズ5として、半数を探索局面へ置き換えて学んだ候補Bを、同じ組の通常局面だけで学んだ対照Aと短時間GSPRTで比べ、学習分布の変更に利得があるかを振り分ける。

## コマンドライン

```console
data/worktrees/teacher-mixing-ratio-runner/target/release/match_runner \
  --run-dir data/matches/search-aware-pst-b-vs-a-stc --seed 161000925 \
  --candidate commit:89c273740a61d9983063baf6890da76b134c3db5 \
  --baseline commit:5a1c7bbe2fdd77bbb08d166c6924d65c509716dc \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はブランチ `search-aware-pst-b` の `89c2737`、基準はブランチ `search-aware-pst-a` の `5a1c7bb` である。
どちらもS0 `5563d75` に `nets/pst.bin` の差し替えだけを加えたコミットであり、重みの学習は[対照Aと候補Bの学習](search-aware-pst-training.md)による。
規則セットは `engine-default`（L0、P0、R1、E0）、両エンジンとも `Threads=1`、`USI_Hash` 256 MBである。
runnerは `622a879` の `match_runner`（SHA-256 `06fe7121…`）で、H1はelo=10、α=β=0.05である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
lishogi Botの `minase` が約4コアを使っていた（利用者の決定により、この負荷の下で同時16局で測った）。
2026年9月26日21時23分から21時35分に実施した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 101（破棄0） |
| ペンタノミアル度数 | [86, 2, 13, 0, 0] |
| LLR | −2.949 |
| 判定 | H0 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 695秒 |

候補Bの得点率は6.9%であり、101ペアのうち86ペアで2局とも負けた。ロジスティックElo換算では約−450に当たるが、この値は採否には使わない。
[採否前の診断](search-aware-pst-diagnostics.md)は、Bの探索が深さ8までにS0の約12倍のノードを要し、通常局面の評価を平均251センチポーン高く見積もることを示しており、この大差はそれと整合する。

## 結論

候補Bは短時間測定でH0となり、振分け規則に従い不採用とする。
事前登録の経路に従い、B対AのLTCとB対S0の測定は行わない。

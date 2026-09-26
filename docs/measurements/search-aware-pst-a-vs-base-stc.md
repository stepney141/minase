# 対照A対S0の短時間測定（STC）

## 目的

[探索局面を用いた評価関数の学習](../plans/search-aware-evaluation.md)のフェーズ5として、通常局面だけを深い教師で学び直した対照Aを、基点S0と短時間GSPRTで比べる。この測定は、Bの結果にかかわらず事前に登録したものである。

## コマンドライン

```console
data/worktrees/teacher-mixing-ratio-runner/target/release/match_runner \
  --run-dir data/matches/search-aware-pst-a-vs-base-stc --seed 165000925 \
  --candidate commit:5a1c7bbe2fdd77bbb08d166c6924d65c509716dc \
  --baseline commit:5563d7526f6d727f195603aadf96147607209a2b \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はブランチ `search-aware-pst-a` の `5a1c7bb`（S0に `nets/pst.bin` の差し替えだけを加えたコミット）、基準はS0 `5563d75` である。
規則セットは `engine-default`（L0、P0、R1、E0）、両エンジンとも `Threads=1`、`USI_Hash` 256 MBである。
runnerは `622a879` の `match_runner`（SHA-256 `06fe7121…`）で、H1はelo=10、α=β=0.05である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
lishogi Botの `minase` が約4コアを使っていた（利用者の決定により、この負荷の下で同時16局で測った）。
2026年9月26日21時35分から22時25分に実施した。

## 結果

| 量 | 値 |
|---|---:|
| 有効ペア | 276（破棄16、手数上限） |
| ペンタノミアル度数 | [85, 6, 142, 0, 43] |
| LLR | −2.970 |
| 判定 | H0 |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 2,999秒 |

対照Aの得点率は41.8%であり、ロジスティックElo換算では約−57に当たる。この値は採否には使わない。
[最終診断](search-aware-pst-diagnostics.md)ではAの着手の損失はS0とほぼ同じであったが、深さ8までの探索ノード数はS0の1.56倍であった。

## 結論

対照Aは短時間測定でH0となり、振分け規則に従い不採用とする。
Bも不採用なので、事前登録の経路に従いA対S0のLTCは行わない。

# strength-stage6-tt-cluster-replay

## 目的

棋力向上段階6の置換表のクラスタ化（コミット2767394）について、直前の採用構成（aspiration windowsのコミット75bb69d）と同じ対局再生で、過去の世代の深い記録の上書き率、照合成功率、および固定深さ6で各手を探索したときの総ノード数を比べ、設計書のフェーズ2の基準（固定深さの総ノード数が3%以上減る）で採否測定へ進めるかを判定する。

## コマンドライン

コミット75bb69dと2767394をそれぞれ固定したworktreeへ、置換表の`probe`と`store`の分類計数と`bestmove`直前の`info string diag`報告を一時的に入れ、[置換表の対局再生診断](strength-stage6-tt-replay.md)と同じ20局（シード20600002）について次を実行した。

```console
MINASE_DIAG=1 python3 data/experiments/stage6-diag-tools/replay_diag.py <worktree>/target/release/minase \
  data/matches/strength-stage5-lmr-ltc 20 20600002 170000 4 replay_<版>_170k.json
python3 data/experiments/stage6-diag-tools/replay_depth.py <worktree>/target/release/minase \
  data/matches/strength-stage5-lmr-ltc 20 20600002 6 4 depth_<版>.json
```

固定深さの再生は、各手を`go depth 6`で探索し、反復完了の`info`行の`nodes`を手ごとに合計した。

## エンジン

直前の採用構成はコミット75bb69d、候補はコミット2767394であり、計数コードはコミットしていない。
いずれも`--protocol usi --rules L0,P0,R1,E0`、`USI_Hash`は既定の256 MBで、1局ごとに1プロセスを起動して置換表を手をまたいで保持した。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）であり、aspiration windowsのSTC（同時対局数16）と並行して4プロセスで走らせた。
固定ノード数と固定深さの探索なので、経過時間は使わない。
同時対局数は該当しない。

## 結果

1手17万ノードの再生（20局8,606手）は次のとおりである。

| 版 | 照合成功率 | 深い記録が過去の世代の深い記録を追い出す | 浅い記録が過去の世代の深い記録を追い出す | 深い記録1件あたりの浅い記録による追い出し |
|---|---|---|---|---|
| 直前の採用構成 | 25.9% | 7.00% | 8.72% | 0.379 |
| クラスタ化 | 25.9% | 6.41% | 7.50% | 0.326 |

固定深さ6の再生（同じ20局8,606手）の総ノード数は、直前の採用構成が1,802,231,687、クラスタ化が1,799,744,594であり、0.14%の減少であった。
破棄ペア数、異常件数、`time_forfeits`、対局の総経過時間は該当しない。

## 結論

クラスタ化は浅い記録による過去の深い記録の上書きを14%減らすが、照合成功率は変わらず、固定深さの総ノード数の減少は0.14%で基準の3%に届かない。
発動しない改良として採否測定へ進めず、実装をコードから外した（コミット9d37246）。

# strength-stage6-aspiration-bench

## 目的

棋力向上段階6のaspiration windows（コミット75bb69d）について、benchの総ノード数の変化と、標準時間制御の予算での窓の使用、窓外れ、および読み直し中の中断の回数を記録する。
発動は保存記録で確認済みなので、この計数は判定基準ではなく記録である。

## コマンドライン

benchは変更前（コミットf16c868、探索コードは段階開始版と同一）と変更後のそれぞれで次を実行した。

```console
bench --depth 4 --repetitions 1 --threads 1
bench --depth 5 --repetitions 1 --threads 1
bench --depth 6 --repetitions 1 --threads 1
```

標準時間制御の予算での計数は、コミット75bb69dを固定したworktreeへ反復数、窓を使った反復数、fail-low、fail-high、および読み直し中の中断の計数と`bestmove`直前の`info string diag`報告を一時的に入れ、次を実行した。

```console
MINASE_DIAG=1 python3 data/experiments/stage6-diag-tools/tc_diag.py <worktree>/target/release/minase \
  data/experiments/stage6-diag-tools/positions.json stc 10 tc_asp_stc.json
MINASE_DIAG=1 python3 data/experiments/stage6-diag-tools/tc_diag.py <worktree>/target/release/minase \
  data/experiments/stage6-diag-tools/positions.json ltc 10 tc_asp_ltc.json
```

局面は[反復深化の診断](strength-stage6-iteration-diag.md)と同じ240局面であり、各局面には手数帯ごとの候補側の着手前残り時間の中央値（[保存記録の再構成](strength-stage6-records-profile.md)）を両側の残り時間に、加算をSTCで100ms、LTCで200msとして`go`を与えた。

## エンジン

変更後はコミット75bb69d、変更前はコミットf16c868であり、計数コードはコミットしていない。
計数は`--protocol usi --rules L0,P0,R1,E0`、`USI_Hash`は既定の256 MBで、局面ごとに新しいプロセスを起動した。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）である。
benchは単一スレッドで、計数は10プロセスを並列に走らせた。
同時対局数は該当しない。

## 結果

benchの総ノード数は次のとおりである。

| 深さ | 変更前 | 変更後 | 変化 | 変更後NPS |
|---|---|---|---|---|
| 4 | 728,561 | 728,561 | 0% | 1,140,607 |
| 5 | 1,459,885 | 1,442,142 | −1.2% | 1,336,008 |
| 6 | 3,350,638 | 3,539,112 | +5.6% | 1,369,710 |

深さ4以下の反復は窓を使わないので総ノード数が一致し、深さ5以上では窓外れの読み直しにより変化する。

標準時間制御の予算での計数は次のとおりである。

| 予算 | 局面 | 平均完了深さ | hard停止 | 反復 | 窓を使った反復 | fail-low | fail-high | 読み直し中の中断 |
|---|---|---|---|---|---|---|---|---|
| STC | 240 | 5.99 | 30 | 1,467 | 508 | 136 | 117 | 11 |
| LTC | 240 | 7.51 | 10 | 1,812 | 852 | 195 | 199 | 5 |

窓を使った反復あたりの窓外れ（fail-lowとfail-highの合計）はSTCで49.8%、LTCで46.2%であり、[反復深化の診断](strength-stage6-iteration-diag.md)の代理指標（評価値の差が50以上、25.4%）より多い。
読み直しの連鎖の中で交互に外れる回数を含むためである。
破棄ペア数、異常件数、`time_forfeits`、対局の総経過時間は該当しない。

## 結論

窓は標準時間制御の予算で反復の35〜47%に使われ、その半数近くで読み直しが起こる。
採否は段階開始版とのSTCとLTCで判定する。

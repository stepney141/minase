# strength-stage6-time-signals-diag

## 目的

棋力向上段階6の時間管理の3項目（fail-lowによる延長、最善手交替時の延長、最善手安定時の早期終了）について、設計書の「標準時間制御の予算での計数」節の手順で、各信号が次の反復へ入るかの判断を実際に変えた件数を数え、発動率を判定する。

## コマンドライン

3項目を含むコミット14727a8を固定したworktreeへ、反復の完了時に直前の構成の判断と当該項目の信号を加えた判断を同じ経過時間で並べて評価する計数と、`bestmove`直前の`info string diag`報告を一時的に入れ、次を実行した。

```console
MINASE_DIAG=1 python3 data/experiments/stage6-diag-tools/tc_diag.py <worktree>/target/release/minase \
  data/experiments/stage6-diag-tools/positions.json stc 3 tc_final_stc.json
MINASE_DIAG=1 python3 data/experiments/stage6-diag-tools/tc_diag.py <worktree>/target/release/minase \
  data/experiments/stage6-diag-tools/positions.json ltc 3 tc_final_ltc.json
```

局面は[反復深化の診断](strength-stage6-iteration-diag.md)と同じ240局面であり、各局面には手数帯ごとの候補側の着手前残り時間の中央値（[保存記録の再構成](strength-stage6-records-profile.md)）を両側の残り時間に、加算をSTCで100ms、LTCで200msとして`go`を与えた。

## エンジン

コミット14727a8に計数を加えたリリースバイナリを`--protocol usi --rules L0,P0,R1,E0`、`USI_Hash`は既定の256 MBで、局面ごとに新しいプロセスとして起動した。
計数コードはコミットしていない。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）であり、aspiration windowsのLTC（同時対局数17）と並行して3プロセスで走らせた。
同時対局数は該当しない。

## 結果

| 予算 | 局面 | 平均完了深さ | 判断数 | fail-lowが変えた判断 | 最善手交替が変えた判断 | 安定が変えた判断 |
|---|---|---|---|---|---|---|
| STC | 240 | 5.96 | 1,430 | 28（2.0%、手の11.7%） | 18（1.3%、手の7.5%） | 48（3.4%、手の20.0%） |
| LTC | 240 | 7.59 | 1,822 | 47（2.6%、手の19.6%） | 20（1.1%、手の8.3%） | 51（2.8%、手の21.3%） |

最善手交替の件数は、fail-lowが既に続行を決めている判断を除いた値、安定の件数は延長の2項目を加えた判断と比べた値である。
括弧内は判断数を分母にした割合と、局面数（1局面1手）を分母にした割合である。
破棄ペア数、異常件数、`time_forfeits`、対局の総経過時間は該当しない。

## 結論

判断数を分母にすると3項目とも5%未満だが、1手の反復深化では最初の数反復の判断が経過時間の不足でどの規則でも続行になり、分母の大半を占める。
時間管理が働く単位は1手の停止の判断なので、発動率は手を分母にした割合で判定し、3項目とも7.5〜21.3%で基準の5%を満たすため、採否測定へ進める。

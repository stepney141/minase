# strength-stage6-records-profile

## 目的

棋力向上段階6の着手時に、段階5のLMRの減深量のSTCとLTCの保存記録から、候補側の完了深さ、停止理由、詰み帯の手の割合、および時計を再構成し、aspiration windowsの発動、mate distance pruningの見送り、および時間管理の設計の根拠にする。

## コマンドライン

```console
python3 data/experiments/stage6-diag-tools/records_profile.py \
  data/matches/strength-stage5-lmr-stc data/matches/strength-stage5-lmr-ltc
python3 scripts/clock_profile.py data/matches/strength-stage5-lmr-stc --role candidate
python3 scripts/clock_profile.py data/matches/strength-stage5-lmr-ltc --role candidate
```

## エンジン

集計した記録は[strength-stage5-lmr-stc](strength-stage5-lmr-stc.md)（820局）と[strength-stage5-lmr-ltc](strength-stage5-lmr-ltc.md)（2,272局）であり、候補はLMRの減深量を採用したコミット399c42f、基準はコミット7877531、規則セットは`L0,P0,R1,E0`である。
集計は候補側の手だけを対象にした。

## 環境

保存記録の集計であり、CPU、`Threads`、`USI_Hash`、同時対局数は元の測定記録が持つ。

## 結果

候補側の完了深さと停止理由は次のとおりである。

| 記録 | 手数 | 深さ5以上 | 最頻値 | 深さ5〜8の割合 | soft停止 | hard停止 | 詰み帯の手 |
|---|---|---|---|---|---|---|---|
| STC | 187,830 | 96.8% | 6（30.0%） | 75.7% | 87.7% | 12.3% | 1.02% |
| LTC | 553,225 | 99.9% | 8（27.0%） | 59.7% | 90.2% | 9.8% | 1.19% |

完了深さの分布は、STCで深さ4が3.0%、5が11.6%、6が30.0%、7が20.8%、8が13.3%、9が6.9%、10が4.1%、LTCで深さ5が0.8%、6が10.2%、7が21.7%、8が27.0%、9が16.3%、10が8.5%である。
深さ10を超える手は駒数の少ない終盤に集中する。

時計の再構成（`clock_profile.py`）による候補側の着手前の残り時間の中央値（ms）は次のとおりであり、基本時間はSTCで200手目、LTCで300手目までに使い切られている。

| 手数帯 | 0〜49 | 50〜99 | 100〜149 | 150〜199 | 200〜299 | 300〜399 | 400〜 |
|---|---|---|---|---|---|---|---|
| STC | 8,815 | 6,282 | 3,924 | 2,024 | 583 | 551 | 573 |
| LTC | 54,172 | 41,678 | 29,906 | 20,072 | 9,060 | 1,497 | 1,177 |

手数帯ごとの平均到達深さはSTCで6.1〜6.7（400手目以降は10.7）、LTCで7.5〜8.1（同12.3）である。
完了反復後に捨てられた計算時間の中央値は両記録とも6〜7msである。

## 結論

標準時間制御では96.8%以上の手が深さ5以上の反復を完了するので、深さ5以上の反復で働くaspiration windowsは発動する。
詰み帯の手は1.2%以下であり、mate distance pruningは発動率の基準5%を満たさないので見送る。
基本時間は序中盤で使い切られるので、時間管理の延長は総時間を増やさず配分を変える変更として測る。

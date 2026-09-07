# strength-stage5-lmr-ltc

## 目的

棋力向上段階5のLMRの減深量（残り深さと手番号の対数に基づく表`floor(ln(d) · ln(m) / 2.0)`をhistory値の閾値128で1増減し、`[0, min(3, d − 2)]`に切り詰める変更）を、直前の採用構成と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage5-lmr-ltc --seed 20510903 \
  --candidate commit:399c42f --baseline commit:7877531 \
  --each time=60000+200 --concurrency 14 gsprt
```

## エンジン

候補はコミット399c42f（分岐stage5-lmr-p1。採用後に分岐strength-stage5へ同じ内容を73578d4として取り込んだ）、基準は順序付けキーの重複計算の除去を採用したコミット7877531、規則セットは`L0,P0,R1,E0`である。
係数と閾値は[診断bench](strength-stage5-lmr-bench.md)で決めた`c = 2.00`、`H = 128`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した14である。
開始から約3時間はpiece-to historyのSTC（同時対局数4）が並行していた。

## 結果

1,118ペアを実行し、有効ペア1,105、破棄ペア13（手数上限）であった。
ペンタノミアル度数は[200, 36, 575, 30, 264]、LLRは+2.992で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は34,601秒である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、LMRの減深量を採用する。

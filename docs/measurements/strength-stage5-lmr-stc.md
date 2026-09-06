# strength-stage5-lmr-stc

## 目的

棋力向上段階5のLMRの減深量（残り深さと手番号の対数に基づく表`floor(ln(d) · ln(m) / 2.0)`をhistory値の閾値128で1増減し、`[0, min(3, d − 2)]`に切り詰める変更）を、直前の採用構成と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage5-lmr-stc --seed 20500903 \
  --candidate commit:399c42f --baseline commit:7877531 \
  --each time=10000+100 --concurrency 14 gsprt --max-pairs 3000
```

## エンジン

候補はコミット399c42f、基準は順序付けキーの重複計算の除去を採用したコミット7877531、規則セットは`L0,P0,R1,E0`である。
係数と閾値は[診断bench](strength-stage5-lmr-bench.md)で決めた`c = 2.00`、`H = 128`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した14である。
同じ機械でpiece-to historyのSTC（同時対局数4）が並行していた。

## 結果

409ペアを実行し、有効ペア408、破棄ペア1（手数上限）であった。
ペンタノミアル度数は[69, 6, 204, 4, 125]、LLRは+2.952で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は3,837秒である。

参考として、候補のbenchは深さ6で総ノード数が7,696,982から5,390,412へ30%減り、深さ5では1,553,119から1,560,273へ0.5%増えた。

## 結論

短時間GSPRTは`H1`かつ異常0件であり、候補をLTCへ進める。

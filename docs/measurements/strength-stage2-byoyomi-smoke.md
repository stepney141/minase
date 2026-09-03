# strength-stage2-byoyomi-smoke

## 目的

棋力向上段階2の採用構成どうしを秒読みつきの時間制御で10ペア対局させ、標準のSTCとLTCが通らない秒読みの経路で時間切れが起きないことを確認する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage2-byoyomi-smoke --seed 20260972 \
  --candidate commit:ce654d9 --baseline commit:ce654d9 \
  --each time=10000+100,byoyomi=200 elo --pairs 10
```

## エンジン

候補と基準はともに採用構成のコミットce654d9、規則セットは`engine-default`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

ペンタノミアル度数は[2, 0, 7, 0, 1]、Eloの点推定は−34.9（95%信頼区間−161.4〜+82.8）であった。
破棄ペアは0、不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は216秒である。
`scripts/clock_profile.py`によると、主時間は50手目までに加算分の100msまで下がり、以後は秒読みの8割を中心とする予算で1手あたり中央値132〜142msを使い、hard打ち切りは2〜3割であった。

## 結論

秒読みの経路で時間切れは0件であり、採用構成の秒読みの安全性を確認した。

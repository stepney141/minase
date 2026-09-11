# strength-stage6-byoyomi-smoke

## 目的

棋力向上段階6の最終構成どうしを秒読みつきの時間制御で10ペア対局させ、標準のSTCとLTCが通らない秒読みの経路で、最善手安定時の早期終了を含む時間管理が時間切れを起こさないことを確認する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-byoyomi-smoke --seed 20710903 \
  --candidate commit:8f4e41c --baseline commit:8f4e41c \
  --each time=10000+100,byoyomi=200 --concurrency 4 elo --pairs 10
```

## エンジン

候補と基準はともに最終構成のコミット8f4e41c、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した4である。
段階開始版との固定200ペア（同時対局数12）が並行していた。

## 結果

ペンタノミアル度数は[3, 0, 5, 0, 2]、Eloの点推定は−34.9（95%信頼区間−206.9〜+120.6）であった。
破棄ペアは0、不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は441秒である。
`scripts/clock_profile.py`によると、主時間は100手目までに加算分の100msまで下がり、以後は秒読みの8割を中心とする予算で1手あたり中央値121〜131msを使い、hard打ち切りは1〜2割であった。

## 結論

秒読みの経路で時間切れは0件であり、最終構成の秒読みの安全性を確認した。

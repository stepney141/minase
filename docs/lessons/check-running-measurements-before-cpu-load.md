# 測定機で重い計算を始める前に進行中の測定を確認する

## 症状

2026年9月25日に、SPSAの利得の模擬比較（全コアの20スレッドで約6分）を2回続けて実行した。
その間、同じ測定機では同時対局数16のLTC採否測定（`spsa-stage9-20260923-ltc`）が進行しており、約13分ぶんの対局が高負荷の下で指された。
LTC側の時間切れとエンジン異常は0件だった。

## 原因

模擬比較はシードごとの実行を`available_parallelism`の全スレッドへ分担する。
採否測定はユーザーサービスとしてバックグラウンドで走るため、実行前にその存在を確かめなかった。

## 以後の規則

測定機でCPUを多く使う処理（ビルド、模擬実験、bench、学習）を始める前に`pgrep -a match_runner`と`pgrep -a spsa_runner`で進行中の測定を確かめ、走っていれば処理のスレッド数を「物理コア数−同時対局数−1」以内に抑えるか、測定の終了を待つ。

## 出典

- [measurements/spsa-gain-simulation.md](../measurements/spsa-gain-simulation.md)
- [measurements/spsa-stage9-20260923-ltc.md](../measurements/spsa-stage9-20260923-ltc.md)

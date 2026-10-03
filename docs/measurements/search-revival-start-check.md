# search-revival-start-check

## 目的

[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)の開始点の確認として、新しい13係数を開始値に置いた処置群TとMを固定100ペアのSTCで対局させ、Tの得点率が40%以上であることを確かめる。

## コマンドライン

```console
match_runner --run-dir data/matches/search-revival-start-check --seed 2026700000 \
  --candidate commit:85f6e87 --baseline commit:b96a931 \
  --each time=10000+100 --concurrency 16 elo --pairs 100
```

## エンジン

候補はコミット85f6e87（Mに8項目を戻し、新しい係数を開始値に置いた処置群T）、基準はコミットb96a931（評価関数のPSTに重みG23を採用したmaster、M）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は既定の256MB、同時対局数は16である。

## 結果

100ペアを実行し、有効ペア90、手数上限による破棄ペア10であった。
ペンタノミアル度数は[14, 4, 48, 1, 23]であり、候補の得点率は54.2%（97.5／180）、Eloは+29.0（95%信頼区間 −17.5〜+76.6）である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は1,808秒である。

## 結論

得点率54.2%は基準の40%以上なので、開始値を寄せ直さずに調整セッションへ進む。
この測定は開始点の確認だけを目的とし、採否の根拠にはしない。

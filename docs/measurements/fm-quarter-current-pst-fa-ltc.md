# fm-quarter-current-pst-fa-ltc

## 目的

[STC](fm-quarter-current-pst-fa-stc.md)で`H1`となった候補Fa（先読みつきの教師で学んだFMの補正1/4）の採否を、段階ゲートのLTCで測る。

## コマンドライン

```console
data/pst-longer-training-runner/minase match run \
  --run-dir data/matches/fm-quarter-current-pst-fa-ltc --seed 24000000 \
  --candidate commit:a4b2cc1ba32981c40bb0d2a9e72c26b662fabc16 \
  --baseline commit:86d8a89ec371c88185e2a421a73fc1d478084976 \
  --each time=60000+200 --concurrency 16 gsprt
```

runnerはSTCと同じ`minase`（SHA-256 `238cdf95…`）である。

## エンジン

候補はブランチ`fm-quarter-current-pst-fa`の`a4b2cc1`（バイナリのSHA-256 `1933fb57…`）、基準はM（master `86d8a89`、バイナリのSHA-256 `92b90e0c…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月6日20時52分23秒に開始し、経過時間は7,529秒（約2時間5分）だった。
直前まで走っていた候補FnのLTCは、利用者の判断で一時停止してから本測定を始めた。
本測定の間、他の作業の対局は重なっていない。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 265（完走260、手数上限による破棄5） |
| ペンタノミアル度数 | [32, 3, 137, 12, 76] |
| 正規化得点 | 0.5933 |
| LLR | 2.995 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

LTCは`H1`であり、エンジン異常、時間切れ、および拒否着手が0件なので、Faは段階ゲートの採用の条件を満たした。
正規化得点は、ロジスティックのEloに換算して約+65であり、STCの約+67とほぼ同じである。
探索速度が約14%落ちる費用を払ったうえで、採用PSTのみの構成に勝った。

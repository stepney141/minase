# fm-quarter-current-pst-fn-ltc

## 目的

[STC](fm-quarter-current-pst-fn-stc.md)で`H1`となった候補Fn（先読みを使わない教師で学んだFMの補正1/4）の採否を、段階ゲートのLTCで測る。
この測定は、判定が出る前に利用者の判断で打ち切った。

## コマンドライン

```console
data/pst-longer-training-runner/minase match run \
  --run-dir data/matches/fm-quarter-current-pst-fn-ltc --seed 12000000 \
  --candidate commit:23f318e541c544ee1aaa0906f6efe60a1695d634 \
  --baseline commit:86d8a89ec371c88185e2a421a73fc1d478084976 \
  --each time=60000+200 --concurrency 16 gsprt
```

runnerはSTCと同じ`minase`（SHA-256 `238cdf95…`）である。

## エンジン

候補はブランチ`fm-quarter-current-pst`の`23f318e`（バイナリのSHA-256 `435adfe9…`）、基準はM（master `86d8a89`、バイナリのSHA-256 `92b90e0c…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月6日16時2分33秒に開始し、20時52分15秒にrunnerを止めた（約4時間50分）。
本測定の間、他の作業の対局は重なっていない。

## 結果

止めた時点で保存されていたペアは634（ペア番号は最大638）であり、完走610、手数上限による破棄24である。
ペンタノミアル度数は[126, 18, 306, 16, 144]、正規化得点は0.5139であり、ロジスティックのEloに換算して約+9.7（95%信頼区間 約−8.9〜+28.4）である。
ログに出力された最後のLLRは0.41であり、測定の後半の数時間は−0.9から+0.4の間を行き来していた。
不正着手、クラッシュ、応答タイムアウト、時間切れ、および拒否着手は、保存されたペアにはなかった。

## 結論

判定は保留のまま、2026年10月6日に一時停止し、10月7日に打ち切った。
打ち切りの理由は、もう一方の候補Faが[LTC](fm-quarter-current-pst-fa-ltc.md)で`H1`（約+65 Elo）となり、Fnの点推定（約+10 Elo）と信頼区間がほとんど重ならないので、Fnの判定を待っても採用する候補は変わらない見込みが大きいことである。
Fnは採用しない。
STCの約+38 EloからLTCの約+10 Eloへ差が縮んだ原因は調べていない。

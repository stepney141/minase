# fm-quarter-current-pst-fn-stc

## 目的

[補正1/4のFMの再学習](../plans/fm-quarter-current-pst.md)のフェーズ3として、候補Fn（先読みを使わない教師）の採否を、段階ゲートのSTCで測る。

## コマンドライン

```console
data/pst-longer-training-runner/minase match run \
  --run-dir data/matches/fm-quarter-current-pst-fn-stc --seed 11000000 \
  --candidate commit:23f318e541c544ee1aaa0906f6efe60a1695d634 \
  --baseline commit:86d8a89ec371c88185e2a421a73fc1d478084976 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

runnerは、[エポック数の延長](../plans/pst-longer-training.md)の測定に使った`minase`（SHA-256 `238cdf95…`）を流用した。そのビルド元の`aee6b87`とM（`86d8a89`）の間で、対局ハーネスのコードは変わっていない。

## エンジン

候補はブランチ`fm-quarter-current-pst`の`23f318e`（バイナリのSHA-256 `435adfe9…`）であり、Mの採用PST（候補L）にFMの補正1/4を加えたものである（[学習の記録](fm-quarter-current-pst-training.md)）。
基準はM（master `86d8a89`、バイナリのSHA-256 `92b90e0c…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月6日13時42分51秒に開始し、経過時間は5,491秒（約92分）だった。
13時44分から14時ごろまで、別の作業の秒読みの測定（`byoyomi-b1-short`、同時対局数11）が重なり、本測定の16局と合わせて約27局が物理20コアで走った。重なりは測定時間の約2割であり、負荷は候補と基準に等しくかかり、時間切れとエンジン異常は0件だった。本測定の開始時には他の対局がないことを確かめており、重なった測定は本測定の開始の約2分後に始まった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 545（完走523、手数上限による破棄22） |
| ペンタノミアル度数 | [97, 8, 253, 13, 152] |
| 正規化得点 | 0.5550 |
| LLR | 2.965 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

STCは`H1`なので、振分け規則に従ってLTCへ進める。
正規化得点は、ロジスティックのEloに換算して約+38である。

# fm-quarter-current-pst-fa-stc

## 目的

[補正1/4のFMの再学習](../plans/fm-quarter-current-pst.md)のフェーズ3として、候補Fa（先読みつきの教師）の採否を、段階ゲートのSTCで測る。

## コマンドライン

```console
data/pst-longer-training-runner/minase match run \
  --run-dir data/matches/fm-quarter-current-pst-fa-stc --seed 23000000 \
  --candidate commit:a4b2cc1ba32981c40bb0d2a9e72c26b662fabc16 \
  --baseline commit:86d8a89ec371c88185e2a421a73fc1d478084976 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

runnerは、[エポック数の延長](../plans/pst-longer-training.md)の測定に使った`minase`（SHA-256 `238cdf95…`）を流用した。そのビルド元の`aee6b87`とM（`86d8a89`）の間で、対局ハーネスのコードは変わっていない。

## エンジン

候補はブランチ`fm-quarter-current-pst-fa`の`a4b2cc1`（バイナリのSHA-256 `1933fb57…`）であり、Mの採用PST（候補L）にFMの補正1/4を加えたものである（[学習の記録](fm-quarter-current-pst-training.md)）。
基準はM（master `86d8a89`、バイナリのSHA-256 `92b90e0c…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月6日15時15分22秒に開始し、経過時間は2,679秒（約45分）だった。
他の作業の対局は重なっていない。終盤の約15分に、対局ハーネスの引数を加える作業のビルドとテスト（`nice -n 19`、2並列）を同じ測定機で行った。時間切れとエンジン異常は0件だった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 282（完走273、手数上限による破棄9） |
| ペンタノミアル度数 | [37, 5, 139, 1, 91] |
| 正規化得点 | 0.5952 |
| LLR | 2.952 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

STCは`H1`なので、振分け規則に従ってLTCへ進める。
正規化得点は、ロジスティックのEloに換算して約+67である。

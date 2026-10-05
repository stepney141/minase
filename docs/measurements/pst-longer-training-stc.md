# pst-longer-training-stc

## 目的

[採用PSTの追加学習のエポック数の延長](../plans/pst-longer-training.md)のフェーズ4として、[候補L](pst-longer-training-training.md)を載せたCLの採否を、段階ゲートのSTCで測る。

## コマンドライン

```console
data/pst-longer-training-runner/minase match run \
  --run-dir data/matches/pst-longer-training-stc --seed 21000000 \
  --candidate commit:66b4ad453ffd5c6a149128042310e924470ac891 \
  --baseline commit:aee6b87e933237cd1b364ef20c27f7a7483ba6d0 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

runnerは、M（`aee6b87`）でビルドした`minase`を`data/pst-longer-training-runner/`へ複製して固定したものである。

## エンジン

候補はCL（ブランチ`pst-longer-training-candidate`の`66b4ad4`、バイナリのSHA-256 `10d3d8b6…`）であり、Mの`crates/minase/nets/pst.bin`を候補Lへ差し替えたものである。
基準はM（master `aee6b87`、採用PSTはPc、バイナリのSHA-256 `5f84a3b1…`）である。
runnerのSHA-256は`238cdf95…`であり、`invocations.json`に記録されている。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月6日2時9分に開始し、経過時間は1,782秒（約30分）だった。
同じ時間帯に他の対局は走っていなかった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 175（完走171、手数上限による破棄4） |
| ペンタノミアル度数 | [16, 2, 80, 3, 70] |
| 正規化得点 | 0.6594 |
| LLR | 2.965 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

STCは`H1`なので、振分け規則に従ってLTCへ進める。
正規化得点0.659は、ロジスティックのEloに換算して約+115であり、Pcが採用時にSTCで示した得点率（0.574）を大きく上回る。
候補Lは評価値の尺度を約14%大きくし、探索の余裕値とのずれを残したまま、この結果を得た。

# pst-longer-training-ltc

## 目的

[STC](pst-longer-training-stc.md)で`H1`となった[候補L](pst-longer-training-training.md)を載せたCLの採否を、段階ゲートのLTCで決める。

## コマンドライン

```console
data/pst-longer-training-runner/minase match run \
  --run-dir data/matches/pst-longer-training-ltc --seed 22000000 \
  --candidate commit:66b4ad453ffd5c6a149128042310e924470ac891 \
  --baseline commit:aee6b87e933237cd1b364ef20c27f7a7483ba6d0 \
  --each time=60000+200 --concurrency 16 gsprt
```

runnerはSTCと同じく、M（`aee6b87`）でビルドした`minase`を複製して固定したものである。

## エンジン

候補はCL（ブランチ`pst-longer-training-candidate`の`66b4ad4`、バイナリのSHA-256 `10d3d8b6…`）、基準はM（master `aee6b87`、バイナリのSHA-256 `5f84a3b1…`）である。
runnerのSHA-256は`238cdf95…`である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月6日3時14分8秒に開始し、経過時間は3,141秒（約52分）だった。

測定の後半に、別の作業の対局が同じ測定機で走っていた。
先読みつきの秒読みの煙試験が2本（`byoyomi-a-smoke-ponder-short`が3時24分から3時38分、`byoyomi-a-smoke-ponder-long`が3時38分から3時57分）、いずれも同時対局数2で走った。
先読みでは両エンジンが同時に探索するので、約4コアを使う。
本測定の16局と合わせて約20コアとなり、物理コア数の上限に達して、1コアを空ける運用条件を満たさなかった。
負荷は候補と基準に等しくかかり、時間切れとエンジン異常は0件だった。
直前の秒読みの煙試験（同時対局数16）は3時14分4秒に終わっており、本測定とは重なっていない。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 117（完走116、手数上限による破棄1） |
| ペンタノミアル度数 | [3, 0, 54, 3, 56] |
| 正規化得点 | 0.7349 |
| LLR | 2.954 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

LTCは`H1`であり、エンジン異常、時間切れ、および拒否着手が0件なので、段階ゲートの規則に従って候補Lを採用する。
正規化得点0.735は、ロジスティックのEloに換算して約+177であり、STCの約+115より大きい。
後半の約33分に測定機のコアが上限まで使われたが、負荷は両エンジンに等しく、時間切れもなく、差の大きさから見て結論は変わらないと判断した。

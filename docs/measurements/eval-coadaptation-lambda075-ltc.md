# eval-coadaptation-lambda075-ltc

## 目的

[STC](eval-coadaptation-lambda075-stc.md)で`H1`となった対照Cc（G23の上へλ=0.75のまま10エポックを重ねたPST）の採否を、段階ゲートのLTCで決める。

## コマンドライン

```console
data/eval-coadaptation-runner/match_runner \
  --run-dir data/matches/eval-coadaptation-lambda075-ltc --seed 16000000 \
  --candidate commit:4114723aac1907202d36b1b9a52693623a666ab5 \
  --baseline commit:4fb158284648c26be039544385359e5704e13cb5 \
  --each time=60000+200 --concurrency 16 gsprt
```

runnerはSTCと同じく、master `4fb1582`でビルドした`match_runner`を複製して固定したものであり、現在の[手引き](../guides/sprt.md)の`minase match run`と同じ対局と統計を行う。

## エンジン

候補はCc（ブランチ`eval-coadaptation-lambda075`の`4114723`、バイナリのSHA-256 `749cdade…`）、基準はM（master `4fb1582`、バイナリのSHA-256 `5cb17c63…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MB、runnerのSHA-256は`616751c0…`である。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月5日11時59分に開始し、経過時間は10,968秒（約3.0時間）だった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 380（完走363、手数上限による破棄17） |
| ペンタノミアル度数 | [56, 12, 169, 22, 104] |
| 正規化得点 | 0.5730 |
| LLR | 2.967 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

LTCは`H1`であり、エンジン異常、時間切れ、および拒否着手が0件なので、段階ゲートの規則に従ってCcを採用する。
Pcを新しい採用PSTとしてmasterへ統合する。

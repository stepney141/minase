# eval-coadaptation-lambda075-stc

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)の追加診断で、固定400ペアでS0に約55 Elo勝った対照Cc（G23の上へλ=0.75のまま10エポックを重ねたPST）の採否を、段階ゲートのSTCで測る。

## コマンドライン

```console
data/eval-coadaptation-runner/match_runner \
  --run-dir data/matches/eval-coadaptation-lambda075-stc --seed 15000000 \
  --candidate commit:4114723aac1907202d36b1b9a52693623a666ab5 \
  --baseline commit:4fb158284648c26be039544385359e5704e13cb5 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

runnerは、補助ツールを`minase`のサブコマンドへ移す前のmaster `4fb1582`でビルドした`match_runner`を複製して固定したものである。
現在の[手引き](../guides/sprt.md)の`minase match run`と同じ対局と統計を行う。

## エンジン

候補はCc（ブランチ`eval-coadaptation-lambda075`の`4114723`、バイナリのSHA-256 `749cdade…`）であり、Mの`crates/minase/nets/pst.bin`を[Pc](eval-coadaptation-lambda075-training.md)へ差し替えたものである。
基準はM（master `4fb1582`、バイナリのSHA-256 `5cb17c63…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MB、runnerのSHA-256は`616751c0…`である。
GSPRTの仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月5日10時52分から11時59分に実施し、経過時間は4,018秒だった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 361（完走340、手数上限による破棄21） |
| ペンタノミアル度数 | [48, 9, 177, 7, 99] |
| 正規化得点 | 0.5735 |
| LLR | 2.950 |
| 判定 | `H1` |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

## 結論

STCは`H1`なので、振分け規則に従ってLTCへ進める。

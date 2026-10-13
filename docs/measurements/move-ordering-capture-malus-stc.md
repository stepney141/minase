# move-ordering-capture-malus-stc

## 目的

[手の順序付けの改善](../plans/archived/move-ordering.md)の候補1（静かな手のβカット時の捕獲履歴の減点、capture history malus）を、段階開始版と同じコードの基準に対して短時間条件のGSPRTで判定する。本測定は、利用者の判断により判定前に停止した。

## コマンドライン

```console
cargo run --release --bin minase -- match run \
  --run-dir data/matches/move-ordering-capture-malus-stc --seed 2100000000 \
  --candidate commit:09716b6 --baseline commit:cd081f4 \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`09716b6`（候補1を実装したコミット）、基準はコミット`cd081f4`（フェーズ1の測定記録を加えただけの文書コミットであり、コードは段階開始版のmaster `b784fe5`と同一）である。規則は`engine-default`（`L0,P0,R1,E0`）、`USI_Hash`は既定の256 MBである。

## 環境

2026年10月10日、Intel Core Ultra 7 265KF（物理20コア、論理20コア）で測定した。候補と基準のワーカー数は1、同時対局数は16（lishogi Botのminaseが2プロセス同居するため明示した）。

## 結果

| 項目 | 値 |
|---|---:|
| 確定ペア数 | 820（保存済みペアは856） |
| ペンタノミアル度数 | [164, 17, 446, 15, 178] |
| LLR | +0.08 |
| 判定 | pending（判定前に停止） |
| 破棄ペア数、エンジン異常、`time_forfeits`、拒否着手 | 停止時点の最終サマリがないため記録なし。ペアの保存記録に異常の記載はない |
| 累計実行時間 | 約2時間12分（`summary.json`の`active_wall_time_ns`） |

開始から約22分で123ペア（LLR +0.13）、約1時間で634ペア（LLR −0.83）、約2時間12分で820ペア（LLR +0.08）であり、判定境界±2.94へ近づく傾向はなかった。
停止は利用者の判断による。バックグラウンドで起動したrunnerはSIGINTを無視する設定で動いていたため、SIGTERMで終了させた。したがって`summary.json`は起動中のまま終了した状態（強制終了扱い）であり、この実行ディレクトリはスループットの校正には使えない。

## 結論

判定は出ていない。利用者は2026年10月10日に、候補ごとにSTCとLTCを行う方式をやめ、全候補を実装した構成を1回のSTCとLTCで判定する方式へ変更したので、本測定は`--resume`で再開しない。

# move-ordering-all-ltc

## 目的

[手の順序付けの改善](../plans/move-ordering.md)の候補6を除く7候補を積み上げた構成を候補、段階開始版を基準として、長時間条件のGSPRTで採否を判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run \
  --run-dir data/matches/move-ordering-all-ltc --seed 2100200000 \
  --candidate commit:ea0c75a --baseline commit:cd081f4 \
  --concurrency 16 --each time=60000+200 gsprt
```

## エンジン

候補はコミット`ea0c75a`（候補1、2、3、4、5、7、8を1候補1コミットで積み上げた構成。[move-ordering-all-stc](move-ordering-all-stc.md)と同じ）、基準はコミット`cd081f4`（コードは段階開始版のmaster `b784fe5`と同一）である。規則は`engine-default`（`L0,P0,R1,E0`）、`USI_Hash`は既定の256 MB、候補が加えた係数`CaptureHistoryDecay`、`HistoryBonusScale`、`HistoryMalusScale`は既定値（23、100、100）である。

## 環境

2026年10月10日、Intel Core Ultra 7 265KF（物理20コア、論理20コア）で測定した。候補と基準のワーカー数は1、同時対局数は16（lishogi Botのminaseが2プロセス同居するため明示した）。

## 結果

| 項目 | 値 |
|---|---:|
| ペア数 | 584（有効566、破棄18） |
| ペンタノミアル度数 | [91, 13, 302, 23, 137] |
| 候補の得点率 | 54.5%（Elo換算で約+31） |
| LLR | +2.95 |
| 判定 | `H1` |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、`time_forfeits` 0、拒否着手0 |
| 経過時間 | 16,182秒（約4時間30分） |

破棄18ペアは手数上限への到達によるものであり、統計には含めない。LLRは66ペアで−0.11、199ペアで+0.49、395ペアで+2.03と推移し、566ペアで境界を越えた。

## 結論

`H1`であり異常は0件なので、7候補を積み上げた構成を採用する。各候補の寄与は本測定からは分からない。

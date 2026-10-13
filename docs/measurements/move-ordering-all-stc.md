# move-ordering-all-stc

## 目的

[手の順序付けの改善](../plans/archived/move-ordering.md)の候補6を除く7候補を積み上げた構成を候補、段階開始版を基準として、短時間条件のGSPRTで候補をLTCへ進めるかを判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run \
  --run-dir data/matches/move-ordering-all-stc --seed 2100100000 \
  --candidate commit:ea0c75a --baseline commit:cd081f4 \
  --concurrency 16 --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`ea0c75a`（候補1、2、3、4、5、7、8を1候補1コミットで積み上げた構成。各段の`bench`は[move-ordering-stack-bench](move-ordering-stack-bench.md)）、基準はコミット`cd081f4`（フェーズ1の測定記録を加えただけの文書コミットであり、コードは段階開始版のmaster `b784fe5`と同一）である。規則は`engine-default`（`L0,P0,R1,E0`）、`USI_Hash`は既定の256 MB、候補が加えた係数`CaptureHistoryDecay`、`HistoryBonusScale`、`HistoryMalusScale`は既定値（23、100、100）である。

## 環境

2026年10月10日、Intel Core Ultra 7 265KF（物理20コア、論理20コア）で測定した。候補と基準のワーカー数は1、同時対局数は16（lishogi Botのminaseが2プロセス同居するため明示した）。

## 結果

| 項目 | 値 |
|---|---:|
| ペア数 | 306（有効298、破棄8） |
| ペンタノミアル度数 | [41, 9, 147, 7, 94] |
| 候補の得点率 | 58.7% |
| LLR | +2.98 |
| 判定 | `H1` |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、`time_forfeits` 0、拒否着手0 |
| 経過時間 | 2,780秒（約46分） |

破棄8ペアは手数上限への到達によるものであり、統計には含めない。先読みは対局条件に含めていない。

## 結論

`H1`であり異常は0件なので、同じ構成をLTC（[move-ordering-all-ltc](move-ordering-all-ltc.md)）へ進める。

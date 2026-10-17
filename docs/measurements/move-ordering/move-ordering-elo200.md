# move-ordering-elo200

## 目的

[手の順序付けの改善](../../plans/archived/move-ordering.md)のフェーズ10として、採用した7候補を積み上げた最終構成の段階開始版に対する強さを、固定200ペアのEloと95%信頼区間で進捗指標として記録する。採否には使わない。

## コマンドライン

```console
cargo run --release --bin minase -- match run \
  --run-dir data/matches/move-ordering-elo200 --seed 2100300000 \
  --candidate commit:ea0c75a --baseline commit:cd081f4 \
  --concurrency 16 --each time=10000+100 elo --pairs 200
```

時間制御は、段階9以降の段階開始版との進捗指標（[strength-stage9-elo200](../strength-stage9/strength-stage9-elo200.md)）と同じSTCの`time=10000+100`である。

## エンジン

候補はコミット`ea0c75a`（最終構成）、基準はコミット`cd081f4`（コードは段階開始版のmaster `b784fe5`と同一）。規則は`engine-default`（`L0,P0,R1,E0`）、`USI_Hash`は既定の256 MB。

## 環境

2026年10月10日、Intel Core Ultra 7 265KF（物理20コア、論理20コア）。候補と基準のワーカー数は1、同時対局数は16（lishogi Botのminaseが2プロセス同居）。

## 結果

| 項目 | 値 |
|---|---:|
| ペア数 | 200（有効197、破棄3） |
| ペンタノミアル度数 | [34, 6, 102, 6, 49] |
| Elo | +26.5（95%信頼区間 −5.4〜+58.9） |
| エンジン異常 | 不正着手0、クラッシュ0、応答タイムアウト0、`time_forfeits` 0、拒否着手0 |
| 経過時間 | 1,968秒（約33分） |

破棄3ペアは手数上限への到達によるもの。

## 結論

最終構成は段階開始版に対して+26.5 Elo（95%信頼区間 −5.4〜+58.9）であり、[LTC](move-ordering-all-ltc.md)の得点率54.5%（約+31 Elo）と整合する。

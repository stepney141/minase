# 段階Aの長い条件の採否測定

## 目的

[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)の段階A（秒読み期の締切と途中結果の採用）を着手前のエンジンと長い条件（持ち時間と秒読みの比150）で比べ、GSPRTで採否を判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-long-c10 --seed 2031000000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=45000+0,byoyomi=300 --concurrency 10 gsprt
```

## エンジン

候補はea0133d（段階A）、基準は33e7269（master）である。規則セットは`engine-default`、両者とも`Threads=1`、`USI_Hash`は256 MB、`ByoyomiMargin`は既定の30 msである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は10である。
同じ測定機ではlishogiのBot（1局あたり`Threads=4`、最大2局同時）が動いており、測定の前半には別の作業の学習（2プロセス）も動いていた。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 164（うち10は手数上限4,096手で破棄） |
| 有効ペア数 | 154 |
| ペンタノミアル度数 | [11, 6, 67, 7, 63] |
| LLR | +2.951 |
| 判定 | `H1` |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 6,047秒（`summary.json`の`active_wall_time_ns`） |

## 結論

段階Aは長い条件でも`H1`となり、異常は0件だった。[短い条件](byoyomi-a-short-c10.md)と合わせて両条件で`H1`なので、段階Aを採用する。

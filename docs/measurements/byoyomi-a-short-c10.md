# 段階Aの短い条件の採否測定

## 目的

[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)の段階A（秒読み期の締切と途中結果の採用）を着手前のエンジンと短い条件（持ち時間と秒読みの比20）で比べ、GSPRTで採否を判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-short-c10 --seed 2030000000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=6000+0,byoyomi=300 --concurrency 10 gsprt
```

## エンジン

候補はea0133d（段階A）、基準は33e7269（master）である。規則セットは`engine-default`、両者とも`Threads=1`、`USI_Hash`は256 MB、`ByoyomiMargin`は既定の30 msである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は10である。
同じ測定機では、lishogiのBot（1局あたり`Threads=4`、先読みあり、最大2局同時）と別の作業の学習（2プロセス）が動いていた。
同時16対局で行った[最初の測定](byoyomi-a-short.md)が過負荷による時間切れで無効になったので、これらの負荷を見込んで同時対局数を10に下げ、新しいシードで測り直した。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 138（うち9は手数上限4,096手で破棄） |
| 有効ペア数 | 129 |
| ペンタノミアル度数 | [7, 0, 51, 7, 64] |
| LLR | +2.958 |
| 判定 | `H1` |
| 異常件数 | 不正着手0、クラッシュ0、応答タイムアウト0、時間切れ0、拒否着手0 |
| 経過時間 | 3,893秒（`summary.json`の`active_wall_time_ns`） |

## 結論

段階Aは短い条件で`H1`となり、異常は0件だった。採用の判定には、長い条件の測定も`H1`であることを要する。

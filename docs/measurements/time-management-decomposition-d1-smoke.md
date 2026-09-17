# time-management-decomposition-d1-smoke

## 目的

[時間管理の変更の分解測定](../plans/time-management-decomposition.md)の「フェーズ1」が定める候補D1の煙試験である。
秒読みつきと加算つきの各10ペアで時間切れ0件を確かめ、あわせて残り時間の推移を記録する。

## コマンドライン

```console
target/release/match_runner --run-dir data/matches/time-management-decomposition-d1-smoke-byoyomi --seed 20260916 \
  --candidate commit:2f72a52 --baseline commit:2f72a52 \
  --each time=9000+0,byoyomi=300 --concurrency 16 elo --pairs 10
target/release/match_runner --run-dir data/matches/time-management-decomposition-d1-smoke-inc --seed 20260918 \
  --candidate commit:2f72a52 --baseline commit:2f72a52 \
  --each time=10000+100 --concurrency 16 elo --pairs 10
```

集計は`scripts/clock_profile.py <実行ディレクトリ> --budget target`である。

## エンジン

候補と基準はともにD1のコミット2f72a52（ブランチ`tm-decomp-d1`。最終候補3087d1dから第3段階のコミットを取り消したもので、第0段階、第1段階、置換表の2修正を含む）、規則セットは`L0,P0,R1,E0`、`Threads`は1、`USI_Hash`は256 MBである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア）、同時対局数は16を指定したが10ペアなので実際の同時対局は10である。
他の測定は走っていなかった。

## 結果

| 実行 | ペンタノミアル | 時間切れ | hardの超過の最大値（候補／基準） | 秒読みの利用率の中央値 | 経過時間 |
|---|---|---:|---:|---:|---:|
| `time=9000+0,byoyomi=300` | [2, 1, 5, 1, 1] | 0 | 12.4 ms／13.3 ms | 0.810 | 253 s |
| `time=10000+100` | [2, 2, 3, 0, 3] | 0 | 13.9 ms／14.0 ms | 該当なし | 129 s |

不正着手、クラッシュ、応答タイムアウト、拒否着手はいずれも0件、完了深さ0の手は0であった。
秒読みつきでは持ち時間が40件すべて尽き、加算つきでは40件とも尽きなかった。

加算つきの20局について、D1の着手前の残り時間の中央値と平均到達深さを手数帯ごとに示す（両側ともD1）。

| 手数帯（ply） | 手数 | 平均到達深さ | 思考時間の中央値 | 残り時間の中央値 |
|---|---:|---:|---:|---:|
| 0〜50 | 802 | 5.90 | 81 ms | 10.3 s |
| 50〜100 | 1,000 | 5.96 | 132 ms | 10.2 s |
| 100〜200 | 2,000 | 5.99 | 132 ms | 9.8 s |
| 200〜300 | 1,861 | 6.87 | 152 ms | 8.6 s |
| 300〜400 | 1,432 | 7.89 | 142 ms | 7.2 s |
| 400〜600 | 914 | 9.34 | 122 ms | 6.1 s |

D1は1手の消費が公平な分担にとどまるため持ち時間を使い切らず、200〜300手の残り時間の中央値は8.6秒である（不採用の最終候補は0.4秒、元の基準は4.8秒。[time-management-efficiency-stc](time-management-efficiency-stc.md)の事後分析）。
到達深さの絶対値は同時対局10の負荷でのものであり、同時対局4で測った過去の値とは比較しない。

## 結論

D1は両方の煙試験で時間切れ0件、hardの超過は最大14 msであり、STCへ進める。

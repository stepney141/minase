# time-management-efficiency-probe

## 目的

現行の時間管理が、issue #7の条件（持ち時間5分と秒読み10秒）とその周辺の時間制御で、初形の1手にどれだけ使うかを測り、[持ち時間の効率的な使用](../plans/time-management-efficiency.md)の原因の特定の根拠にする。

## コマンドライン

初形から`go`を1回送り、`bestmove`までの壁時計時間と`info`行を記録した。
起動は`target/release/minase --protocol usi --rules lishogi`、`setoption name USI_Hash value 256`、`Threads`は表のとおりである。
`go`の引数は各行に示す。`btime 288100`は、Lishogi-Botが持ち時間300,000 msから`move_overhead` 1,900 msと秒読み10,000 msを引いて送る値である。

## エンジン

コミット008f659の作業ツリー（変更なし）から`cargo build --release`で作ったバイナリであり、ソースとの一致は`cargo build`が再コンパイルを要しないことで確認した。
規則セットは`lishogi`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、`USI_Hash`は256 MBである。
同じ機で同時対局数16のLTC（合法手生成高速化第2期）が並行しており、負荷平均は16〜17であった。
到達深さはこの負荷の影響を受けるが、停止の判断は壁時計で行われるので、予算に対する超過の機構は負荷に依存しない。

## 結果

softとhardは`clock_budget`の式（`ply = 0`、整数演算）で再計算した値である。

| `go`の引数 | `Threads` | soft | hard | 実測（ms） | 最後の2反復の完了時刻 | 停止理由 |
|---|---:|---:|---:|---:|---|---|
| `btime 300000 wtime 300000 byoyomi 10000` | 1 | 9,333 | 37,332 | 13,373 | 深さ11が6,848 ms、深さ12が13,359 ms | soft |
| `btime 288100 wtime 288100 byoyomi 10000` | 1 | 9,280 | 37,120 | 13,391／13,635／13,527 | 深さ11が6.9〜7.1秒、深さ12が13.4〜13.6秒 | soft |
| 同上 | 4 | 9,280 | 37,120 | 12,617 | 深さ11が3,688 ms、深さ12が12,599 ms | soft |
| `btime 0 wtime 0 byoyomi 10000` | 1 | 8,000 | 8,000 | 4,077 | 深さ9が1,667 ms、深さ10が4,066 ms | soft |
| `btime 0 wtime 0 byoyomi 30000` | 1 | 24,000 | 24,000 | 13,516 | 深さ11が6,997 ms、深さ12が13,501 ms | soft |
| `btime 588100 wtime 588100 byoyomi 30000` | 1 | 26,613 | 106,452 | 45,944 | 深さ12が13,423 ms、深さ13が45,926 ms | soft |
| `btime 295000 wtime 295000 binc 5000 winc 5000` | 1 | 4,811 | 19,244 | 6,872 | 深さ10が4,026 ms、深さ11が6,861 ms | soft |

codexの独立調査（同日、同じバイナリ、`--rules engine-default`、`btime 300000 wtime 300000 byoyomi 10000`）は初形で11,708 ms（深さ12でsoft停止）、初形から4手進めた局面で7,355 ms（深さ10〜13の最善手が一致し安定時の早期終了）、残り0・秒読み10秒で4,056 msと3,302 msを記録した。

## 結論

持ち時間局面では、softに達する前に始めた最後の反復が完了まで走るため、実測はsoftの1.4〜1.7倍になり、hardは1件も働かない。
秒読み局面では、softとhardが一致するため予測の規則が経過時間0.4·hard以降の反復開始を禁じ、実測は秒読みの0.41〜0.45にとどまる。

## 参考

先行する削除済みブランチ`time-management-byoyomi`の成果物（設計書、診断記録`time-management-byoyomi-diag.md`と`time-management-byoyomi-k1.md`、教訓`simulate-acceptance-criteria-on-the-candidate.md`、診断スクリプト`clock_budget_stats.py`と`byoyomi_game_profile.py`）は、コミット9f0172fのオブジェクトとして`git show 9f0172f:<パス>`で参照できる。

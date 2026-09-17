# time-management-opening-coefficient-diag

## 目的

[秒読みの予算に序盤の係数を加える](../plans/time-management-opening-coefficient.md)の「フェーズ2」が定める時計の診断である。
issueの条件（持ち時間5分、秒読み10秒）の先手60手自己対局で初手の実測思考時間がhard 8,532 msに100 msを加えた値以下であること、全手でhardの超過が30 ms未満であること、および`time=9000+0,byoyomi=300`の10ペア煙試験で時間切れ0件であることを確かめる。

## コマンドライン

```console
python3 scripts/byoyomi_game_profile.py --engine <ブランチtime-management-opening-coefficientのtarget/release/minase> \
  --rules L0,P0,R1,E0 --hash 256 --time 300000+0 --byoyomi 10000 --black-moves 60 --budget byoyomi-opening \
  --record data/experiments/time-management-efficiency/oc-5m10s.record.json \
  --json data/experiments/time-management-efficiency/oc-5m10s.summary.json
target/release/match_runner --run-dir data/matches/time-management-opening-coefficient-smoke --seed 20260916 \
  --candidate commit:2945d7e --baseline commit:2945d7e \
  --each time=9000+0,byoyomi=300 --concurrency 16 elo --pairs 10
```

集計は`scripts/clock_profile.py <実行ディレクトリ> --budget byoyomi-opening`と上記スクリプトの出力である。

## エンジン

コミット2945d7e（ブランチ`time-management-opening-coefficient`。分解測定の共通基準B′に秒読みの項の序盤の係数を加えたもの）、規則セットは`L0,P0,R1,E0`、`Threads`は1、`USI_Hash`は256 MBである。
固定深さのbench（深さ5、15局面）の総ノード数は1,268,607でB′と一致し、秒読み0の代表入力の予算値は単体テストで現行と一致する。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア）。
2つの実行は同時に走らせ、他の測定はなかった。

## 結果

### 5分＋10秒の先手60手自己対局

先手60手（119手）まで対局し、時間切れは0件、完了深さ0の手は0であった。
初手（ply 0）の実測思考時間は4,010 ms（soft 2,133 ms、hard 8,532 ms、深さ10をsoft停止）であり、許容値8,632 ms以下である。
現行構成の同じ診断（[time-management-efficiency-diag](time-management-efficiency-diag.md)）の初手11,891 msから約3分の1になった。
最初の6手（ply 0〜10）の実測は1.6〜4.9秒、19手目（ply 36）以降はsoftが現行と同じ9.3秒前後に戻る。
持ち時間は先手が44手目、後手が46手目に尽きた（現行は37手目と36手目）。

| 側 | 局面 | 手数 | 平均到達深さ | softとの比の中央値 | softとの比の90パーセンタイル | hard停止の割合 | hardの超過の最大値 | 秒読みの利用率の中央値 | 完了深さ0 |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 先手 | 持ち時間 | 44 | 11.98 | 0.776 | 1.663 | 2.3% | 4.2 ms | 該当なし | 0 |
| 先手 | 秒読み | 16 | 10.25 | 0.721 | 1.001 | 12.5% | 5.7 ms | 0.577 | 0 |
| 後手 | 持ち時間 | 46 | 12.76 | 0.633 | 1.843 | 2.2% | 9.0 ms | 該当なし | 0 |
| 後手 | 秒読み | 13 | 10.85 | 0.539 | 1.001 | 15.4% | 6.3 ms | 0.431 | 0 |

持ち時間局面の最長は先手21,352 ms、後手18,331 msであり、現行の機構どおりsoftを超えて始めた反復が完了まで走った手である。
秒読み局面の利用率は現行と同じ範囲であり、本書の変更の対象外である。

### 10ペア煙試験（`time=9000+0,byoyomi=300`）

ペンタノミアル度数は[3, 0, 4, 2, 0]、破棄ペアは1（手数上限4,096手）、時間切れ、不正着手、クラッシュ、応答タイムアウト、拒否着手はすべて0件、経過時間は744秒、20局の手数の中央値は470手であった。
hardの超過の最大値は候補11.4 ms、基準12.1 ms（持ち時間局面）、14.3 msと13.0 ms（秒読み局面）であり、秒読みの利用率の中央値は0.475で現行と同じである。

## 結論

判定(a)（秒読みなしの同一性）と判定(b)（初手8,632 ms以下、hardの超過30 ms未満、時間切れ0件）をすべて満たし、採用する。
issueの条件の初手は11.9秒から4.0秒になり、最初の数手の長考は秒読み相当ではなく持ち時間の分担と現行の超過の範囲に収まった。

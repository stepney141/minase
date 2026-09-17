# time-management-efficiency-stage2-bench

## 目的

[持ち時間の効率的な使用](../plans/time-management-efficiency.md)の第2段階（深さの抑制）について、「棄却した再構成の各段階」の「深さの抑制（第2段階）」が定める事前判定を行う。
抑制した反復（`search_depth`を`root_depth`より浅く読み直す反復）の所要時間を、直前の全深さの反復の所要時間で割った比の中央値が0.05未満なら、読み直しが置換表の照会だけで終わる空費と判定して段階を見送る。

## コマンドライン

第1段階のコミット551ea76の上に第2段階を実装した作業ツリー（コミットせず、差分は`data/experiments/time-management-efficiency/stage2-depth-suppression.patch`に保存）のリリースバイナリで、次を実行した。

```console
python3 data/experiments/time-management-efficiency/stage2_bench.py target/release/minase \
  data/experiments/stage6-diag-tools/positions.json 4 \
  data/experiments/time-management-efficiency/stage2-bench.json
```

手順は次のとおりである。
各局面で新しいプロセスを起動し、`go depth 8`で深さごとの完了時刻T_dを記録する（T_8が100 ms未満の局面は、100 ms以上になる最小の深さdを用いる）。
同じ局面で新しいプロセスを起動し直して置換表を空にし、`go movetime ⌊1.2·T_d⌋`を与える。
深さdの完了時に経過時間がsoftの半分を超えるので抑制が始まり、`info string suppressed`で示される最初の抑制した反復の所要時間を、直前の全深さの反復の所要時間で割った比を求める。

## エンジン

第2段階を実装した作業ツリーのバイナリ（SHA-256は`stage2-bench.json`の`binary_sha256`）、規則セット`L0,P0,R1,E0`、`USI_Hash`は256 MB、`Threads`は1である。
局面は[strength-stage6-iteration-diag](strength-stage6-iteration-diag.md)と同じ240局面である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア）、並列度4であり、同じ機で別の測定（同時対局数16）が並行していた。
比は同じ局面内の反復どうしの時間比なので、負荷の影響は分子と分母に同じ向きに掛かる。

## 結果

240局面のうち、校正の深さは8が224局面、9〜12が16局面であった。
抑制した反復を観測できた局面は128、観測できなかった局面は112である。
観測できなかった局面は、直近4反復の最善手の一致による早期終了が抑制の開始より先にsoft停止を選んだものであり、第2段階までは早期終了を維持する設計どおりである。

観測できた128局面では、抑制した反復がhardで中断された件数は0であった。
直前の全深さの反復の所要時間は中央値251 ms（最小39 ms）であるのに対し、抑制した反復の所要時間は最大1 ms（0 msが124局面、1 msが4局面）であり、比の中央値、第1四分位、第3四分位はいずれも0.0であった。

## 結論

比の中央値0.0は判定値0.05を下回り、抑制した反復は直前に読んだ深さの置換表の照会だけで終わって時間を使っていない。
設計書の判定規則に従い、第2段階は見送り、実装の差分はコミットせずにパッチとして保存した。
予算の後半は、第1段階のとおり次の深さの部分的な探索（hard中断と途中結果の採用）に使う。

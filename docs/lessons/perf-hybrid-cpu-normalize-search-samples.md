# ハイブリッドCPUのperfは性能コアの表を計測区間の処理だけで正規化する

## 症状

第2期高速化の最終プロファイルで、`perf report`の先頭の表が`libc`の未解決アドレス97.6%で埋まり、探索関数が見えなかった。
2番目の表にも`libc`が11.7%あり、そのまま読むと探索の自己時間が1割過小になった。

## 原因

測定機（Intel Core Ultra 7 265KF）は性能コアと効率コアのハイブリッド構成であり、`perf record`は`cpu_atom`と`cpu_core`の2事象を記録して`perf report`は事象ごとに表を分ける。
計測区間外の置換表クリア（`memset`）はOSが効率コアへ移して実行することが多く、効率コアの表をほぼ独占する一方、性能コアの表にも一部が混ざる。

## 以後の規則

ハイブリッドCPUでは`perf report --sort dso,symbol`で性能コアの表を使い、`-g caller`で`bench::main`から直接呼ばれた`libc`の標本（置換表クリア）を除いてから探索関数の自己時間を100%に正規化する。
`perf script -F tid`でスレッドが1本であることを先に確かめ、表の分割をスレッドの違いと誤読しない。

## 出典

[第2期高速化の最終の残存負荷](../measurements/movegen-speedup-2-final-profile-depth5.md)。

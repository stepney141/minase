# 対局と並行する処理はメモリの上限を課したサービスで動かす

## 症状

2026年11月6日から7日にかけて、[標準レシピv1の測定](../plans/nnue-standard-recipe.md)で、同時対局数16のSTCと並行してNNUEの学習を作業機（メモリ30 GB）で動かした。
STCの間、使えるメモリは約6 GBしか残っていなかった。
対局の32個のエンジンが`USI_Hash` 256 MBと探索の表で各約0.5 GBを使い、lishogiのBotの2個のエンジンが各約2.5 GBを使っていたからである。
教師63本の学習は、全局面をメモリに置く既定の`--buckets 1`では約20 GBを要する見込みだった。
メモリ上限5 GB、スワップなしのサービスで`--buckets 16`にすると、学習中のサービスのメモリの最大は約3.2 GBで収まったが、続く`diag nnue-removal`は上限を超えてOOMで止められた。
対局の時間切れは0件だった。

## 原因

対局のエンジンは置換表と探索の表を対局ごとに持つので、同時対局数とハッシュの大きさに比例して常駐メモリが増える。
並行する処理がメモリの空きを超えると、カーネルは使用中のエンジンのページもスワップへ追い出すので、持ち時間のある対局では時間切れや探索の遅れとして現れるおそれがある（今回は上限を課したので起きていない）。
CPUのスレッド数を空きコアに抑えても、メモリの使用量は抑えられない。
学習器のbucket数で抑えられるのは学習の工程だけで、同じ入力を読む診断のメモリは別に要る。

## 以後の規則

持ち時間のある対局と並行して学習、診断、または検査を動かすときは、`free`で対局中の空きメモリを確かめ、処理を`systemd-run --user -p MemoryMax=<空きより小さい値> -p MemorySwapMax=0`のサービスで動かす。
上限を超えた処理はOOMで止まるだけなので、対局は守られる。
学習は`--buckets`で上限の内側に収め、上限の内側に収まらない診断は、対局の終わった後に上限を上げて実行する。

## 出典

- [measurements/nnue-standard-recipe/nnue-standard-recipe-window-stc.md](../measurements/nnue-standard-recipe/nnue-standard-recipe-window-stc.md)
- [lessons/check-running-measurements-before-cpu-load.md](check-running-measurements-before-cpu-load.md)

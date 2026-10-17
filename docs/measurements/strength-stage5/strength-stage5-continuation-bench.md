# strength-stage5-continuation-bench

## 目的

棋力向上段階5のcontinuation history（2手前の自分の手の駒種と到達升から応手の駒種と到達升を引く`[29][144][29][144]`の表）について、採否測定へ進む前に、benchの静かな手の順序付けで表の値が0でない手の割合（発動率）が5%以上かを判定する。

## コマンドライン

counter move historyの実装（[strength-stage5-cmh-bench](strength-stage5-cmh-bench.md)）にcontinuation historyを加えた作業ツリーで、順序付け時に評価した静かな手の総数と表の値が0でない手の数を数える一時的な計測を入れ、次のコマンドを実行した。

```sh
cargo run --release --bin bench -- --depth 5
```

## エンジン

基点はコミット6084078（piece-to historyの候補）であり、counter move historyとcontinuation historyの実装は見送りのためコミットしていない。
対局ではないため、規則セットは該当しない。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）であり、benchは単一スレッドで実行した。
同じ機械で別の時間制御測定が並行していたため、NPSは記録しない。
`USI_Hash`と同時対局数は該当しない。

## 結果

深さ5の探索で順序付け時に評価した静かな手は1,390,751手であり、continuation historyの値が0でない手は8,569手（0.62%）、counter move historyとcontinuation historyのいずれかが0でない手は16,125手（1.16%）であった。
総ノード数は1,761,806であった。
破棄ペア数、異常件数、`time_forfeits`、対局の総経過時間は該当しない。

## 結論

発動率が基準の5%を大きく下回るため、continuation historyは発動しない改良として見送る。
2表を合わせても1.16%であり、根探索の開始時に初期化する1,744万要素の表は1回の探索では埋まらない。

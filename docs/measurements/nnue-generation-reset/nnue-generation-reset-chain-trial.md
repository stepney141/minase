# 連続生成スクリプトの通し試験

## 目的

[世代反復の設計書](../../plans/nnue-generation-reset.md)のフェーズ1の完了条件として、連続生成スクリプト`scripts/vast/generation_chain.py`を、vast.aiで借りた安い機械と少ない局数で動かし、世代の連鎖、失敗時の停止、借り直し後の再開、待ちの上限での自己停止、および口座のAPIが使えない間の動作を確かめる。

## コマンドライン

試験は、本番の参照を汚さないよう、リポジトリの使い捨てのcloneを`--worktree`に指定して行った。
サービスは`systemd-run --user`（nice値19、メモリ上限8 GB）で起動した。
試験用に変えた引数は次のとおりであり、その他の引数は本番と同じ意味を持つ。

```console
python3 scripts/vast/generation_chain.py run --run-dir <試験の場所>/<run名> --worktree <使い捨てのclone> \
  --match-runner <decision: H1 を表示するだけの偽物> \
  --gen0-commit 6f874692fb91544c94394ca0e8df25299ef2e02d --first-generation 1 --last-generation <1または2> \
  --positions-per-generation <15,000または600,000> --books-estimate <2または4> \
  --games <40または500> --nodes 500 --random-moves 0 --max-ply 4000 --hash-mb 16 \
  --seed-base 9500000000 --seed-generation-step 10000000 --seed-book-step 100000 --max-books 20 \
  --calibration-books 1 --lambda 0.75 --feature-set P --width 256 --head pairwise-l16-l32 --batch 1024 \
  --epochs 2 --patience 5 --train-seed 1 --device cuda --validation-interval 1000 --lr-schedule cosine:3:0.1 --train-threads 4 \
  --removal-sample 100 --removal-seed 1 --bench-depth 5 \
  --image rust:1.98.0-bookworm --disk 30 --max-price 0.3 --min-cpu-cores 12 --cpu-pattern . --min-duration-days 1 \
  --min-reliability 0.95 --min-inet-up 100 --min-disk 30 --book-minutes 10 --deadline-margin 1.3 \
  --training-wait-hours 0.2 --idle-limit-hours <1または0.05> --poll-seconds 15 --book-stall-minutes 20 --ssh-failure-minutes 5 \
  --offer 57664956
```

口座のAPIの失敗は、`PATH`の先頭に置いた`vastai`のラッパーで模擬した。
ラッパーは、印のファイルがあれば401の文言を出して終了コード1で終わり、なければ本物の`vastai`を実行する。

## エンジン

第0世代のコミットは`6f874692`（手書きの評価関数v0を線形の経路だけで表したMNUE、K=1000、検査和`74afe0e5…`）である。
規則セットは`L0,P0,R1,E0`である。

## 環境

生成機はvast.aiの出品57664956（Xeon E5-2680 v4、28スレッド、ギリシャ、1時間0.108米ドル）である。
cgroupのCPU上限は26.88個分、同時対局数は28だった。
作業機はIntel Core Ultra 7 265KF（20コア、メモリ30 GB、RTX 3060 Ti）である。

## 結果

実行は2026年11月4日に行った。
残高は130.7429米ドルから130.6911米ドルへ減ったので、試験全体の費用は約0.05米ドルである。

| 確かめた条件 | 手順 | 結果 |
|---|---|---|
| 2世代の連鎖 | 2世代を続けて生成した。 | 第1世代の本のヘッダは第0世代のコミットと検査和`74afe0e5…`を持ち、第2世代の本のヘッダは第1世代のコミット`6f4ad619`とネットの検査和`46eb820e…`を持った。Kは第1世代の教師から求めた1656.21が第1世代と第2世代のネットの両方に入った。 |
| 口座のAPIの失敗 | 生成機を作った直後からラッパーでAPIを失敗させた。 | 取得、検査、学習、コミット、および次の世代への移行は進み、第2世代の本の取得の後にコンテナの鍵で生成機が止まった（状態は`exited`、意図は`stopped`）。削除を確かめられないので終了コード1で終わった。印を消して同じコマンドを再実行すると、削除だけを行って「完了」で終わり、インスタンスの一覧が空になった。 |
| 生成コマンドの強制終了 | 本を1本受け取った後に、生成機で生成コマンドを`kill -9`で止めた。 | 作業者が`exit-status`に137を書き、失敗した本のログを`book-1.failed.log`へ移した。スクリプトは生成機を止めて削除し、終了コード1で終わった。 |
| 借り直し後の再開 | 上の状態から`--offer`を付けて再実行した。 | 新しい生成機の`job.json`の`skip_books`に作業機の本0が入り、生成機の本は1、2、および3だけになった。計4本で学習とコミットまで終わった。 |
| 待ちの上限での自己停止 | 待ちの上限を3分にし、本がそろった後でスクリプトを`SIGTERM`で止めた。 | スクリプトは生成機を止めずに終了コード143相当で終わった。最後の本から約3分後に生成機が停止を要求し、状態が`exited`になった。その状態でスクリプトを再実行すると、接続できない旨を表示して止まった。 |

第1世代のSTCの経路は、偽物の実行ファイルで`H1`を返して通した。
本物のハーネスについては、同じコミット同士の2ペアの対局を`--worktree`の中で実行して`decision: pending`を確かめ、`--resume`でも同じ判定の行が出ることを別のcloneで確かめた。
中断、`H0`、および`pending`で第2世代の学習を始めないことは、単体テストで確かめている。

## 結論

連続生成スクリプトは設計書のフェーズ1の完了条件を満たしたので、本番の生成に使う。

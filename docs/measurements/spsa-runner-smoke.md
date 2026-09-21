# `spsa_runner` の煙試験

## 目的

`spsa_runner` が調整用ビルドの実エンジンで対局を進行し、強制終了と再開の後に反復のファイルを欠番なく揃え、開始時の検査が誤った入力を拒否することを確かめる。

## コマンドライン

```console
spsa_runner --run-dir smoke1 --seed 777 \
  --engine commit:175b97d --params params-default.txt \
  --rules engine-default --each time=2000+50 --concurrency 4 \
  --iterations 6 --pairs-per-iteration 2
```

パラメーターファイルは、`spsa_runner params --engine commit:175b97d --rules engine-default` が生成した22係数の既定のファイルである。
最初の起動を50秒後に、`--resume` による2回目の起動を100秒後に `SIGKILL` で強制終了し、3回目の `--resume` を完了まで実行した。
開始時の検査は、係数名 `MinMoves` を `MinMovez` に書き換えたファイル、通常ビルドのminaseを起動コマンドで `--engine` に渡す指定、およびシードだけを778に変えた `--resume` の3通りで確かめた。

## エンジン

θ+とθ−はどちらも、コミット`175b97deece2e75302bdca9a3bf91cedd749121b`の調整用ビルドである。
runnerは同じコミットのビルドである。
規則セットは`engine-default`（L0、P0、R1、E0）である。

## 環境

CPUはIntel Core Ultra 7 265KFであり、物理コア数と論理コア数はいずれも20である。
両エンジンは`Threads=1`、`USI_Hash`は既定の256 MB、同時対局数は4である。

## 結果

最初の起動は反復を1つも完了せずに終了し、2回目の起動は反復1と反復2を保存して終了した。
3回目の起動は残りの4反復を実行し、`iterations/` には反復1から6までのファイルが欠番なく揃った。
再開時の検査（実行条件の一致、適用番号の連続、およびθの連鎖）はいずれも通った。
12ペアのうち有効ペアは11、手数上限への到達による破棄は1であった。
エンジン異常、不正着手、応答タイムアウト、`time_forfeits`、および拒否着手はいずれも0件であった。
強制終了の後にエンジンのプロセスは残らなかった。
開始時の検査は、3通りともエラーで終了し、新しい実行ディレクトリを作らなかった。
エラーの文言はそれぞれ `unknown parameter: MinMovez`、`engine declares no Tune_ options; use a tuning build`、`manifest does not match requested session` であった。
3回目の起動の経過時間は215.5秒であった。

## 結論

`spsa_runner` は実エンジンでの対局、強制終了からの再開、および開始時の検査のいずれも設計どおりに動作した。

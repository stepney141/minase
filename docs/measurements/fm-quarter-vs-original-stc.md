# FM補正1/4と元FMの短時間比較

## 目的

FM補正を1/4に縮めた単一候補と元FMを比較し、補正振幅への介入が棋力を改善するか調べる。
PSTとの採用比較ではない。

## コマンドライン

2026年9月9日にfm-eval worktreeから起動した。
測定名はfm-quarter-vs-original-stc、シードは20690909であり、既存測定の保存済みシードと今回の3,000ペアの範囲は重ならない。

```sh
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/fm-quarter-vs-original-stc \
  --seed 20690909 \
  --candidate commit:3143b082ac15efceb889ba35ce79ed37abe8594d \
  --baseline commit:658a301a6d782f7ca359c7fb8b3eec515ccf45ce \
  --each time=10000+100 \
  --candidate-hash 256 --baseline-hash 256 \
  --rules engine-default --concurrency 19 \
  gsprt --max-pairs 3000
```

実際の起動コマンド、ハーネスのSHA256、監視プロセスの状態はworktreeの`data/fm-quarter/`に保存した。
監視は保存済みペアのforfeitを検出するとプロセス群を停止し、その理由をstatus.jsonへ記録する。
再開時は上の`--run-dir`を同じパスの`--resume`へ変え、コミット、シード、条件、ペア上限を保持する。
異常停止後は原因を調査してから再開する。

## エンジン

候補は`3143b082ac15efceb889ba35ce79ed37abe8594d`、基準は`658a301a6d782f7ca359c7fb8b3eec515ccf45ce`である。
モデルの変更はFM指数9から10と検査和だけであり、PST、駒価値、評価尺度、埋め込み、符号、探索コードを固定した。
テストの重み依存の期待値はPythonの特徴対の直接和から更新した。
規則は両側engine-defaultである。

候補重みのSHA256は`d781bc0466451ccf7b14494916e72c8086dbac6681d9b464a03b02c7d6dab0ba`、元重みは`962e22432f622e526c4f4bfcbad2f053994002afb6a6a4381a9ad25b4a337cb1`である。
[モデル検証記録](fm-quarter-model-validation.json)には98,843局面での整数補正とRust評価の照合を、[参照値](fm-quarter-fixture-reference.json)には独立計算の結果を保存した。
Rustの全618テスト、フォーマット、全ターゲットのClippyが通過した。
エンジンバイナリのSHA256は対局ディレクトリのmanifest.jsonを正とし、起動時に[保存したmanifest](fm-quarter-vs-original-stc.manifest.json)の両バイナリと照合した。

## 環境

CPUはIntel Core Ultra 7 265KF、20物理コア・20論理コアである。
両側1スレッド、USI_Hashは各256 MB、同時19ペア、持ち時間は各10秒と1手あたり0.1秒加算である。
ハーネスはコミットのアーカイブから各エンジンをreleaseビルドする。

## 結果

測定中であり、棋力の判定は未確定である。
有効ペア数、ペンタノミアル度数、対数尤度比、破棄ペア数、異常件数、time_forfeits、累計時間は、終了時にsummary.jsonと保存ペアから記録する。
途中経過から採否を判断しない。

## 結論

未確定である。
元FMに対してH1、または上限pendingで対数尤度比が非負ならPSTとの比較へ進み、それ以外ならこの候補を採用へ進めない。
採用にはPSTとの短時間・長時間の規定測定を別途要する。
設計判断は[設計書](../plans/fm-quarter.md)を参照する。

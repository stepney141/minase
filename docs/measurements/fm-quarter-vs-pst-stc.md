# 補正1/4のFMとPSTの短時間比較

## 目的

元FMへの改善を確認した補正1/4候補を採用済みPSTと比較し、長時間の採用判定へ進めるかを選別する。

## コマンドライン

2026年9月9日にfm-eval worktreeから起動した。
候補の縮小率と重みは元FMとの比較から変更していない。

```sh
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/fm-quarter-vs-pst-stc \
  --seed 20790909 \
  --candidate commit:3143b082ac15efceb889ba35ce79ed37abe8594d \
  --baseline commit:b0153cf27df655a32cb9a255fd380b21bba002b5 \
  --each time=10000+100 \
  --candidate-hash 256 --baseline-hash 256 \
  --rules engine-default --concurrency 19 \
  gsprt --max-pairs 3000
```

シードは元FMとの比較の20690909から100,000離し、最大3,000ペアの対局列を分離した。
起動前に元リポジトリの保存済みmanifestを確認し、今回のシードと重なる既存測定がないことを確認した。
長時間測定に進む場合のシードは20890909に固定した。
短時間と長時間、および元FMとの測定結果を合算しない。

実行条件、ログ、監視状態はworktreeの`data/fm-quarter-vs-pst-stc/`に保存する。
保存済みペアにforfeitがあれば監視プロセスが停止し、理由をstatus.jsonへ記録する。
中断時は保存結果を保持し、再開時には`--run-dir`を同じパスの`--resume`へ変え、残りの条件を維持する。
異常停止後は原因を調査してから再開する。

## エンジン

候補は`3143b082ac15efceb889ba35ce79ed37abe8594d`、PST基準は`b0153cf27df655a32cb9a255fd380b21bba002b5`である。
PST基準は既存FM採否測定と同じである。
PST重み、探索用駒価値、評価尺度、規則は両モデルで一致する。
FMの推論と差分累算の処理費用を含むエンジン全体を、同じ時間条件で比較する。

候補重みのSHA256は`d781bc0466451ccf7b14494916e72c8086dbac6681d9b464a03b02c7d6dab0ba`、基準重みは`b172a1b3e1c620bbdb1b2f22d0c5ebc9fa11a891703d88c59c710301173feb5a`である。
候補の検証は[モデル検証記録](fm-quarter-model-validation.json)と[独立参照値](fm-quarter-fixture-reference.json)にある。
前段でRustの618テスト、フォーマット、全ターゲットのClippyが通過しており、今回はコードと重みを変更していない。
両エンジンのバイナリSHA256と資源条件は対局ディレクトリのmanifest.jsonを正とし、起動時に[保存したmanifest](fm-quarter-vs-pst-stc.manifest.json)と実バイナリを照合した。

## 環境

Intel Core Ultra 7 265KF、20物理コア・20論理コアで実行する。
両側1スレッド、USI_Hash各256 MB、同時19ペア、持ち時間は各10秒と1手あたり0.1秒加算である。
規則は両側engine-defaultである。

## 結果

測定中であり、判定は未確定である。
終了時に判定位置までの有効ペア数、ペンタノミアル度数、対数尤度比、破棄ペア数、異常件数、time_forfeits、累計実行時間を保存する。
判定位置より後の並列実行分を採否統計へ加えない。

## 結論

未確定である。
H1、または3,000ペア上限のpendingで対数尤度比が非負なら長時間測定へ進み、それ以外は不採用とする。
採用は長時間測定のH1かつ異常0件を条件とする。
詳細は[採否設計](../plans/fm-quarter-adoption.md)を参照する。

# 補正1/4のFMとPSTの長時間比較

## 目的

PSTとの短時間比較を通過した補正1/4のFM候補について、長時間条件で採否を判定する。

## コマンドライン

2026年9月9日に、fm-eval worktreeから事前に固定した条件で起動した。
候補と基準は短時間測定から変更していない。

```sh
target/release/match_runner \
  --run-dir /home/stepney141/board-games/minase/data/matches/fm-quarter-vs-pst-ltc \
  --seed 20890909 \
  --candidate commit:3143b082ac15efceb889ba35ce79ed37abe8594d \
  --baseline commit:b0153cf27df655a32cb9a255fd380b21bba002b5 \
  --each time=60000+200 \
  --candidate-hash 256 --baseline-hash 256 \
  --rules engine-default --concurrency 19 \
  gsprt --max-pairs 100000
```

シードは短時間測定の20790909から100,000離した20890909である。
既存測定のmanifestとの照合結果と全コマンドは、worktreeの`data/fm-quarter-vs-pst-ltc/execution.json`に保存した。
短時間測定の得点や尤度比は合算しない。

監視プロセスは保存されたペアにforfeitを検出すると停止し、理由を同ディレクトリのstatus.jsonへ記録する。
異常停止後は原因を調査してから再開する。
通常の上限到達でpendingなら、同じ条件とシードで`--run-dir`を同じパスの`--resume`へ変え、`--max-pairs`だけを増やす。

## エンジン

候補は`3143b082ac15efceb889ba35ce79ed37abe8594d`、基準は`b0153cf27df655a32cb9a255fd380b21bba002b5`である。
短時間比較と同一バイナリを用い、PST重み、探索用駒価値、評価尺度、規則を揃えたままFM推論の計算負荷を含めて比較する。
候補重みのSHA256は`d781bc0466451ccf7b14494916e72c8086dbac6681d9b464a03b02c7d6dab0ba`、基準重みは`b172a1b3e1c620bbdb1b2f22d0c5ebc9fa11a891703d88c59c710301173feb5a`である。
バイナリSHA256と資源条件は対局ディレクトリのmanifest.jsonを正とし、起動時に[保存したmanifest](fm-quarter-vs-pst-ltc.manifest.json)を短時間測定および実バイナリと照合した。

コードと重みは変更していない。
候補はモデル変換時に98,843局面で照合済みで、Rustの618テスト、フォーマット、全ターゲットのClippyが通過している。
[短時間測定](fm-quarter-vs-pst-stc.md)は有効567ペアでH1に達し、異常0件で長時間測定への条件を満たした。

## 環境

Intel Core Ultra 7 265KF、20物理コア・20論理コアで実行する。
両側1スレッド、USI_Hash各256 MB、同時19ペア、持ち時間は各60秒と1手あたり0.2秒加算である。
規則は両側engine-defaultである。

## 結果

測定中であり、採否は未確定である。
終了時には判定位置までの有効ペア数、ペンタノミアル度数、対数尤度比、破棄ペア数、異常件数、time_forfeits、累計実行時間を保存する。
判定位置より後に保存された並列実行分を採否統計に含めない。

## 結論

未確定である。
H1かつ異常0件なら採用し、H0なら不採用とする。
上限pendingを裁量で採用せず、同じ測定を延長する。
詳細は[採否設計](../plans/fm-quarter-adoption.md)を参照する。

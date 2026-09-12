# 段階7の鏡映重み共有モデルの短時間測定

## 目的

[棋力向上段階7](../plans/strength-stage7.md)の鏡映重み共有候補を段階開始版と短時間条件（STC）で比較し、長時間の最終測定へ進めるかを判定する。

## コマンドライン

2026年9月12日に、固定した実行ファイルで次の測定を開始した。
実行プロセスの引数、起動ログ、保存されたmanifestの測定条件は一致している。

```console
data/strength-stage7/match_runner \
  --run-dir data/matches/strength-stage7-mirror-stc --seed 20900903 \
  --candidate commit:960c56bcbb6088d0f0466d670ceea36dffeda5b7 \
  --baseline commit:d0d3c52024c5ab4d74cc82588640a55953426aad \
  --each time=10000+100 gsprt --max-pairs 3000
```

両者の持ち時間は10秒、1手ごとの加算は0.1秒で、秒読みは使わない。
同じ開始局面から先後を入れ替えた2局を1ペアとし、[測定手順](../sprt.md)の一般化逐次確率比検定で判定する。
帰無仮説H0は0 Elo、対立仮説H1は10 Elo、第一種と第二種の誤り確率はともに0.05である。

対数尤度比（LLR）の判定境界は約−2.944と+2.944、実行上限は3,000ペアである。
H1と判定された場合、または上限到達による判定保留時にLLRが0以上の場合は、長時間条件での最終測定へ進む。
H0と判定された場合、または上限到達時のLLRが負の場合は候補を採用しない。
採用の確定には、別の長時間測定でH1となり、エンジン異常、時間切れ、拒否着手が0件であることを要する。

## エンジン

候補は`960c56bcbb6088d0f0466d670ceea36dffeda5b7`、基準は`d0d3c52024c5ab4d74cc82588640a55953426aad`である。
候補は開始重みの鏡映対を平均した重みを持ち、学習と評価値の診断は[strength-stage7-mirror-training](strength-stage7-mirror-training.md)に記録した。
探索用駒価値47値と出力Kは基準と同じである。

manifestに保存された候補バイナリのSHA-256は`b3728444d9a844c83bc5959b5832959cb2ce0d647a4f5addd1a5ddcebf4bc154`、基準バイナリは`63c3512f466f47b0c3b8951b3fc74e0b786bc0b91b8ef7e6449fd16159a8c79f`であり、コミット別に保存された実行ファイルと一致した。
規則は`engine-default`から解決された`L0,P0,R1,E0`を両エンジンと審判層に適用する。

固定した`data/strength-stage7/match_runner`は版0.1.0で、SHA-256は`900d67a658d6075f34ef44a906e5c930ae661232f3f435be5a4de90498bd62ad`である。
実行中プロセスのバイナリもこの検査和と一致した。

## 環境

manifestのCPUはIntel Core Ultra 7 265KFで、物理20コア、論理20コアである。
候補と基準の`Threads`は各1、`USI_Hash`は各256 MB、同時対局数は自動計算された19である。
手数上限は4,096手、応答タイムアウトは120秒とする。

## 結果

測定は実行中であり、採否は未確定である。
実行プロセスの稼働と`summary.json`の`invocation_active: true`を確認した。
ペンタノミアル度数、LLRと判定、有効ペア数、破棄ペア数、エンジン異常、拒否着手、`time_forfeits`、経過時間の最終値は未確定であり、終了後の集計で記録する。

測定条件と各ペアは`data/matches/strength-stage7-mirror-stc/`、標準出力は`data/strength-stage7/mirror-stc.log`、継続に必要な実行情報は`data/strength-stage7/progress.json`に保存されている。

## 結論

短時間測定は実行中であり、長時間の最終測定への進行と候補の採否は未確定である。

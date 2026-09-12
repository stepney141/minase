# 段階7の鏡映重み共有モデルの長時間測定

## 目的

[短時間測定](strength-stage7-mirror-stc.md)を通過した鏡映重み共有候補を段階開始版と長時間条件（LTC）で比較し、[棋力向上段階7](../plans/strength-stage7.md)での採否を確定する。

## コマンドライン

2026年9月12日20時41分48秒（日本標準時）に、固定した実行ファイルで次の測定を開始した。
実行プロセスの引数、起動ログ、および保存されたmanifestの条件は一致している。

```console
data/strength-stage7/match_runner \
  --run-dir data/matches/strength-stage7-mirror-ltc --seed 20910903 \
  --candidate commit:960c56bcbb6088d0f0466d670ceea36dffeda5b7 \
  --baseline commit:d0d3c52024c5ab4d74cc82588640a55953426aad \
  --rules engine-default --each time=60000+200 \
  --candidate-hash 256 --baseline-hash 256 \
  --concurrency 19 --max-ply 4096 --response-timeout 120 \
  gsprt --max-pairs 100000
```

両者の持ち時間は60秒、1手ごとの加算は0.2秒で、秒読みは使わない。
同じ開始局面から先後を入れ替えた2局を1ペアとし、[測定手順](../sprt.md)の一般化逐次確率比検定で判定する。
帰無仮説H0は0 Elo、対立仮説H1は10 Elo、第一種と第二種の誤り確率はともに0.05である。
対数尤度比（LLR）の判定境界は約−2.944と+2.944とする。
上限100,000ペアで判定保留となった場合は、同じ実行ディレクトリを`--resume`で再開し、`--max-pairs`だけを増やす。

派生前のシード入力範囲20910904〜21010903を、[シード監査](strength-stage7-seed-audit.md)の過去150記録、段階7の試行生成500局、および短時間測定の投入上限3,000ペアと照合した。
過去のハッシュ付き149資料に変更はなく、範囲の重複と派生後のシード衝突はいずれも0件だった。
この照合は保存記録のある実行を対象とし、次の測定シードは本測定が実際に使った範囲を確認してから割り当てる。
再照合のスクリプトは`data/strength-stage7/mirror-ltc-seed-audit.py`、結果は`data/strength-stage7/mirror-ltc-seed-audit.json`に保存した。

## エンジン

候補は`960c56bcbb6088d0f0466d670ceea36dffeda5b7`、基準は`d0d3c52024c5ab4d74cc82588640a55953426aad`である。
候補は開始重みの鏡映対を平均した重みを持ち、探索用駒価値47値と出力Kは基準と同じである。
学習と評価値の診断は[strength-stage7-mirror-training](strength-stage7-mirror-training.md)に記録した。

manifestの候補バイナリのSHA-256は`b3728444d9a844c83bc5959b5832959cb2ce0d647a4f5addd1a5ddcebf4bc154`、基準バイナリは`63c3512f466f47b0c3b8951b3fc74e0b786bc0b91b8ef7e6449fd16159a8c79f`であり、短時間測定と同じ実行ファイルを使う。
規則は`engine-default`から解決された`L0,P0,R1,E0`を両エンジンと審判層に適用する。
固定したrunnerは版0.1.0で、SHA-256は`900d67a658d6075f34ef44a906e5c930ae661232f3f435be5a4de90498bd62ad`である。

## 環境

manifestのCPUはIntel Core Ultra 7 265KFで、物理20コア、論理20コアである。
候補と基準の`Threads`は各1、`USI_Hash`は各256 MB、同時対局数は19である。
手数上限は4,096手、応答タイムアウトは120秒とする。
教師データの生成、学習、および他の採否測定は並行して実行しない。

## 結果

測定は実行中であり、採否は未確定である。
ペンタノミアル度数、LLRと判定、有効ペア数、破棄ペア数と理由、エンジン異常、時間切れ、拒否着手、および累計時間は終了後に記録する。
測定条件と各ペアは`data/matches/strength-stage7-mirror-ltc/`、標準出力は`data/strength-stage7/mirror-ltc.log`、継続に必要な実行情報は`data/strength-stage7/progress.json`に保存されている。

## 結論

鏡映重み共有候補の採否は未確定である。
本測定がH1となり、エンジン異常、時間切れ、および拒否着手が0件の場合だけ採用する。

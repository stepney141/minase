# strength-stage8-improving-bench

## 目的

[棋力向上段階8](../plans/strength-stage8.md)のフェーズ3として、SEEによる捕獲手の枝刈りを採用した構成のbench深さ6で、improvingでないノードのfutility余裕値の倍率、LMRの追加減深の損失、および手の単位の発動率を、設計書の規則で決め直す。

## コマンドライン

計測は本番の経路へ入れず、[再診断の計測差分](strength-stage8-rediag-tools/diagnostic.patch)を採用構成へ当てた別ターゲットディレクトリのビルドで行い、候補は[評価スクリプト](strength-stage8-rediag-tools/evaluate.py)で評価した。

```console
git apply docs/measurements/strength-stage8-rediag-tools/diagnostic.patch
CARGO_TARGET_DIR=target/diag cargo build --release --bin bench
target/diag/release/bench --depth 6 --threads 1 --repetitions 1
MINASE_STAGE8_DIAG=data/experiments/stage8-rediag-formal/passive target/diag/release/bench --depth 6 --threads 1 --repetitions 1
MINASE_STAGE8_DIAG=data/experiments/stage8-rediag-formal/verified MINASE_STAGE8_VERIFY=1 target/diag/release/bench --depth 6 --threads 1 --repetitions 1
MINASE_STAGE8_LMR_DIAG=data/experiments/stage8-rediag-formal/lmr.txt target/diag/release/bench --depth 6 --threads 1 --repetitions 1
python3 docs/measurements/strength-stage8-rediag-tools/evaluate.py \
  --samples data/experiments/stage8-rediag-formal/verified --lmr data/experiments/stage8-rediag-formal/lmr.txt \
  --json docs/measurements/strength-stage8-improving-bench/results.json \
  --text docs/measurements/strength-stage8-improving-bench/results.txt
```

全候補の表は[results.txt](strength-stage8-improving-bench/results.txt)と[results.json](strength-stage8-improving-bench/results.json)にある。

## エンジン

SEEによる捕獲手の枝刈りの採用構成（コミット`c1c7227`と探索が同一のコミット`64a1524`）である。
`bench`は規則セット`engine-default`を固定で使い、深さ6の総ノード数は1,556,722である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、rustc 1.98.0であり、2026年9月20日に1スレッドで測った。
件数は決定的であり、経過時間は使わない。

## 結果

### 独立した参照との照合

4回の実行の総ノード数（副次的な探索を除く）は、すべて計測なしの1,556,722と一致した。
参照探索の有無で、捕獲手、非捕獲手、ノード、および補正の集計の4標本はバイト単位で一致した。
手の分類、静的評価の全再計算、および`see_prunes`と参照実装の契約（余裕値0と、ノードで実際に使う余裕値の両方）は全件で一致した。
探索値は対象手の64件に1件（11,976件）を空の置換表で参照探索し、記録側が良い結果とした2,584件はすべて参照探索でも良い結果であり、参照探索が良い結果とした2,589件のうち記録側も良い結果であったものは2,584件（99.81%）で、基準の80%を満たした。

### futility余裕値の倍率

対象手は、improvingが「上がっていない」の適用するノードでfutilityが展開した非捕獲手171,908手、良い結果は2,245手である。
表の各欄は「枝刈りされる手、失う割合（失う良い結果）」である。

| 残り深さ | 対象手 | 良い結果 | 3/4 | 1/2 | 1/4 | 選んだ倍率 |
|---|---:|---:|---|---|---|---:|
| 1 | 71,219 | 1,792 | 10,417、3.85%（69） | 23,171、5.80%（104） | 34,409、7.81%（140） | 1/4 |
| 2 | 70,332 | 425 | 8,027、2.12%（9） | 19,157、6.82%（29） | 37,585、10.59%（45） | 1/2 |
| 3 | 30,357 | 28 | 1,381、3.57%（1） | 6,962、7.14%（2） | 10,716、10.71%（3） | 1/2 |

残り深さ3は良い結果が28手しかなく、倍率の推定は他の深さより粗い。

### LMRの追加減深

対象手は9,522手（残り深さ3が850手、深さ4が8,672手）で、本来の深さの探索がαを上回った手は20手である。
失う割合は現行の減深量で3／20（15.0%）、追加後で6／20（30.0%）であり、増分は15.0ポイントで基準の10ポイントを超えた。
減深量が既に上限で対象外の手は106,221手である。

### 手の単位の発動率

分母は、futilityが展開すると決めた非捕獲手405,461手とLMRの判断を受ける非捕獲手487,328手の和集合553,563手である（重複339,226手は1回だけ数える）。
追加減深を外した構成では、選んだ倍率で展開されなくなる手は60,528手で、発動率は10.93%である。
追加減深を含めた場合は69,990手、12.64%であった。
参考値として、起案時のノードの単位の発動率は1.57%（3,740／237,695）である。

## 結論

improvingでないノードのfutility余裕値の倍率は、残り深さ順に1/4、1/2、1/2とする（余裕値は12、75、75）。
LMRの追加減深は、失う割合の増分が15.0ポイントで10ポイントを超えたので外し、余裕値の変更だけで測る。
手の単位の発動率は10.93%で5%以上なので、実装して採否測定へ進める。

# 棋力向上段階9の診断C（実戦開始の自己対局の信号）

## 目的

[棋力向上段階9](../plans/strength-stage9.md)の診断Cとして、実戦棋譜の局面から始める自己対局が、乱数開始の自己対局より、王の安全度の特徴と対局結果の結びつきを強く含むかを、群ごとの対局単位の対数損失の差 `g` とその差 `G` で測り、追加C（実戦開始の自己対局の局面をλ=0.75の教師の分類として加える）を採るかを判定する。

## コマンドライン

```console
target/release/lishogi_import openings --input data/strength-stage9/human/games-final.ndjson \
  --output data/strength-stage9/diag-c/openings-all.txt --nodes 100000 --concurrency 18 --hash-mb 16 \
  --report data/strength-stage9/diag-c/openings-report.json
# 棋譜IDごとに1局面を固定シード1で抽出して openings-diag-c.txt（2,000行）を作る
target/release/selfplay_gen generate --openings data/strength-stage9/diag-c/openings-diag-c.txt \
  --output data/strength-stage9/diag-c/human-start.bin --seed 2100000 --nodes 100000 \
  --random-moves 0 --concurrency 18 --hash-mb 16 --max-ply 4000
target/release/selfplay_gen generate --output data/strength-stage9/diag-c/random-start.bin \
  --games 2000 --seed 2000000 --nodes 100000 --random-moves 0 --concurrency 18 --hash-mb 16 --max-ply 4000
target/release/pst_probe --pst nets/pst.bin --positions <各MNSD> --king-features <各MNKF>
tools/train/.venv/bin/python tools/train/pst/human_signal_diag.py human-start \
  --human-start data/strength-stage9/diag-c/human-start.bin --random-start data/strength-stage9/diag-c/random-start.bin \
  --human-start-features data/strength-stage9/diag-c/human-start.mnkf \
  --random-start-features data/strength-stage9/diag-c/random-start.mnkf \
  --pst data/strength-stage9/pst-1633f53.bin --output-k 1072.6529541015625 \
  --bootstrap 2000 --seed 1 --output data/strength-stage9/diag-c/diag-c.json
```

抽出は、開始局面の全体を棋譜IDごとにまとめ、IDを固定シード1で並べ替えて先頭2,000個を選び、各IDから1局面を同じ乱数で選んだ（一括スクリプト `data/strength-stage9/teacher-pipeline.sh`）。

## エンジン

生成と開始局面の抽出はworktree `strength-stage9-teacher`（`f89b467`、段階開始版の学習PSTを追加特徴24列の重み0で拡張した重み）で、探索は100,000ノード、規則は `engine-default`、生成の設定は世代2と同じ（乱数の着手なし、手数上限4,000、置換表16 MB）である。
特徴値は `strength-stage9-diag-b`（`2fe29dd`）の118列、推定はブランチ `strength-stage9` の `30fbb01` のスクリプト、学習PSTの予測は段階開始版の `nets/pst.bin` と出力K 1072.6529541015625である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）で、他の負荷はない。
2026年9月22日に実施した。

## 結果

### 開始局面

[診断B](strength-stage9-human-signal.md)で取得した23,567局のうち、途中の局面から始まる459局とBotの参加する10,654局を除いた12,454局を既定の規則セットで再生し、手数が40以上で20の倍数、中盤（駒数47以上）、成り権の保留なしの候補42,089局面を得た。
100,000ノードの探索値の絶対値が500を超える21,317局面を除き、5,598局から20,772局面が残った。
このうち棋譜1つにつき1局面の条件で2,000局面を抽出した（1,000未満なら判断材料不足、1,000以上2,000未満なら全部を使う条件には当たらない）。

### 生成

| 群 | 局数 | 所要時間 | 記録した局面 | 平均手数 | 手数上限での破棄 | 勝敗 | 捕獲・成りの除外 | 詰み帯の除外 |
|---|---:|---:|---:|---:|---:|---|---:|---:|
| 実戦開始 | 2,000 | 67分 | 721,210 | 709.5 | 105 | 先手924、後手911、引き分け60 | 135,967 | 8,462 |
| 乱数開始 | 2,000 | 71分 | 808,566 | 643.0 | 77 | 先手936、後手938、引き分け49 | 142,942 | 8,398 |

実戦開始の試行16局では平均1,455手だったが、2,000局では乱数開始と近い長さになった。

### 推定

対象は中盤の局面で、引き分けの対局の局面は二値化できないので除いた。
分割は、実戦開始が棋譜IDのハッシュの偶奇、乱数開始が対局番号のハッシュの偶奇である。

| 群 | 係数側（対局、局面） | 検証側（対局、局面） | 正則化 | 基準の損失 | `g` | 標準誤差 | 片側95%下限 | 片側95%上限 |
|---|---|---|---:|---:|---:|---:|---:|---:|
| 実戦開始 | 928、133,846 | 904、129,463 | 0.1 | 約0.620 | +0.000031 | 0.000098 | −0.000139 | +0.000185 |
| 乱数開始 | 921、197,290 | 944、204,052 | 0.1 | 約0.585 | +0.000026 | 0.000037 | −0.000033 | +0.000085 |

`G = g(実戦開始) − g(乱数開始)` は+0.000006、標準誤差0.000105、片側95%の下限は−0.000172である。
2つのモデルはどちらの群でも収束し、再標本化で指標を計算できない反復はなかった。
候補の10列の係数は両群とも絶対値0.024以下であり、群の間で符号が一致しない列が多い。

## 結論

`g(実戦開始)` と `G` のどちらも片側95%の区間が0をまたぐので、診断Cは「判断材料不足」であり、追加Cは採らない。
実戦棋譜の局面から始めても、自己対局の対局結果は、学習PSTの予測に王の安全度の10列を加えたモデルで有意に予測が改善する信号を含まなかった。

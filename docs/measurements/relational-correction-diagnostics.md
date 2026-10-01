# 局所2駒表の採否前の診断

## 目的

[関係補正項の計画](../plans/relational-correction.md)の利用者の判断による測定のフェーズ3として、[学習した](relational-correction-training.md)半径1の局所2駒表について、自己選択の診断で倍率を決め、駒除去差分の符号と250根での着手と評価を診断する。
16ペアの対局は倍率の診断のためだけに行い、勝敗を採否に使わない。

## コマンドライン

```console
# 自己選択の診断の対局（候補は倍率1の表）
cargo run --release --bin match_runner -- \
  --run-dir data/matches/relational-pair-1-gdiag --seed 98500000 \
  --candidate commit:d7447b7 --baseline commit:c47106a \
  --each time=10000+100 elo --pairs 16
# 自己選択の差G
P=tools/train/.venv/bin/python
prep=docs/measurements/relational-correction-prep/prep
data=data
$P -B $prep/g_candidate.py --run-dir $data/matches/relational-pair-1-gdiag \
  --weights $data/relational-correction/phase2/pair-1.bin \
  --teacher $data/relational-correction/prep/samples/teacher.json \
  --probe target/release/pst_probe --extractor target/release/match_positions \
  --max-pairs 16 --output-dir $data/relational-correction/phase3/g-pair-1
# 駒除去差分の符号
$P -B $prep/removal.py --weights $data/relational-correction/phase2/pair-1.bin \
  --config $data/strength-stage9/lookahead-pst.toml --probe target/release/pst_probe \
  --seed 1 --sample-size 10000 --output-dir $data/relational-correction/phase3/removal-pair-1
# 250根の着手と評価
$P -B docs/measurements/qsearch-output-diagnostics/diagnose.py run \
  --engine S0=<零の表の実行ファイル> --weights S0=$data/relational-correction/prep/zero-tables/pst-zero-r1.bin \
  --engine C=data/worktrees/relational-pair-1/target/release/minase \
  --weights C=$data/relational-correction/phase2/pair-1.bin \
  --judge C --margin 117.7 --data $data/search-aware-evaluation \
  --out $data/relational-correction/phase3/diagnostics-250-pair-1 --jobs 16
```

診断の道具は本ブランチのコミット`5c60b55`（Gと駒除去差分）と`diagnose.py`の版3対応のコミットのものである。

## エンジン

候補はブランチ`relational-pair-1`のコミット`d7447b7`であり、S0（`c47106a`）に局所2駒表の推論と差分更新のコミットと、倍率1の重み（SHA-256 `c207372c…`）を加えたものである。
基準はS0の`c47106a`である。
250根の診断のS0には、本ブランチで採用PSTに半径1の零の表を付けた重みを埋め込んだ実行ファイルを使った。この構成の評価と探索がS0と一致することは[速度の記録](relational-correction-speed.md)で確かめてある。
規則は`L0,P0,R1,E0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）で、2026年10月2日2時に実施した。
16ペアの対局は同時19対局、両エンジン1スレッド、置換表256 MB、`time=10000+100`で行い、経過時間は390秒だった。
250根の診断は同時16根で、他の測定を走らせずに行った。

## 結果

### 自己選択の差と倍率

16ペアの対局で候補が手番を持ち、探索評価がセンチポーン値の局面は8,668局面であり、全局面が教師の標本（検証分割の100,000局面）との共通層に入った（被覆率100%）。
基準が手番を持つ8,667局面の被覆率も100%であり、被覆率の条件を置き換える必要はなかった。
層を揃えた補正量の平均は、対局の標本で+45.04センチポーン、教師の標本で−1.71センチポーンであり、$G=46.75$センチポーンだった。
$G\le G^\ast=88.25$センチポーンを満たすので、[設計書](../plans/relational-correction.md)の規則により倍率1の表を候補とした。
倍率1/4の診断は行っていない。

対局の結果は、ペンタノミアル度数[6, 0, 6, 0, 4]、Elo −43.7（95%信頼区間 −194.3〜+91.5）であり、エンジン異常、時間切れ、拒否着手は0件だった。
この結果は採否に使わない。

### 駒除去差分の符号

既存の駒除去診断と同じ選び方の352件で、駒1枚を除いた前後の評価差の符号が学習PSTだけの評価と逆になった例は0件だった。

### 250根の着手と評価

静止探索の計画の250根のうち、保存済みの教師値が通常の評価値でない8根を除いた242根で診断した。
判定の差mは現在のS0で測り直した117.7センチポーンとした。

| エンジン | 安定した根 | 平均の選択損失 | 通常の根 | 静的評価の平均誤差 | 平均絶対誤差 | 評価を下げる成り手 |
|---|---:|---:|---:|---:|---:|---:|
| S0 | 206 | 22.28 | 242 | −105.34 | 223.79 | 1/368 |
| 候補 | 206 | 22.66 | 242 | −104.56 | 220.93 | 2/368 |

| 指標 | 平均差（候補−S0） | 対局単位の標準誤差 | 下限 | 根数 | 対局数 |
|---|---:|---:|---:|---:|---:|
| 選択損失 | +0.38 | 5.28 | −9.97 | 206 | 173 |
| 静的評価の絶対誤差 | −2.87 | 4.37 | −11.44 | 242 | 202 |

どちらの指標も下限が0を超えないので、候補はS0より有意に悪化していない。
改善も検出されておらず、約200根の標本では小さな差を識別できない。

診断の要約は`relational-correction-prep/prep/diagnostics/`に写した。

## 結論

倍率は1とし、駒除去差分の逆転はなく、250根の診断でもS0からの有意な悪化はなかったので、倍率1の候補`d7447b7`をS0との段階ゲートへ出す。

# 棋力向上段階9の診断A（深い探索の感度）

## 目的

[棋力向上段階9](../plans/strength-stage9.md)の診断Aとして、10,000,000ノードの探索が100,000ノードの探索に比べて、中盤の露出局面（手番側の王駒について項目1の走り駒の特徴が発火している局面）の値を対照局面より下げるかを、層別の差の差Dで測り、追加A（中盤の露出局面の付け直し）を採るかを判定する。

## コマンドライン

層の計画、抽出、推定は `tools/train/pst/depth_sensitivity_diag.py` で行い、付け直しは教師用の固定worktreeの `selfplay_gen rescore` で行った。

```console
tools/train/.venv/bin/python tools/train/pst/depth_sensitivity_diag.py plan \
  --data <基本の教師の11ファイル> --king-features <118列のMNKF 11ファイル> \
  --output data/strength-stage9/diag-a/plan.json
tools/train/.venv/bin/python tools/train/pst/depth_sensitivity_diag.py sample \
  --plan data/strength-stage9/diag-a/plan.json --data ... --king-features ... \
  --split pilot --per-group 1000 --seed 1 --output-dir data/strength-stage9/diag-a/pilot
target/release/selfplay_gen rescore --input <MNSD> \
  --targets data/strength-stage9/diag-a/pilot/<MNSD名>.targets.txt \
  --output data/strength-stage9/diag-a/pilot/<MNSD名>.<ノード数>.mnrs \
  --nodes <100000 または 10000000> --hash-mb 16 --concurrency 1
tools/train/.venv/bin/python tools/train/pst/depth_sensitivity_diag.py estimate \
  --sample data/strength-stage9/diag-a/pilot/sample.json --data ... \
  --shallow <100000のMNRS 11ファイル> --deep <10000000のMNRS 11ファイル> \
  --bootstrap 2000 --seed 1 --output data/strength-stage9/diag-a/pilot/result.json
```

本番は `--split main --per-group 213` で同じ手順を繰り返した。

## エンジン

付け直しは、worktree `strength-stage9-teacher`（ブランチ `strength-stage9-teacher`、`73513ca`。`strength-stage9` の `18570cc` に、段階開始版の学習PSTを追加特徴24列の重み0で拡張した `nets/pst.bin` と、付け直しファイルの規則名の表記の修正を加えたもの）の `selfplay_gen` で行った。
この重みは段階開始版と同じ評価値を返し、`bench` 深さ5の総ノード数598,014が段階開始版と一致する。
探索は局面単独（履歴なし、置換表16 MBを局面ごとに消去、1スレッド）、規則は `engine-default` である。
予備の付け直しファイル22個は、規則名の欄が `engine-default` の表記で書かれていたので、推定の前にMNSDと同じ `L0,P0,R1,E0` の表記へ欄を書き換えた。本番の付け直しファイルは修正後のバイナリが直接この表記で書いた。
特徴値は `strength-stage9-diag-b` の `2fe29dd` が書き出した118列のMNKFであり、露出の判定には手番側の項目1の走り駒の6列（列6〜11）を使った。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
同じ機で別の作業者のSPSAの本番セッション（同時対局16）が動いていたので、付け直しは効率コア（論理CPU 8〜19）へ固定し、ファイルごとに1プロセスを並列に走らせた。
`rescore` の `--concurrency` は対象が疎な場合に並列化が効かず、1局面あたり6.7秒（10,000,000ノード）の直列処理になった。
2026年9月22日に実施した。

## 結果

### 層の計画

訓練の分割の中盤の局面のうち、手番側の王駒が1枚でない202局面を除いた対象は、露出局面が8,482,246、対照局面が6,332,706である。
層は世代3通り×手番2通り×相手の王駒の数2通り×保存済みの探索値の5区分の60層であり、相手の王駒が2枚の層は露出局面が最大42局面で重みの合計は0.0000であった。
対局は `hash64(seed, game) % 2` で予備用と本番用に分け、予備用に露出4,006,108局面、本番用に露出4,476,138局面が入った。

### 予備（各群1,000局面）

| 量 | 値 |
|---|---:|
| 通常の値を持つ局面 | 露出999、対照1,000 |
| 詰みの帯へ移った局面 | 露出1（深い探索で勝ち）、対照0 |
| 除外で外した層の重み | 0.00002 |
| D | +8.3センチポーン |
| 標準誤差（対局単位の再標本化2,000回、対局1,939） | 5.5 |
| 片側95%の上端 | +17.6 |
| 両側90%区間 | [−0.6, +17.6] |
| 完了した深さの平均 | 浅い探索5.8、深い探索11.6 |
| 本番の各群の局面数 `ceil(1000 × (5.5/12)²)` | 213 |

### 本番（各群213局面）

| 量 | 値 |
|---|---:|
| 通常の値を持つ局面 | 露出213、対照212 |
| 詰みの帯へ移った局面 | 露出0、対照1（負け） |
| 除外で外した層の重み | 0.0087 |
| D | −5.6センチポーン |
| 標準誤差（対局425） | 13.9 |
| 片側95%の上端 | +18.4 |
| 両側90%区間 | [−27.4, +18.4] |
| 完了した深さの平均 | 浅い探索5.8、深い探索11.6 |

本番の標準誤差は予備からの見積もり12を上回ったが、判定は上端が−30以下かどうかであり、上端は+18.4で基準から48センチポーン離れている。
予備と本番のどちらでも、深い探索は露出局面の値を対照局面より下げていない。

### 所要時間

100,000ノードの付け直しは1局面あたり0.08秒、10,000,000ノードは6.7秒であり、予備の2,000局面は10並列で約40分、本番の426局面は約10分を要した。
追加Aを採る場合の対象（基本の教師の中盤の露出局面8,482,246）は、この速度では10並列でも約66日を要するので、設計書の48時間の上限に収めるには約26,000局面への抽出が必要だった。

## 結論

診断Aは基準を満たさず、追加A（深い探索による付け直し）は採らない。
100,000ノードから10,000,000ノードへ深くしても、中盤の露出局面の探索値は対照局面と同じ程度にしか動かず、敗局の分析で見た王の危険は探索の深さだけでは教師値に現れない。

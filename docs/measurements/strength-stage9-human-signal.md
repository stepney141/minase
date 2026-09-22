# 棋力向上段階9の診断B（実戦棋譜の信号）

## 目的

[棋力向上段階9](../plans/strength-stage9.md)の診断Bとして、lishogiの実戦棋譜で、学習PSTの予測に王の安全度の10列を加えると対局結果の予測が改善するかを、検証用の対局における対局ごとの平均の対数損失の差で測り、追加B（実戦棋譜の局面をλ=0の教師の分類として加える）を採るかを判定する。

## コマンドライン

```console
python3 scripts/fetch_lishogi_games.py --output data/lishogi/games.ndjson --state data/lishogi/state.json
target/release/lishogi_import games --input data/strength-stage9/human/games-final.ndjson \
  --output data/strength-stage9/human/human.bin --seed 290501 --nodes 100000 \
  --concurrency 18 --hash-mb 16 --report data/strength-stage9/human/games-report.json
target/release/pst_probe --pst nets/pst.bin --positions data/strength-stage9/human/human.bin \
  --king-features data/strength-stage9/human/human.mnkf
tools/train/.venv/bin/python tools/train/pst/human_signal_diag.py human-games \
  --data data/strength-stage9/human/human.bin --king-features data/strength-stage9/human/human.mnkf \
  --games-table data/strength-stage9/human/human.bin.games.json \
  --pst data/strength-stage9/pst-1633f53.bin --output-k 1072.6529541015625 \
  --bootstrap 2000 --seed 1 --output data/strength-stage9/human/diag-b.json
```

## エンジン

取得はブランチ `strength-stage9` の `408c3e3` のスクリプト、変換はworktree `strength-stage9-teacher`（`f89b467`、段階開始版の学習PSTを追加特徴24列の重み0で拡張した重み）の `lishogi_import`、特徴値は `strength-stage9-diag-b`（`2fe29dd`）の118列、推定はブランチ `strength-stage9` の `30fbb01` のスクリプトである。
学習PSTの予測のロジットは段階開始版の `nets/pst.bin`（`1633f53` と同じ内容）と出力K 1072.6529541015625で計算した。
変換の探索は局面単独（履歴なし、置換表16 MBを局面ごとに消去、1スレッド、100,000ノード）、規則は `engine-default` である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）である。
取得は2026年9月22日10時23分から14時45分（4時間22分、要求3,353回、エラー13回、失敗3人はいずれもBotの読み取りタイムアウト）、変換は18並列で38分、推定は数分で終わった。

## 結果

### 取得と受理

チーム `chu-shogi-club` と `chushogi-wc` の会員を起点に対戦相手をたどり、3,340人から23,567局を取得した。リーダーボードのAPIは404で使えなかった。
受理の条件を順に適用した結果は次のとおりである。

| 理由 | 件数 |
|---|---:|
| 途中の局面から始まる（`initialSfen`） | 459 |
| Botの参加 | 10,654 |
| 非レート戦 | 2,758 |
| 持ち時間の区分がない | 2 |
| レーティングが欠けるか暫定 | 6,975 |
| 終局の理由が投了と王駒の捕獲以外（時間切れ480、timeout 12、合意の引き分け11、反復5、裸玉12） | 520 |
| 既定の規則セットでの再生で勝者が一致しない | 2 |
| 投了者と最後の局面の手番が整合しない | 162 |
| 受理 | 2,035 |

lishogiの書き出しは `speed` 欄を持たないので、持ち時間の区分は `clock`（リアルタイム）と `daysPerTurn`（通信対局）から導いた。
投了者の整合は、棋譜に投了者の記録がないため「最後の局面の手番が勝者の相手であること」で判定しており、相手の手番中に投了した162局がこの条件で除外された。この判定は設計書の「投了者と勝者の整合」の実装上の解釈であり、除外は判定を変えずに記録した。
受理した2,035局から、生成時の3つの除外を通過した303,592局面を記録した。

### 推定

棋譜IDのハッシュの偶奇で、診断用（係数と正則化を決める側）に1,015局151,962局面、検証用に1,020局151,630局面を分けた。
基準のモデルの説明変数はレーティングの差と平均、持ち時間の区分（通信対局、基本時間の3区分、欠損）、手番の先後、駒数の帯であり、候補はこれに手番側の王駒の項目1の走り駒6列と項目2の（攻め1または2以上, 守り0）の4列を加えた。
L2正則化は診断用の対局の5分割交差検証で {0, 1e-4, 1e-3, 1e-2, 1e-1} から選び、基準、候補とも0.1が選ばれた。どちらもニュートン法で収束した。

| 量 | 値 |
|---|---:|
| 検証用の対局ごとの平均対数損失（基準） | 約0.317 |
| 差 `g`（基準−候補）の対局平均 | +0.000023 |
| 標準誤差（対局単位の再標本化2,000回） | 0.000020 |
| 片側95%の信頼下限 | −0.000010 |
| 片側95%の信頼上限 | +0.000058 |

候補の10列の係数は絶対値0.015以下であり、走り駒1枚の距離1が+0.014、他は−0.010から−0.002の間にある。

## 結論

`g` の片側95%の信頼下限が0を超えないので、診断Bは基準を満たさず、追加Bは採らない。
受理した実戦棋譜では、レーティングと学習PSTの予測だけで対局結果の対数損失が0.32まで下がり、王の安全度の10列は検証用の対局で予測を改善しなかった。

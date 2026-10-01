# 局所2駒表とS0の固定ノード数の200ペア

## 目的

[関係補正項の計画](../plans/relational-correction.md)の利用者の判断による測定で、[STC](relational-correction-stc.md)が`H0`となった候補について、[相対2駒評価の計画](../plans/relative-pair-eval.md)の「診断」節の規定どおり、固定ノード数の200ペアのEloを測り、評価の質と速度の損失を切り分ける。
この測定は解釈のためであり、採否には使わない。

## コマンドライン

```console
cargo run --release --bin match_runner -- \
  --run-dir data/matches/relational-correction-nodes200 --seed 98700000 \
  --candidate commit:d7447b7 --baseline commit:c47106a \
  --each nodes=100000 elo --pairs 200
```

## エンジン

候補はブランチ`relational-pair-1`のコミット`d7447b7`、基準はS0の`c47106a`であり、[STC](relational-correction-stc.md)と同じである。
規則は`L0,P0,R1,E0`、両エンジンとも1手100,000ノードである。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）で、2026年10月2日3時13分から3時28分に実施した。
同時19対局、両エンジンとも1スレッド、置換表256 MBである。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 200（有効185、手数上限で破棄15） |
| ペンタノミアル度数 | [44, 3, 86, 3, 49] |
| Elo | +9.4（95%信頼区間 −26.4〜+45.4） |
| エンジン異常、時間切れ、拒否着手 | 0件 |
| 経過時間 | 909秒 |

## 結論

同じノード数では候補とS0の差を検出できず、点推定は+9.4 Eloにとどまった。
STCの約−75 Eloは、局所2駒表の計算による探索速度の低下（零の表で0.775倍）で主に説明され、評価の改善がその損失を補う大きさには達していない。

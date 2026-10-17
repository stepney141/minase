# strength-stage6-coefficients-diag

## 目的

棋力向上段階6の最終構成（コミット8f4e41c。aspiration windows、internal iterative reduction、最善手安定時の早期終了を含む）で、設計書の「係数の再導出」節に従い反復継続の固定比、対局長の定数、および加算の係数を再導出し、変更の対象を確定する。

## コマンドライン

```console
python3 data/experiments/stage6-diag-tools/id_diag.py target/release/minase \
  data/experiments/stage6-diag-tools/positions.json 8 10 data/experiments/stage6-diag-tools/id_depth8_final.json
python3 data/experiments/stage6-diag-tools/iteration_diag.py data/experiments/stage6-diag-tools/id_depth8_final.json
```

対局長の定数と終局時の残り時間は、最後に採用した項目の[STC](strength-stage6-stable-stc.md)と[LTC](strength-stage6-stable-ltc.md)の保存記録（1,796局）から、候補側について集計した。
残り手数の期待値は、その手数まで続いた対局の総手数から手数を引いて2で割った平均である。

## エンジン

コミット8f4e41cのリリースバイナリを`--protocol usi --rules L0,P0,R1,E0`、`USI_Hash`は既定の256 MBで起動した。
探索コードの変更はない。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）であり、反復深化は10プロセス並列で走らせ、他の対局は走らせていない。
経過時間は同じ局面内の反復の比だけに使う。

## 結果

深さ5から8までの累積時間比の中央値は深さ順に1.86、2.42、1.97、2.15、合わせた中央値は2.13（790標本）であり、0.1刻みで2.1、現行の2.5との差は16%である。

手数帯ごとの1側あたりの残り手数の期待値は次のとおりである。

| 手数 | 0 | 100 | 200 | 300 | 400 | 500 |
|---|---|---|---|---|---|---|
| 残り手数の期待値 | 257.9 | 211.4 | 166.5 | 131.0 | 129.9 | 167.8 |
| 現行の見積り | 225 | 175 | 125 | 100 | 100 | 100 |

`EXPECTED_PLIES`の再導出値は0手目の期待値の2倍を10刻みへ丸めた520（現行450との差16%）、`MIN_MOVES`の再導出値は手数帯の最小の期待値を10刻みへ丸めた130（現行100との差30%）である。
終局時の候補側の残り時間の中央値は、STCで1,669ms（基本時間の16.7%）、LTCで8,673ms（同14.5%）である。
破棄ペア数、異常件数、`time_forfeits`、対局の総経過時間は該当しない。

## 結論

固定比と`EXPECTED_PLIES`は差が20%以内、加算の係数は終局時の残り時間が基本時間の25%未満なので変更の対象にしない。
`MIN_MOVES`だけが差30%で対象になるため、`MIN_MOVES = 130`を1組の候補として最終構成とのSTCとLTCで測る。

# 段階BのB0の短い条件の選別（中断）

## 目的

[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)の段階Bの候補B0を、段階Aの採用構成と短い条件（持ち時間と秒読みの比20）で比べ、選別を通過するかを判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-b0-short-r2 --seed 2080000000 \
  --candidate commit:ec9bed6 --baseline commit:72fb0ce --each time=6000+0,byoyomi=300 --concurrency 11 gsprt --max-pairs 3000
```

## エンジン

候補はec9bed6（B0）、基準は72fb0ce（段階Aの採用構成に、挙動を変えない信号の計算と検査の修正を加えたもの）である。規則セットは`engine-default`、両者とも`Threads=1`、`USI_Hash`は256 MB、`ByoyomiMargin`は既定の30 msである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は11である。同じ測定機ではlishogiのBot（1局あたり`Threads=4`、最大2局同時）が動いており、負荷平均は約19だった。

## 結果

利用者の判断により、判定が出る前の2026年10月7日23時頃に測定を止めた。

| 項目 | 値 |
|---|---|
| 完了ペア数 | 562 |
| 番号順に取り込んだ有効ペア数 | 518 |
| ペンタノミアル度数 | [118, 19, 263, 31, 87] |
| LLR | −2.227（判定保留） |
| 得点率 | 約47.1% |
| 時間切れ | 7件（候補側：ペア10、72、205。基準側：ペア18、230、445、450） |
| 経過時間 | 17,853秒（`summary.json`の`active_wall_time_ns`） |

時間切れはいずれもBotの対局と重なった時間帯の秒読み期の手で、両側にほぼ対称だった。

## 結論

B0は段階Aより弱い傾向（得点率約47%、LLR −2.23）を示した。選別を通過するには`H1`または上限到達時のLLR 0以上が必要であり、この推移から届く見込みはないと利用者が判断して止めた。B0は選別を通過しない。

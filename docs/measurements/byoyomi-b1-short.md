# 段階BのB1の短い条件の選別（無効）

## 目的

[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)の段階Bの候補B1を、段階Aの採用構成と短い条件（持ち時間と秒読みの比20）で比べる選別の測定である。

## コマンドライン

```console
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-b1-short --seed 2050000000 \
  --candidate commit:e5af46e --baseline commit:72fb0ce --each time=6000+0,byoyomi=300 --concurrency 11 gsprt --max-pairs 3000
```

## エンジン

候補はe5af46e（B1）、基準は72fb0ce（段階Aの採用構成に、挙動を変えない信号の計算と検査の修正を加えたもの）である。規則セットは`engine-default`、両者とも`Threads=1`、`USI_Hash`は256 MB、`ByoyomiMargin`は既定の30 msである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は11である。
開始の約5分後に、別の作業の採否測定（`fm-quarter-current-pst-fn-stc`、同時16対局）が同じ測定機で始まり、lishogiのBotの2局同時の対局も重なった。負荷平均は最大約36に達した。

## 結果

完了したペアは29件で、番号順に取り込んだ有効15ペアのペンタノミアル度数は[2, 0, 10, 0, 3]、LLRは+0.068で判定保留だった。
完了したペアには時間切れが5件あり、基準側が3件、候補側が2件だった。いずれも秒読み期の手で、思考時間は300.0〜306.8 msだった。
経過時間は`summary.json`の`active_wall_time_ns`で約992秒である。

## 結論

測定機の過負荷による時間切れが出たので、本測定は採否に使わず、途中で止めた。別の測定が終わった後に、別の実行ディレクトリと新しいシードで測り直す。測り直しの理由は過負荷という外部要因であり、結果に基づく選び直しではない。

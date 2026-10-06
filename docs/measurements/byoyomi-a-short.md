# 段階Aの短い条件の採否測定（無効）

## 目的

[秒読みつき時計での持ち時間と秒読みの活用](../plans/byoyomi-time-usage.md)の段階Aを着手前のエンジンと短い条件（持ち時間と秒読みの比20）で比べ、GSPRTで採否を判定する。

## コマンドライン

```console
cargo run --release --bin minase -- match run --run-dir data/matches/byoyomi-a-short --seed 2029000000 \
  --candidate commit:ea0133d --baseline commit:33e7269 --each time=6000+0,byoyomi=300 --concurrency 16 gsprt
```

## エンジン

候補はea0133d（段階A）、基準は33e7269（master）である。規則セットは`engine-default`、両者とも`Threads=1`、`USI_Hash`は256 MB、`ByoyomiMargin`は既定の30 msである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は16である。
測定の全期間にわたり、同じ測定機で段階Bの実装のビルドと全件の検査（4スレッドで探索する実時間の検査を含む）を並行して実行した。負荷平均は最大23に達した。

## 結果

取り込みを止めた時点で、有効108ペアのペンタノミアル度数は[5, 0, 53, 9, 41]、LLRは+2.446で判定保留だった。
完了したペアの記録には時間切れが7件あり、すべて候補の負けだった。7件とも秒読み期のhard停止の手で、思考時間は300.5〜331.2 msであり、秒読み300 msを超えた。
エンジン異常と拒否着手の件数は、測定を強制終了したので最終サマリに出ていない。
経過時間は`summary.json`の`active_wall_time_ns`で約2,242秒である。

## 結論

時間切れが出たので、本測定は採否に使わない。原因は、締切の余裕30 msの候補が、測定と並行して実行したビルドと検査による過負荷で締切を超えたことである（[測定機で重い計算を始める前に進行中の測定を確認する](../lessons/check-running-measurements-before-cpu-load.md)）。測定機にほかの負荷がない状態で、別の実行ディレクトリと新しいシードから測り直す。

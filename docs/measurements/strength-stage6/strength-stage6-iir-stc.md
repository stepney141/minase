# strength-stage6-iir-stc

## 目的

棋力向上段階6のinternal iterative reduction（残り深さ3以上で置換表の記録手がないノードの残り深さを1減らす変更）を、直前の採用構成（aspiration windows）と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-iir-stc --seed 20620903 \
  --candidate commit:9d37246 --baseline commit:75bb69d \
  --each time=10000+100 --concurrency 18 gsprt --max-pairs 3000
```

## エンジン

候補はコミット9d37246（internal iterative reductionのコミット1797322の後に置換表クラスタ化を撤回した構成で、探索コードはaspiration windowsとinternal iterative reductionだけを含む）、基準はaspiration windowsを採用したコミット75bb69d、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した18である。
他の対局や診断は走らせていない。

## 結果

1,445ペアを実行し、有効ペア1,419、破棄ペア26（手数上限）であった。
ペンタノミアル度数は[277, 29, 742, 26, 345]、LLRは+2.965で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は11,151秒である。

## 結論

短時間GSPRTは`H1`かつ異常0件であり、候補をLTC（[strength-stage6-iir-ltc](strength-stage6-iir-ltc.md)、シード20630903）へ進める。

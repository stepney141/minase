# strength-stage6-aspiration-stc

## 目的

棋力向上段階6のaspiration windows（深さ5以上の反復で前回の評価値を中心に半幅50の窓を使い、外れた側を倍々に広げて読み直し、根でβ打ち切りと保存種別の分類を行う変更）を、段階開始版と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-aspiration-stc --seed 20600903 \
  --candidate commit:75bb69d --baseline commit:75a08de \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミット75bb69d、基準は段階開始版のコミット75a08de（探索コードは段階5完了後の156816dおよびbc67813と同一）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した16である。
測定の途中で、置換表クラスタ化の対局再生診断（4プロセス）とcodexによる実装作業が並行していた。

## 結果

230ペアを実行し、有効ペア223、破棄ペア7（手数上限）であった。
ペンタノミアル度数は[28, 2, 106, 8, 79]、LLRは+2.979で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は2,139秒である。

## 結論

短時間GSPRTは`H1`かつ異常0件であり、候補をLTC（[strength-stage6-aspiration-ltc](strength-stage6-aspiration-ltc.md)、シード20610903）へ進める。

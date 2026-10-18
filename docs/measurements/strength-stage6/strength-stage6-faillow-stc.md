# strength-stage6-faillow-stc

## 目的

棋力向上段階6のfail-lowによる延長（1手の探索で根のaspiration windowsがfail-lowを起こしたら、その手の残りの反復継続の判断からsoftの条件を外し、hardの予測だけで次の反復へ入るかを決める変更）を、直前の構成（internal iterative reductionまで）と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-faillow-stc --seed 20640903 \
  --candidate commit:4efe9bf --baseline commit:9d37246 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミット4efe9bf、基準はinternal iterative reductionを含むコミット9d37246（LTCが`H1`だが外部要因の時間切れを伴い、採用の可否は利用者の決定待ち。暫定の基準）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した16（前の測定で外部のメモリ圧迫による時間切れが出たため余裕を残した）である。
他の対局や診断は走らせていない。

## 結果

302ペアを実行し、有効ペア292、破棄ペア10（手数上限）であった。
ペンタノミアル度数は[83, 7, 153, 7, 42]、LLRは−2.945で`decision: H0`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は2,957秒である。

## 結論

短時間GSPRTは`H0`であり、fail-lowによる延長は不採用とする。
実装はコミットad0d9feで外し、最善手交替時の延長と統合した判断関数の`extend`引数は残した。

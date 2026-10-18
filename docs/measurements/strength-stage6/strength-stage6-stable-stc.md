# strength-stage6-stable-stc

## 目的

棋力向上段階6の最善手安定時の早期終了（直近4反復の最善手が同じなら、次の反復へ入る条件を「経過時間がsoft未満」から「経過時間に固定比2.5を掛けた予測完了時刻がsoft以下」へ置き換える変更）を、直前の構成（internal iterative reductionまで）と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-stable-stc --seed 20660903 \
  --candidate commit:8f4e41c --baseline commit:9d37246 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミット8f4e41c（延長の2項目を外した後に早期終了だけを加えた構成）、基準はinternal iterative reductionを含むコミット9d37246（暫定の基準）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した16である。
他の対局や診断は走らせていない。

## 結果

281ペアを実行し、有効ペア275、破棄ペア6（手数上限）であった。
ペンタノミアル度数は[34, 4, 150, 6, 81]、LLRは+2.988で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は2,460秒である。

## 結論

短時間GSPRTは`H1`かつ異常0件であり、候補をLTC（[strength-stage6-stable-ltc](strength-stage6-stable-ltc.md)、シード20670903）へ進める。

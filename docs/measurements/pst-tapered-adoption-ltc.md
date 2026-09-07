# pst-tapered-adoption-ltc

## 目的

2端点PST（序中盤用と終盤用のPSTを盤上総駒数で線形補間する候補）を開始版と標準LTCのGSPRTで比較し、採否を判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/pst-tapered-adoption-ltc --seed 20550903 \
  --candidate commit:77fe8c03e485d3a83b00d8f8a4127f7dbee0faca \
  --baseline commit:e7ccafc32939801d4e6436a860731be21a66f4cb \
  --each time=60000+200 gsprt
```

## エンジン

候補は2端点PSTを埋め込んだコミット`77fe8c03e485d3a83b00d8f8a4127f7dbee0faca`（バイナリのSHA-256 `b1e078d118318ad1b0bda2d9a3ea44c2f043730bc7ec35dd39006b5e3aa800dc`）であり、基準は開始版の評価実装と重みを持つコミット`e7ccafc32939801d4e6436a860731be21a66f4cb`（SHA-256 `5ba2b8df341964a966d248a7813e406fa844c5690873eb3a7952c1073887158c`）である。
候補と基準の差は、MNPTバージョン2の補間評価、埋め込み重み、および固定した探索用駒価値であり、探索コードは同一である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）を両エンジンと審判層に与えた。
シード20550903は、[STC](pst-tapered-adoption-stc.md)のシード20540903から10,000離れており、対局列は重複しない。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）、OSはLinuxである。
候補と基準の`Threads`は1、`USI_Hash`は256 MB、同時対局数は自動計算の19、手数上限は4,096手、応答タイムアウトは120秒である。
この測定中に他の生成、学習、対局は走らせていない。

## 結果

2026年9月7日から8日にかけて191ペアを実行し、有効ペア187、破棄ペア4（ペア13、93、100、147の第1局が手数上限4,096手に到達）であった。
ペンタノミアル度数は[21, 2, 84, 0, 80]、LLRは+2.970で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は4,520秒（1.3時間）である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、2端点PSTを採用する。

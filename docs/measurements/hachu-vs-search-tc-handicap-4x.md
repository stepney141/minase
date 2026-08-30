# HaChu対minaseの時間ハンデ固定局数Elo（minase持ち時間4倍）

## 目的

等時間で−252 Eloだった差の要因分析として、minaseの持ち時間をHaChuの4倍（120秒＋2秒対30秒＋1秒）にした固定局数Eloで、差が探索時間の増加で埋まるかを確認する。

## コマンドライン

```console
match_runner --seed 20260825 --candidate commit:a49bd46 \
  --candidate-limit time=120000+2000 --baseline "cecp:../hachu-debian/hachu" \
  --baseline-limit time=30000+1000 --rules L1,L3,P5,P6,R2,E1,E2 \
  --concurrency 8 elo --pairs 24
```

## エンジン

候補はコミットa49bd46（`Threads=1`）、基準はHaChuのDebianパッケージ収録のオリジナル版（コミットdf26f4a、`gcc -O2`でビルド、規則オプションは既定）である。
規則セットは`L1,L3,P5,P6,R2,E1,E2`（RULES.md第13版当時の表記。第14版以後は`L1,L3,P0,P5,P6,R2,E1,E2`）である。

## 環境

等時間測定（[hachu-vs-search-tc60.md](hachu-vs-search-tc60.md)）と同一で、CPUはIntel Core Ultra 7 265KF（物理20コア、論理20、SMTなし）、候補`Threads=1`、基準は単一スレッド、`USI_Hash`と`memory`は256 MB、同時対局数8である。
時間制御は候補が120秒＋2秒加算、基準が30秒＋1秒加算である。
測定日は2026年8月25日である。

## 結果

24ペアでペンタノミアル度数は[4, 0, 15, 0, 5]、Elo +14、95%信頼区間は[−71, +102]だった。
異常は`rejected_moves=1`（P5系統の既知の規則差）のみで、破棄ペア数と`time_forfeits`は記録なし。
経過時間は6,969秒だった。

## 結論

持ち時間4倍でminaseは統計的に互角へ達し、等時間の−252からの約265の回復は持ち時間2倍化およそ2回分に相当する。

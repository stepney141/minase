# hachu-pst-gen0

## 目的

評価関数マイルストーンの最終採用である学習PST（世代0）と外部エンジンHaChuを、対等な時間制御の固定局数対局で比べ、Eloと95%信頼区間を参考値として記録する。

## コマンドライン

```console
cargo run --release --bin match_runner -- --candidate commit:7e13888 --baseline "cecp:../hachu-debian/hachu" --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 16 --seed 20260829 elo --pairs 200
```

この測定は`--run-dir`の導入前に実施したため、実行ディレクトリは残っていない。
測定名は本記録で付けた名前である。

## エンジン

候補はコミット7e13888（学習PST）である。
基準はHaChuのDebianパッケージ収録版（コミットdf26f4a、RULES.md［E5］）を既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）で用いた。
ビルド手順は docs/sprt.md の外部エンジン比較に従う（詳細は記録なし）。
規則セットは`L1,L3,P0,P5,P6,R2,E1,E2`（RULES.md第33条第7項のHaChu既定設定に対応する組合せ）を審判層と候補の双方に与えた。

## 環境

CPU型、物理コア数、論理コア数は記録なし。
候補は`Threads=1`、`USI_Hash`は既定、同時対局数は16である。
HaChuのワーカー数は記録なし（ハーネスは`memory 256`を送る）。

## 結果

200ペア（破棄0）、ペンタノミアル度数は[137, 0, 60, 0, 3]、Eloは−282（95%信頼区間[−330, −241]）である。
`rejected_moves`は0、`time_forfeits`は0、`illegal_moves`は1である。
経過時間は14,411秒（4.0時間）である。
`illegal_moves`の1件はペア135第1局（minase先手）の1,199手目にHaChuが審判層の合法手にない着手を返したもので、規約どおりHaChuの反則負けとして算入した。
ハーネスのログは開始手順しか残さないため着手は特定できないが、1,199手に及ぶ対局でHaChuが自身の反復検出の記憶範囲を超えて既出局面を再現し、R2に反したと考えるのが最も自然である。
HaChuのプロトコル拒否（`rejected_moves`）は0であり、規則の不一致は確認されなかった。
終局した局面の着手列をログへ残す改良は、対局ハーネスの課題として別途扱う。

## 結論

v0のHaChu戦（docs/plans/match-harness.md、等時間で−252 Elo）と信頼区間が重なり、自己対局で得た+87 EloはHaChu戦の成績には表れていない。

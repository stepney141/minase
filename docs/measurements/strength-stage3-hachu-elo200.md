# strength-stage3-hachu-elo200

## 目的

棋力向上段階3の採用構成とHaChuを対等な時間制御で固定200ペア対局させ、段階間の進捗指標として記録する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage3-hachu-elo200 --seed 20330903 \
  --candidate commit:1894d14 --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 19 \
  elo --pairs 200
```

## エンジン

候補は採用構成のコミット1894d14（駒価値の再調整とSEE）である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian/hachu`）で、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）に対応する規則セット`L1,L3,P0,P5,P6,R2,E1,E2`を審判層とminaseの双方へ与えた。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補の`Threads`は1、HaChuは`memory 256`、`USI_Hash`は256MB、同時対局数は19（HaChuがワーカー数を報告しないため明示）である。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[16, 0, 99, 0, 85]、Eloの点推定は+125.0（95%信頼区間+92.0〜+160.3）であった。
`crashes=1`であり、不正着手、応答タイムアウト、`time_forfeits`、および拒否着手は0件、経過時間は10,804秒である。
クラッシュはペア76第1局の2,001手目にHaChu側（後手）が起こしたもので、測定規約どおり当該局の反則負けとして算入した。
minase側の異常は0件である。

## 結論

段階3の採用構成はHaChuに対して+125.0 Eloである。段階2完了時の+149.3 Elo（95%信頼区間+115.5〜+186.0）と信頼区間が重なり、200ペアの精度では段階2からの変化を判別できない。この測定は進捗指標であり、変更の採否には用いない。

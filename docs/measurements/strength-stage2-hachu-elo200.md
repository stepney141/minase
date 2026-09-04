# strength-stage2-hachu-elo200

## 目的

棋力向上段階2の採用構成とHaChuを対等な時間制御で固定200ペア対局させ、段階間の進捗指標として記録する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage2-hachu-elo200 --seed 20260933 \
  --candidate commit:ce654d9 --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 19 \
  elo --pairs 200
```

開始直後に外部から2回停止されたため、ペア完了0件の状態から同じ条件を`--resume`で再開した。
再開を含むため、`summary.json`の`interrupted`は`true`であり、スループットの校正には使わない。

## エンジン

候補は採用構成のコミットce654d9である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian/hachu`）で、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）に対応する規則セット`L1,L3,P0,P5,P6,R2,E1,E2`を審判層とminaseの双方へ与えた。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補の`Threads`は1、HaChuは`memory 256`、`USI_Hash`は256MB、同時対局数は19（HaChuがワーカー数を報告しないため明示）である。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[13, 0, 93, 0, 94]、Eloの点推定は+149.3（95%信頼区間+115.5〜+186.0）であった。
`illegal_moves=1`であり、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手は0件、経過時間は11,536秒である。
不正着手はペア141第1局の1,357手目にHaChu側（後手）が返したもので、測定規約どおり当該局の反則負けとして算入した。
minase側の異常は0件である。

## 結論

段階2の採用構成はHaChuに対して+149.3 Eloであり、段階1完了時の+79.5 Eloから約70 Elo上昇した。この測定は進捗指標であり、変更の採否には用いない。

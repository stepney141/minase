# strength-stage4-hachu-elo200

## 目的

棋力向上段階4の採用構成（futility pruning）の外部エンジンHaChuに対する強さを、対等な時間制御の固定200ペアのEloで進捗指標として記録する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-hachu-elo200 --seed 20470903 \
  --candidate commit:a641083 --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 19 \
  elo --pairs 200
```

## エンジン

候補はfutility pruningを採用したコミットa641083である。
基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian/hachu`としてビルド済み）で、規則オプションは既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは両者ともRULES.md第33条第7項の`L1,L3,P0,P5,P6,R2,E1,E2`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補の`Threads`は1、`USI_Hash`は256MB、HaChuのメモリは256MB、同時対局数は明示の19である。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[7, 0, 82, 0, 111]、Eloの点推定は+200.2（95%信頼区間+164.8〜+239.9）であった。
`crashes=6`であり、不正着手、応答タイムアウト、`time_forfeits`、および拒否着手は0件、経過時間は12,872秒である。
クラッシュはペア52、118、171、174、194の第1局とペア162の第2局で、いずれも2,001手目または2,002手目にHaChu側が起こしたもので、測定規約どおり当該局の反則負けとして算入した。
段階3の測定でも同じ手数でHaChu側のクラッシュが1件あり、HaChu側の長手数の局に固有の挙動である。
minase側の異常は0件である。

## 結論

段階4の採用構成はHaChuに対して+200.2 Eloである。段階3完了時の+125.0 Elo（95%信頼区間+92.0〜+160.3）とは信頼区間が重ならず、200ペアの精度でも段階3からの向上が認められる。この測定は進捗指標であり、変更の採否には用いない。

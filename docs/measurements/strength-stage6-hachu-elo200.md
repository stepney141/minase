# strength-stage6-hachu-elo200

## 目的

棋力向上段階6の最終構成（aspiration windows、internal iterative reduction、最善手安定時の早期終了を採用）の外部エンジンHaChuに対する強さを、固定200ペアのEloで進捗指標として記録する。
採否の判定には使わない。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-hachu-elo200 --seed 20700903 \
  --candidate commit:8f4e41c --baseline "cecp:../hachu-debian/hachu" \
  --rules L1,L3,P0,P5,P6,R2,E1,E2 --each time=60000+1000 --concurrency 10 \
  elo --pairs 200
```

## エンジン

候補は段階6の最終構成のコミット8f4e41c、基準はHaChu（Debianパッケージ収録のオリジナル版、コミットdf26f4a、`../hachu-debian`でビルド）であり、規則オプションはHaChuの既定設定（“Okazaki rule” 無効、“Promote on entry” 有効、“Allow repeats” 無効）である。
規則セットは審判層とminase側の双方に`L1,L3,P0,P5,P6,R2,E1,E2`を与えた。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補の`Threads`は1、`USI_Hash`は256MB、HaChuのメモリは256MB、同時対局数は明示の10である。
他の対局や診断は走らせていない。

## 結果

200ペアを実行し、有効ペア200、破棄ペア0であった。
ペンタノミアル度数は[2, 0, 47, 0, 151]、Eloの点推定は+334.1（95%信頼区間+289.0〜+390.1）であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は17,766秒である。

## 結論

段階6の最終構成はHaChuに対して+334.1 Eloであり、段階5完了時の+188.5 Elo（95%信頼区間+153.1〜+228.0）および2端点PST採用後の対等条件の+207.5 Elo（同+168.8〜+251.4）と信頼区間が重ならず、段階6でHaChu戦の強さが有意に伸びた。
進捗指標として記録する。

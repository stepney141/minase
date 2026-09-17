# time-management-opening-coefficient-elo

## 目的

[秒読みの予算に序盤の係数を加える](../plans/time-management-opening-coefficient.md)で採用した係数について、利用者の求めにより秒読みつきの条件で棋力の変化がないことを確かめる参考測定である。
秒読みのない時間制御では式が現行と一致するので、棋力が変わり得るのは秒読みつきの対局だけである。
採否の根拠は時計の診断（[time-management-opening-coefficient-diag](time-management-opening-coefficient-diag.md)）であり、本測定は採否を変える判定には使わない。

## コマンドライン

```console
target/release/match_runner --run-dir data/matches/time-management-opening-coefficient-elo --seed 20290918 \
  --candidate commit:2945d7e --baseline commit:ae1c59b \
  --concurrency 16 --each time=9000+0,byoyomi=300 elo --pairs 300
```

時間制御は持ち時間と秒読みの比が5分＋秒読み10秒と同じ（30倍）になるよう選び、係数が働く最初の36手に持ち時間が残る条件にした。

## エンジン

候補はコミット2945d7e（B′に秒読みの項の序盤の係数を加えたもの）、基準はコミットae1c59b（B′）、規則セットは`L0,P0,R1,E0`、両側の`Threads`は1、`USI_Hash`は256 MBである。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は明示した16、他の測定はなかった。

## 結果

| 項目 | 値 |
|---|---|
| 有効ペア数 | 290（300実行、10は手数上限4,096手で破棄） |
| ペンタノミアル度数 | [67, 4, 147, 8, 64] |
| Eloの点推定 | −1.2 |
| 95%信頼区間 | −28.4〜+26.0 |
| 時間切れ・不正着手・クラッシュ・応答タイムアウト・拒否着手 | すべて0件 |
| 経過時間 | 4,037秒 |

600局の手数は35〜4,096手（中央値420手）である。

## 結論

秒読みつきの条件で、係数の有無による棋力の差は点推定−1.2 Elo、95%信頼区間−28〜+26 Eloであり、差は検出されなかった。
300ペアの信頼区間の幅からは、−28 Eloを超える低下がないことまでしか言えない。
これは参考測定であり、採用の根拠は時計の診断のままである。

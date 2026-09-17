# time-management-opening-coefficient-noninferiority

## 目的

[持ち時間の効率的な使用](../plans/time-management-efficiency.md)で採用した秒読みの項の序盤の係数と置換表の2修正について、利用者の求めにより、秒読みつきの時間制御で統合ブランチがmasterより有意に弱くなっていないことを確かめる非劣性の測定である。
候補と基準を入れ替え、masterを候補、統合ブランチの先頭を基準にしたGSPRTで、`H0`なら「masterが有意に強いとは言えない」、すなわちH1の10 Elo規模の低下が検出されないと判定する。
秒読みのない時間制御では式が現行と一致し、固定深さのbench総ノード数もmasterと一致するので、棋力が変わり得るのは秒読みつきの対局だけである。
採否の根拠は時計の診断（[time-management-opening-coefficient-diag](time-management-opening-coefficient-diag.md)）のままであり、本測定は採否を変える判定には使わない。

## コマンドライン

```console
target/release/match_runner --run-dir ../../matches/time-management-opening-coefficient-noninferiority --seed 20400000 \
  --candidate commit:7f9d2a7 --baseline commit:373d3d8 \
  --concurrency 16 --each time=9000+0,byoyomi=300 gsprt --max-pairs 4000
```

時間制御は[参考のElo測定](time-management-opening-coefficient-elo.md)と同じく、持ち時間と秒読みの比が5分＋秒読み10秒と同じ（30倍）になるよう選び、係数が働く最初の36手に持ち時間が残る条件にした。
シードは、既存の測定のシードから上限ペア数以上離れた未使用の値である。

## エンジン

候補はコミット7f9d2a755f75599baee506691241d7f21617f748（master。統合ブランチの分岐点）、基準はコミット373d3d857cf6f27884c9a1856d10dd98dc09eb1c（統合ブランチ`time-management`の先頭。係数と置換表の2修正を含む）、規則セットは`L0,P0,R1,E0`、両側の`Threads`は1、`USI_Hash`は256 MBである。
仮説はH0がelo=0、H1がelo=10、α=β=0.05である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、同時対局数は明示した16、他の測定はなかった。

## 結果

| 項目 | 値 |
|---|---|
| 実行ペア数 | 3,765（うち191は手数上限4,096手で破棄） |
| 有効ペア数 | 3,574 |
| ペンタノミアル度数 | [742, 135, 1823, 117, 757] |
| 候補（master）の得点率 | 50.1% |
| LLR | −2.989 |
| 判定 | `H0` |
| 時間切れ・不正着手・クラッシュ・応答タイムアウト・拒否着手 | すべて0件 |
| 経過時間 | 46,710秒（`summary.json`の`active_wall_time_ns`） |

7,530局の手数は12〜4,096手（中央値419手）である。
LLRは860ペア時点で+0.69まで上がった後に下がり、3,574有効ペアでH0の境界を越えた。

## 結論

masterを候補にしたGSPRTは`H0`であり、秒読みつきの条件で統合ブランチがmasterより10 Elo規模で弱いという仮説は棄却された。
得点率50.1%は差がほぼ0であることを示すが、GSPRTは点推定の精度を保証しないので、小さな差の有無は言えない。
本測定は非劣性の確認であり、採用の根拠は時計の診断のままである。

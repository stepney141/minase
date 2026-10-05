# eval-coadaptation-lambda075-stc400

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)の「追加診断」の「追加学習の対照」として、G23の上へλ=0.75のまま10エポックを重ねた対照Pcを載せたCcを、[Cλ対S0](eval-coadaptation-before-stc400.md)と同じ開始局面の組でS0と400ペア対局させる。
Pλの負けを、混合比λ=1.0の効果と10エポックの追加学習の効果に分けることが目的であり、採否の根拠にはしない。

## コマンドライン

```console
target/release/match_runner \
  --run-dir data/matches/eval-coadaptation-lambda075-stc400 --seed 13000000 \
  --candidate commit:4114723aac1907202d36b1b9a52693623a666ab5 \
  --baseline commit:4fb158284648c26be039544385359e5704e13cb5 \
  --each time=10000+100 --concurrency 16 elo --pairs 400
```

振り分けの計算は、[判定スクリプト](eval-coadaptation-analysis/analyze.py)の`stage1`を、`--before`にCλ対S0、`--after`にCc対S0を与えて行い、出力を[lambda075.json](eval-coadaptation-analysis/lambda075.json)に保存した。
出力のE1がEc、RがEc−E0である。

```console
python3 docs/measurements/eval-coadaptation-analysis/analyze.py stage1 \
  --before data/matches/eval-coadaptation-before-stc400 \
  --after data/matches/eval-coadaptation-lambda075-stc400
```

## エンジン

候補はCc（ブランチ`eval-coadaptation-lambda075`の`4114723`、バイナリのSHA-256 `749cdade…`）であり、Mの`crates/minase/nets/pst.bin`を[Pc](eval-coadaptation-lambda075-training.md)へ差し替えたものである。
基準はS0（master `4fb1582`、バイナリのSHA-256 `5cb17c63…`）である。
規則セット、`Threads`、`USI_Hash`、およびrunnerは、Cλ対S0の測定と同じである。
判定スクリプトは、両測定の`manifest.json`の基本シード、思考制限、同時対局数、置換表容量、および基準のバイナリのSHA-256が一致することを確かめた。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月5日8時36分から9時47分に実施し、経過時間は4,220秒だった。
開始直後に[評価値の分布の比較](eval-coadaptation-distribution.md)を`nice -n 19`の1スレッドで約12秒実行したほかは、他の処理は走っていなかった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 400（完走372、手数上限による破棄28） |
| ペンタノミアル度数 | [53, 16, 175, 17, 111] |
| Elo（match_runnerの正規近似、自身の完走ペア） | +55.1（95%信頼区間 +31.3〜+79.4） |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

破棄されたペアの番号は、4、18、19、82、87、152、163、168、171、173、177、183、194、195、199、205、221、227、252、262、300、308、323、332、339、360、368、371である。

Cλ対S0と共通に完走した349ペアでの量は次のとおりである。
信頼区間は、共通ペアをペア番号単位で10,000回復元抽出する対応のあるブートストラップのパーセンタイル法による。
判定不能の理由はなかった。

| 量 | 点推定 | 95%信頼区間 |
|---|---:|---|
| E0（Cλ対S0） | −145.9 | −174.0〜−120.0 |
| Ec（Cc対S0） | +56.7 | +32.4〜+81.6 |
| Ec − E0 | +202.6 | +166.7〜+239.8 |

## 結論

Ecの95%信頼区間の上限は0を下回らないので条件Aは偽、Ec−E0の95%信頼区間の下限は0を上回るので条件Bは真である。
振り分けは「混合比による悪化だけを示した」である。
10エポックの追加学習だけによる負けは確認されず、同じ追加学習をλ=1.0で行うと、λ=0.75より約200 Elo弱くなった。
Pλの約150 Eloの負けは、追加学習ではなく混合比λ=1.0に結び付く。
さらに、Ecの95%信頼区間の下限は0を上回ったので、Ccは採用PSTより強い可能性がある。
設計書のとおり、段階ゲートで採否を測るかを利用者に確かめる。

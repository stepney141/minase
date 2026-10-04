# eval-coadaptation-after-stc400

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)のフェーズ4として、探索係数をCλに合わせて[SPSAで調整](eval-coadaptation-spsa.md)した候補Cλ\*を、[調整前の測定](eval-coadaptation-before-stc400.md)と同じ開始局面の組でS0と400ペア対局させ、負けがどれだけ縮むかを測る。
この測定は第1段の判定に使い、採否の根拠にはしない。

## コマンドライン

```console
target/release/match_runner \
  --run-dir data/matches/eval-coadaptation-after-stc400 --seed 13000000 \
  --candidate commit:42121149000eebd4ef0f7e6822d90ef1ef5b2fe9 \
  --baseline commit:4fb158284648c26be039544385359e5704e13cb5 \
  --each time=10000+100 --concurrency 16 elo --pairs 400
```

判定の計算は[判定スクリプト](eval-coadaptation-analysis/analyze.py)で行い、出力を[stage1.json](eval-coadaptation-analysis/stage1.json)に保存した。

```console
python3 docs/measurements/eval-coadaptation-analysis/analyze.py stage1 \
  --before data/matches/eval-coadaptation-before-stc400 \
  --after data/matches/eval-coadaptation-after-stc400
```

## エンジン

候補はCλ\*（ブランチ`eval-search-coadaptation`の`4212114`、バイナリのSHA-256 `de788bc3…`）であり、Cλの係数の表の既定値へセッションの最終値を書き込んだものである。
基準はS0（master `4fb1582`、バイナリのSHA-256 `5cb17c63…`）であり、調整前の測定と同じバイナリである。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MB、runnerのSHA-256は`616751c0…`で、調整前の測定と同じである。
判定スクリプトは、両測定の`manifest.json`の基本シード、思考制限、同時対局数、置換表容量、および基準のバイナリのSHA-256が一致することを確かめた。

Cλ\*では、単体テスト`razoring_accepts_quiescence_equal_to_alpha`が失敗する。
このテストは、razoringの余裕値から探索窓を作る局面を固定しており、`RazoringMargin2`が2,278へ上がると、静止探索が窓の下限をそのまま返すという前提が成り立たなくなる。
エンジンの実行ファイルはテストに依存しないので、測定の結果には影響しない。
Cλ\*は採否の測定へ進まず、ブランチもmasterへ統合しないので、テストは直していない。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月5日6時41分から7時54分に実施し、経過時間は4,376秒だった。
同じ時間帯に他の対局や学習は走っていなかった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 400（完走372、手数上限による破棄28） |
| ペンタノミアル度数 | [180, 7, 157, 4, 24] |
| Elo（match_runnerの正規近似、自身の完走ペア） | −157.0（95%信頼区間 −184.3〜−131.4） |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

破棄されたペアの番号は、5、19、57、59、62、78、96、97、114、126、130、139、140、141、146、197、209、220、252、261、264、267、270、307、361、375、385、392である。
両測定で完走した共通ペアは349である。

共通ペアでの第1段の量は次のとおりである。
信頼区間は、共通ペアをペア番号単位で10,000回復元抽出する対応のあるブートストラップのパーセンタイル法による。

| 量 | 点推定 | 95%信頼区間 |
|---|---:|---|
| E0（Cλ対S0） | −146.5 | −174.0〜−121.2 |
| E1（Cλ\*対S0） | −157.3 | −184.9〜−130.8 |
| R = E1 − E0 | −10.8 | −45.9〜+25.4 |
| D1 = E1 − 0.5·E0 | −84.0 | −112.8〜−55.6 |

回復率R/|E0|の点推定は−0.07である。

## 結論

Rの95%信頼区間は0を含むので、第1段の振り分けは「回復を示せない」である。
探索係数をCλに合わせて調整し直しても、Cλの約150 Eloの負けは縮まなかった。
D1の信頼区間の上限も−55.6であり、負けの半分以上が縮む場合とは両立しない。
E1の95%信頼区間の下限は0を上回らないので、Cλ\*の採否の測定には進まない。
設計書の判定表のとおり、第2段の対照は測らずに計画を終える。

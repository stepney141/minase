# eval-coadaptation-before-stc400

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)のフェーズ2として、混合比λ=1.0で学び直したPSTを載せた候補Cλを、探索係数を調整し直す前の状態でS0と400ペア対局させ、負けの大きさE0を測る。
この測定は第0段の振り分けと、第1段以降の回復量の基準に使い、採否の根拠にはしない。

## コマンドライン

`cargo run`と同じmasterのビルド（`target/release/match_runner`）を直接実行した。

```console
target/release/match_runner \
  --run-dir data/matches/eval-coadaptation-before-stc400 --seed 13000000 \
  --candidate commit:5f8f4256283a94cb99e45fc94be86bbd9610a653 \
  --baseline commit:4fb158284648c26be039544385359e5704e13cb5 \
  --each time=10000+100 --concurrency 16 elo --pairs 400
```

判定の計算は、[判定スクリプト](eval-coadaptation-analysis/analyze.py)で行い、出力を[stage0.json](eval-coadaptation-analysis/stage0.json)に保存した。

```console
python3 docs/measurements/eval-coadaptation-analysis/analyze.py stage0 \
  --before data/matches/eval-coadaptation-before-stc400
```

## エンジン

候補はCλ（ブランチ`eval-search-coadaptation`の`5f8f425`、バイナリのSHA-256 `ba411535…`）であり、Mの`crates/minase/nets/pst.bin`を[Pλ](eval-coadaptation-lambda100-training.md)へ差し替えたものである。
基準はS0、すなわちM（master `4fb1582`、バイナリのSHA-256 `5cb17c63…`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）、`Threads`は両者1、`USI_Hash`は両者256MBである。
runnerのSHA-256は`616751c0…`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB）、同時対局数は16である。
2026年10月4日21時44分から22時55分に実施し、経過時間は4,211秒だった。
同じ時間帯に他の対局や学習は走っていなかった。

## 結果

| 項目 | 値 |
|---|---|
| ペア数 | 400（完走375、手数上限による破棄25） |
| ペンタノミアル度数 | [175, 12, 159, 3, 26] |
| 正規化得点 | 0.2953 |
| Elo（match_runnerの正規近似） | −151.1（95%信頼区間 −177.9〜−125.8） |
| E0（ブートストラップ） | −151.1（95%信頼区間 −177.5〜−125.1） |
| 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手 | すべて0件 |

破棄されたペアの番号は、4、5、24、74、78、91、94、132、133、134、138、162、165、168、200、216、236、237、242、278、281、328、335、338、365である。
ブートストラップは、完走375ペアをペア番号単位で10,000回復元抽出し、パーセンタイル法で区間を求めた。

## 結論

E0の95%信頼区間の上限−125.1 Eloは−50 Eloを下回るので、第0段の振り分けは「負けが大きい」である。
現行の採用PSTから学び直したPλも、過去のλ=1.0の再学習（約−164 Elo）と同程度の大きな負けを示した。
設計書のとおり、第1段のCλの調整セッションへ進む。

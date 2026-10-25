# 線形経路を駒価値で初期化したネットの非劣性のLTC

## 目的

[線形経路を駒価値で初期化する計画](../../plans/archived/nnue-material-linear-init.md)のフェーズ5の採否を決める測定として、[STC](nnue-material-linear-init-noninferiority-stc.md)を通った新しいネットを、masterを候補とする長時間の時間制御（LTC）のGSPRTにかけ、新しいネットがmasterより約10 Elo以上弱い可能性を退けられるかを調べる。

## コマンドライン

本体のリポジトリのルートで、STCと同じrunner（`data/nnue-material-linear-init/minase-master`、SHA-256 `bd0632e4e85237d443f171b647d0b4fef377a243e507d8937e0117681d4d8133`）で実行した。

```console
data/nnue-material-linear-init/minase-master match run \
  --run-dir data/matches/nnue-material-linear-init-noninferiority-ltc --seed 6500000000 \
  --candidate commit:b8bebfb3 --baseline commit:6d47da2a \
  --each time=60000+200 --concurrency 16 gsprt
```

仮説は既定のとおり、H0をelo=0、H1をelo=10とし、誤り率はα=β=0.05、ペア数の上限は既定の100,000である。
候補にmaster、基準に新しいネットを置いたので、`H0`は「masterが10 Elo強い」という仮説を誤り率5%で退けたこと、すなわち新しいネットが約10 Elo以上弱い可能性を退けたことを意味する。
シードは、STCの基本シード6,400,000,000から上限ペア数より離れた値を選んだ。

## エンジン

| 役 | コミット | 実行ファイルのSHA-256 | 埋め込みのネット |
|---|---|---|---|
| 候補 | `b8bebfb370a6844f060eab1b7abb895d58c5345d`（master） | `4733e7e9f520e992c262a8f5d968706bce8d828e529ba0c746220e6f7c3c11aa` | 現行のネット（`faa5fbe2…`、線形経路の初期値は学習済みPST） |
| 基準 | `6d47da2a9663918f8992290834d9c47e695de98f` | `111b9f66765a9043b1d553de1792b501cd0ea563eeff04d46dacff34cfdec272` | 新しいネット（`5257f374…`、線形経路の初期値は探索用駒価値） |

規則セットは`engine-default`（L0,P0,R1,E0）である。

## 環境

測定機（Intel Core Ultra 7 265KF、物理20コア、論理20コア、実メモリ33,218,797,568バイト）で、2026年10月25日16時25分から10月26日4時2分まで行った。
候補と基準のワーカー数（`Threads`）はともに1、`USI_Hash`はともに256 MB、同時対局数は16、時間制御は持ち時間60秒、1手0.2秒加算である。
測定の間、同じ機械でlishogiのBotも対局していた。
1ペアあたりの時間は約26秒だった。

## 結果

| 項目 | 値 |
|---|---|
| ペンタノミアル度数（候補のペア得点0、0.5、1、1.5、2） | [322, 72, 816, 63, 306] |
| 有効ペア数 / 実行ペア数 | 1,579 / 1,605 |
| 破棄ペア数（手数上限4,096） | 26 |
| LLR | −2.964 |
| 判定 | `H0` |
| エンジン異常（不正着手、クラッシュ、応答タイムアウト） | 0、0、0 |
| `time_forfeits` | 0 |
| `rejected_moves` | 0 |
| 経過時間（`summary.json`の`active_wall_time_ns`） | 41,784秒（11時間36分） |

ペンタノミアル度数は、候補の負け越し側（0と0.5）が394、勝ち越し側（1.5と2）が369である。
LLRは、931ペアで−2.02まで下がった後に1,208ペアで−0.90まで戻り、1,579ペアで境界−2.944を越えた。

## 結論

LTCは`H0`であり、エンジン異常、時間切れ、および拒否着手は0件だったので、設計書の採否の規則により新しいネットを採用とした。

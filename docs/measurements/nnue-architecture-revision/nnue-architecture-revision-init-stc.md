# nnue-architecture-revision-init-stc

## 目的

第1層の初期値を小さくして学習したネットを候補、幅128の直接内積のネットを基準とする短時間の時間制御（STC）のGSPRTで、[設計書](../../plans/archived/nnue-architecture-revision.md)の切り分けの最初の候補が判定を通るかを調べる。

## コマンドライン

対局は、候補のコミットからビルドしたrunner（`data/nnue-architecture-revision/minase-runner-init`、SHA-256 `a043b361af35b971eee1d80d08b6ab0819bedd0b974a0dfb3b52cab173febd68`）で、本体のリポジトリのルートから実行した。

```console
minase-runner-init match run --run-dir data/matches/nnue-architecture-revision-init-stc --seed 20261113000000 \
  --candidate commit:9af68a00dfe397ecfc680e74972c58db20d452e5 \
  --baseline commit:ee1d6e08f294a5bfdc41c6737d445e0944928f77 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

| 役 | コミット | 実行ファイルのSHA-256 | 埋め込みのネット |
|---|---|---|---|
| 候補 | `9af68a00dfe397ecfc680e74972c58db20d452e5` | `b9e0cc2ad5aec529a81cb8856170e572e5e481433c3af7c0198a30c9cde071f5` | 幅256（`987f060d…`） |
| 基準 | `ee1d6e08f294a5bfdc41c6737d445e0944928f77` | `e6289d55c2874d46a71a8fce0f03e5b35338c71d9643ff19b0f0079d37965f12` | 幅128（`5257f374…`） |

規則セットは`engine-default`（L0,P0,R1,E0）である。
候補の学習は[学習の記録](nnue-architecture-revision-init-training.md)にある。

## 環境

2026年11月4日に測定機（Intel Core Ultra 7 265KF、物理20コア、論理20コア）で行った。
候補と基準のワーカー数（`Threads`）はともに1、`USI_Hash`はともに256 MB、同時対局数は16、時間制御は持ち時間10秒、1手0.1秒加算である。
同じ機械でlishogiのBotと、別の作業の探索が動いていた時間がある。

## 結果

| 項目 | 値 |
|---|---|
| ペンタノミアル度数（候補のペア得点0、0.5、1、1.5、2） | 79、16、158、10、43 |
| 有効ペア数 | 306 |
| 破棄ペア数 | 3 |
| LLR | −2.950 |
| 判定 | `H0` |
| 候補の得点率 | 43.6% |
| 得点率から求めたElo（95%信頼区間） | −44.5（−69.9〜−19.6） |
| 異常（不正着手、クラッシュ、タイムアウト、時間切れ、拒否着手） | いずれも0件 |
| 経過時間 | 2,869.0秒 |

Eloの大きさは逐次検定で止めた標本の値であり、採否の根拠には使わない。

参考として、保存された342ペアの棋譜から[配置の距離](nnue-architecture-revision-init-stc/placement_single.out)を求めた。
Mは候補側で0.4474、基準側で0.4548である。
20〜99手の5配置の割合は次のとおりである。

| 配置 | 候補側 | 基準側 |
|---|---:|---:|
| 奔王が1〜2段 | 79.0% | 86.0% |
| 龍馬が1〜2段 | 65.0% | 68.2% |
| 醉象が3〜5筋 | 70.2% | 62.8% |
| 金将が3段以上 | 38.1% | 43.4% |
| 麒麟が右端2筋 | 41.7% | 34.2% |

## 結論

`H0`であり、この候補も基準より10 Elo以上強いとは言えない。
検証損失は基準より3.2×10⁻⁵低いが、棋力の改善には結びつかず、最初の候補の約−59 Eloとの差は信頼区間の幅の中にある。

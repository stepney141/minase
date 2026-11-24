# 世代反復の第3世代の判定（STC）

## 目的

[世代反復の設計書](../../plans/nnue-generation-iteration.md)の「世代の手順」に従い、[第3世代の学習](nnue-generation-iteration-gen3-training.md)で作ったNNUE（着手ごとに差分で更新できる小さなニューラルネットの評価関数）のネットを候補、暫定最良世代である第2世代のネットを対照として、短時間条件（STC、持ち時間10秒、1手0.1秒加算）の一般化逐次確率比検定（GSPRT）を行う。
このGSPRTは、候補が対照より0 Elo強いという仮説`H0`と、10 Elo強いという仮説`H1`のどちらに近いかを、対数尤度比（LLR）で判定する。
`H1`なら第3世代を、以後の世代の対照に使う暫定最良世代にする。
第3世代は、学習の条件を標準レシピv4に替えて学んだ最初の世代である。

## コマンドライン

本体のリポジトリのルートで、第1世代の判定と同じ[`run-stc.sh`](nnue-generation-iteration-gen1-stc/run-stc.sh)により、nice値19、メモリ上限12 GiB、スワップなしの`systemd-run`のサービスとして実行した。

```console
minase match run \
  --run-dir data/matches/nnue-generation-iteration-gen3-stc --seed 20261303000000 \
  --candidate commit:f2eb705febe317229addc561ff8a0838c545e2eb \
  --baseline commit:f2eb705febe317229addc561ff8a0838c545e2eb \
  --candidate-option EvalFile=<第3世代のネット> --baseline-option EvalFile=<第2世代のネット> \
  --rules lishogi --each time=10000+100 --concurrency 10 gsprt --max-pairs 3000
```

## エンジン

| 側 | コミット | ネットのSHA-256（`/mnt/synology_nas/minase/nnue/nets/`に同じ名前で置いてある） |
|---|---|---|
| 候補（第3世代） | `f2eb705febe317229addc561ff8a0838c545e2eb` | `cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244` |
| 対照（第2世代） | `f2eb705febe317229addc561ff8a0838c545e2eb` | `736227caec4d391a73595b47be877dccb06a570baae18e0ca0030b2c0dba0129` |

両側のコミットは、世代反復の全世代で共通して使うコミット（設計書の「反復のコミット」）であり、エンジンの実行ファイルは同じ1つのビルド（SHA-256は`73e8ad00…`）である。
対局の実行記録`manifest.json`の`network_sha256`は、候補が世代の記録`gen-3/generation.json`の値と、対照が`gen-2/generation.json`の値と一致した。
規則セットは、オンライン対局サイトlishogiの実装に近い`lishogi`（規則コード`L1,L2,P0,P3,R1,E1,E3`）である。

## 環境

2026年11月24日11時34分から11時55分（UTC）まで、作業機（Intel Core Ultra 7 265KF、物理コア20、論理コア20、メモリ30 GiB）で行った。
対局ツールは同じコミットの`minase`（SHA-256は`42bd5b34…`）で、起動は1回である。
エンジンの設定は、両側ともスレッド数（`Threads`）が1、置換表の容量（`USI_Hash`）が256 MBであり、同時対局数は10である。
同じ作業機で、lishogiのBotのエンジンが動いていたので、第1世代と第2世代の判定と同じ同時対局数にした。

## 結果

```text
summary: mode=gsprt pairs=110 valid_pairs=110 discarded_pairs=0
pentanomial: [2, 0, 19, 18, 71]
llr: 2.9700794016
decision: H1
engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 rejected_moves=0
elapsed: 1209.590776 s
```

判定は`H1`であり、第3世代は第2世代より有意に強い。
`pentanomial`（先後を入れ替えた2局を1ペアとした、ペアごとの5通りの結果の数）は候補から見た値で、2局とも負けたペアが2、1敗1分が0、1勝1敗か2分が19、1勝1分が18、2局とも勝ったペアが71である。
110ペアの得点率は0.855で、Eloに直すと第3世代が+308（95%信頼区間+255〜+375）である。
220局の終局理由は、王駒の捕獲181局、裸玉21局、千日手18局である。
破棄したペア、エンジンの異常、時間切れ、および拒否着手は0件だった。

対局中の候補の側の駒の配置を集計した結果は、次のとおりである（[出力](nnue-generation-iteration-gen3-stc/placement-nnue-generation-iteration-gen3-stc.out)）。
配置の距離Mは、全21駒種の配置の分布が、人間の上位者、既存の中将棋エンジンHaChu、およびAlphaZero型のネットの配置からどれだけ離れているかを1つにまとめた値である（定義は[結果回帰の設計書](../../plans/archived/nnue-outcome-regression.md#配置の距離)）。

| 項目 | 第3世代 | 第2世代（[判定の記録](nnue-generation-iteration-gen2-stc.md)） | 第1世代（[判定の記録](nnue-generation-iteration-gen1-stc.md)） |
|---|---:|---:|---:|
| M | 0.2832 | 0.2721 | 0.2650 |
| 奔王が1〜2段 | 7.5% | 5.7% | 3.9% |
| 龍馬が1〜2段 | 8.6% | 10.4% | 10.5% |
| 醉象が3〜5筋 | 2.9% | 1.2% | 1.7% |
| 金将が3段以上 | 2.2% | 2.2% | 2.1% |
| 麒麟が右端2筋 | 0.2% | 0.1% | 0.2% |

表のMより下の5行は、20〜99手の局面で、その位置に駒がある割合である。
Mは世代ごとに約0.01ずつ大きくなっているが、配置の偏りが問題になったネットの0.46前後（[結果回帰の配置の記録](../nnue-outcome-regression/nnue-outcome-regression-placement-stc.md)）より小さい。
3つの世代は対照が違うので、世代のあいだの推移を見る目安として読む。
この値は世代の判定に使わない。

1局ごとの集計（[出力](nnue-generation-iteration-gen3-stc/games-nnue-generation-iteration-gen3-stc.out)、110ペアの220局）では、平均の探索深さは候補が17.00、対照が17.01だった。

## 結論

第3世代は、第2世代との判定が`H1`となり、暫定最良世代になった。

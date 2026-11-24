# 世代反復の第3世代とmasterの比較（STC）

## 目的

[世代反復の設計書](../../plans/archived/nnue-generation-iteration.md)の第3世代を終えた時点で、利用者が2026年11月24日に、世代反復をいったん止めて、第3世代を第2世代およびmasterと短時間条件（STC、持ち時間10秒、1手0.1秒加算）で比べると決めた。
本書は、このうちmasterとの比較の記録である。第2世代との比較は[第3世代の判定](nnue-generation-iteration-gen3-stc.md)にある。
候補は、[第3世代の学習](nnue-generation-iteration-gen3-training.md)で作ったNNUE（着手ごとに差分で更新できる小さなニューラルネットの評価関数）のネットである。
対照は、masterのエンジンに、測定の時点で探索の測定の標準だったネット（SHA-256は`37054c9e…`、以下「標準のネット」）を渡したものである。この測定の後に、利用者の決定で、標準は第3世代のネットに替わった（[NNUEの評価関数の手引き](../../guides/evaluation-nnue.md)の「探索の測定に使う標準のネット」）。
比較には、世代の判定と同じ一般化逐次確率比検定（GSPRT）を使う。
このGSPRTは、候補が対照より0 Elo強いという仮説`H0`と、10 Elo強いという仮説`H1`のどちらに近いかを、対数尤度比（LLR）で判定する。

この比較は、設計書の「最終候補の採否」が定める採否の測定ではない。
設計書の採否は、世代反復を始める前のmasterのネットを埋め込んだ旧基準を対照とし、配置の距離、STC、および長時間条件（LTC）の順に測る。

## コマンドライン

本体のリポジトリのルートで、[`run-stc.sh`](nnue-generation-iteration-gen3-master-stc/run-stc.sh)により、nice値19、メモリ上限12 GiB、スワップなしの`systemd-run`のサービスとして実行した。

```console
minase match run \
  --run-dir data/matches/nnue-generation-iteration-gen3-master-stc --seed 20261320000000 \
  --candidate commit:f2eb705febe317229addc561ff8a0838c545e2eb \
  --baseline commit:524ef376420ffc929c653eb1aeb3a5502f498ebb \
  --candidate-option EvalFile=<第3世代のネット> --baseline-option EvalFile=<標準のネット> \
  --rules lishogi --each time=10000+100 --concurrency 10 gsprt --max-pairs 3000
```

## エンジン

| 側 | コミット | エンジンの実行ファイルのSHA-256 | ネットのSHA-256（`/mnt/synology_nas/minase/nnue/nets/`に同じ名前で置いてある） |
|---|---|---|---|
| 候補（第3世代） | `f2eb705febe317229addc561ff8a0838c545e2eb` | `73e8ad00…` | `cbbccf6f5ee7097fd890086d2cf4bca72ed969da28353cb0c4a2f4d781acc244` |
| 対照（master） | `524ef376420ffc929c653eb1aeb3a5502f498ebb` | `70a013f7…` | `37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea` |

候補のコミットは、世代反復の全世代で共通して使うコミット（設計書の「反復のコミット」）である。
対照のコミットは、比較を決めた時点のmasterの先頭である。
測定を起動した時点のmasterの先頭は`481110ca`に進んでいたが、`524ef376`との差に`crates/`、`Cargo.toml`、および`Cargo.lock`の変更はない。
2つのコミットのあいだの`crates/minase/src`と`crates/minase-core/src`の差は8ファイルの20行であり、中身はRust 1.100のclippyの指摘への対応と、局面の表示に使う駒の名前の変更である。探索、評価、および時間管理の変更は含まない。
対局の実行記録`manifest.json`の`network_sha256`は、候補が世代の記録`gen-3/generation.json`の値と、対照が標準のネットのファイル全体のSHA-256と一致した。

2本のネットは、どちらも王の位置を使わない入力P型である。
標準のネットは、中断した世代反復の第2世代の教師37本（訓練96,462,343局面）から、日本中将棋連盟の規則セット`JCSA`を学習器に渡して学んだものである（[学習の記録](../nnue-tatara-framework-and-halfkp-vm/nnue-tatara-framework-and-halfkp-vm-training.md)）。
第3世代のネットは、教師も学習器の引数も`lishogi`である。
本書の対局の規則セットは、オンライン対局サイトlishogiの実装に近い`lishogi`（規則コード`L1,L2,P0,P3,R1,E1,E3`）なので、対照のネットは学んだ規則と違う規則で指している。

## 環境

2026年11月24日11時55分から12時15分（UTC）まで、作業機（Intel Core Ultra 7 265KF、物理コア20、論理コア20、メモリ30 GiB）で行った。
対局ツールは反復のコミットの`minase`（SHA-256は`42bd5b34…`）で、起動は1回である。
エンジンの設定は、両側ともスレッド数（`Threads`）が1、置換表の容量（`USI_Hash`）が256 MBであり、同時対局数は10である。
同じ作業機で、lishogiのBotのエンジンが動いていたので、世代の判定と同じ同時対局数にした。
シード20,261,320,000,000は、世代の判定のシード（20,261,301,000,000〜20,261,303,000,000）と設計書の「最終候補の採否」のシード（20,261,310,000,000〜20,261,310,500,000）のどちらからも、上限のペア数3,000より大きく離れている。

## 結果

```text
summary: mode=gsprt pairs=109 valid_pairs=109 discarded_pairs=0
pentanomial: [2, 0, 17, 2, 88]
llr: 2.9539711011
decision: H1
engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 rejected_moves=0
elapsed: 1114.986870 s
```

判定は`H1`であり、第3世代のネットを渡した反復のコミットのエンジンは、標準のネットを渡したmasterのエンジンより、`lishogi`のSTCで有意に強い。
`pentanomial`（先後を入れ替えた2局を1ペアとした、ペアごとの5通りの結果の数）は候補から見た値で、2局とも負けたペアが2、1敗1分が0、1勝1敗か2分が17、1勝1分が2、2局とも勝ったペアが88である。
109ペアの得点率は0.899で、Eloに直すと候補が+380（95%信頼区間+312〜+479）である。
破棄したペア、エンジンの異常、時間切れ、および拒否着手は0件だった。

以下の集計は、判定が出た時点で対局中であり、その後に終わった1ペアを含む110ペアの220局で行った。
終局理由は、王駒の捕獲196局、裸玉22局、千日手2局である。
1局ごとの集計（[出力](nnue-generation-iteration-gen3-master-stc/games-nnue-generation-iteration-gen3-master-stc.out)）では、候補の勝ちが197局、対照の勝ちが21局、引き分けが2局であり、平均の探索深さは候補が16.21、対照が15.76だった。

対局中の候補の側の駒の配置を集計した結果は、次のとおりである（[出力](nnue-generation-iteration-gen3-master-stc/placement-nnue-generation-iteration-gen3-master-stc.out)）。
配置の距離Mは、全21駒種の配置の分布が、人間の上位者、既存の中将棋エンジンHaChu、およびAlphaZero型のネットの配置からどれだけ離れているかを1つにまとめた値である（定義は[結果回帰の設計書](../../plans/archived/nnue-outcome-regression.md#配置の距離)）。

| 項目 | masterとの対局 | 第2世代との対局（[判定の記録](nnue-generation-iteration-gen3-stc.md)） |
|---|---:|---:|
| M | 0.2639 | 0.2832 |
| 奔王が1〜2段 | 9.2% | 7.5% |
| 龍馬が1〜2段 | 10.9% | 8.6% |
| 醉象が3〜5筋 | 3.6% | 2.9% |
| 金将が3段以上 | 2.8% | 2.2% |
| 麒麟が右端2筋 | 0.1% | 0.2% |

表のMより下の5行は、20〜99手の局面で、その位置に駒がある割合である。
同じ第3世代のネットでも、対照が違うとMは0.02ほど変わる。
どちらの値も、配置の偏りが問題になったネットの0.46前後（[結果回帰の配置の記録](../nnue-outcome-regression/nnue-outcome-regression-placement-stc.md)）より小さい。

## 結論

第3世代のネットは、masterの標準のネットに対して`lishogi`のSTCで`H1`となった。
対照のネットは`JCSA`で学んだものを`lishogi`で指させているので、この結果は`lishogi`での強さの比較であり、`JCSA`での強さは測っていない。
設計書の採否は、旧基準を対照とする配置の距離、STC、およびLTCで別に決める。

# 載せ替えの前と後のA型のSTC

## 目的

[tatara化とHalfKP_vmの設計書](../../plans/archived/nnue-tatara-framework-and-halfkp-vm.md)の「載せ替えの確認」の節に従い、王の位置を使わない入力特徴集合A型のNNUEについて、tataraのLayerStackへ載せ替える前の版を候補、載せ替えた後の版を基準として、短時間条件（STC）の一般化逐次確率比検定（GSPRT）を行う。
このGSPRTは、候補（前のA型）が基準（後のA型）より0 Elo強いという仮説`H0`と、10 Elo強いという仮説`H1`のどちらに近いかを判定する。
設計書は、STCと長時間条件（LTC）がどちらも`H0`なら載せ替えを採り、どちらかが`H1`（前のA型が強い）なら設計書を待機中にして利用者へ報告すると定めている。

## エンジンとネット

| 側 | コミット | ネット（`/mnt/synology_nas/minase/nnue/nets/`） |
|---|---|---|
| 候補（前のA型） | `a48f03aa` | `fc86cce6b2f60c357061222fd54afdd62a5a88330b0a9207c93ec0119622ee71.bin` |
| 基準（後のA型） | `8b853b66` | `37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea.bin` |

2本のネットは[載せ替えの確認の学習](nnue-tatara-framework-and-halfkp-vm-training.md)で同じ教師から学んだものである。
`8b853b66`は設計書の作業ブランチ`nnue-tatara-framework-and-halfkp-vm`の測定時の先頭であり、ネットを学んだ学習器のコミット`8389b160`からの差分は学習器だけで、エンジンのコードは同じである。

## コマンドライン

本体のリポジトリのルートで、`8b853b66`に固定したworktreeの`minase`を対局ツールとして、nice値19、メモリ上限12 GiB、スワップなしの`systemd-run`のサービスで実行した。

```console
minase match run \
  --run-dir data/matches/nnue-tatara-framework-and-halfkp-vm-stc --seed 20261118000000 \
  --candidate commit:a48f03aa --baseline commit:8b853b66 \
  --candidate-option EvalFile=<前のネット> --baseline-option EvalFile=<後のネット> \
  --rules JCSA --each time=10000+100 --concurrency 7 gsprt --max-pairs 3000
```

## 環境

2026年11月18日21時30分から22時52分まで、測定機（Intel Core Ultra 7 265KF、物理コア20、論理コア20、メモリ30 GiB）で行った。
対局ツールは`minase` 2.6.0（実行ファイルのSHA-256は`e7eea73d…`）で、起動は1回である。
両側とも`Threads`は1、`USI_Hash`は256 MB、同時対局数は7である。
同時対局数は、lishogiのBotのエンジン（各4スレッド）と、並行して動かしたHalfKP_vmの学習（学習のスレッド4）の分を空けて決めた。
HalfKP_vmの学習は21時30分から22時29分ごろまでと、22時50分から動いていた。

## 結果

```text
summary: mode=gsprt pairs=284 valid_pairs=281 discarded_pairs=3
pentanomial: [39, 5, 138, 12, 87]
llr: 2.9522952180
decision: H1
engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 rejected_moves=0
elapsed: 4794.912556 s
```

判定は`H1`であり、前のA型が後のA型より有意に強い。
`pentanomial`（ペアごとの5通りの結果の数）は候補から見た値で、2局とも負けたペアが39、1敗1分が5、1勝1敗か2分が138、1勝1分が12、2局とも勝ったペアが87である。
有効な281ペアの得点率は0.5916で、Eloに直すと前のA型が+64.4（95%信頼区間+37.2〜+92.4）である。
捨てた3ペア（ペア番号166、168、277）は、どれも2局目が手数の上限4,096手に達して打ち切られた。
有効な562局の終局理由は、詰み540局、千日手18局、駒枯れ4局である。
エンジンの異常、時間切れ、および拒否着手は0件だった。

## 判断

設計書の規則により、LTCは行わず、設計書を待機中にして利用者へ報告した。

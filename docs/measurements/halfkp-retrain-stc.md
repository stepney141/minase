# 現行の学習器で再学習したHalfKP-hm-b8と現行のP型のSTC

## 目的

HalfKP-hm-b8を、現行の埋め込みネット（P型、D0b）と同じ学習コマンドから入力特徴集合の指定だけを変えて再学習し、短時間条件（STC）のGSPRTでmasterと比べて、[初回の比較](nnue-replacement-features-stc.md)の大差が学習条件の古さによるものかを確かめる。

## コマンドライン

学習は、測定機で別のSPSAが動いていたので、優先度19、CPUスレッド4本、メモリ上限12 GBで行った（`data/halfkp-retrain/train.sh`）。
学習器はmaster `26c52daa`でビルドした`data/halfkp-retrain/minase-trainer`（SHA-256 `431d2b5b…`）である。
引数は、[D0bの学習](nnue-hidden-layer-training.md)と比べて`--feature-set`と`--threads`だけが異なる。

```console
systemd-run --user --scope -q -p MemoryMax=12G -p MemorySwapMax=0 \
  nice -n 19 data/halfkp-retrain/minase-trainer nnue train --feature-set HalfKP-hm-b8 --width 128 \
  --data data/nnue-lr-schedule-gen/generated-*.bin \
  --init-pst data/pst-fm-joint-training/j75/pst-float.npz \
  --base crates/minase/nets/pst.bin \
  --lambda 1.0 --teacher-cp-per-logit calibrate \
  --lookahead-gamma 0.9 --lookahead-plies 40 \
  --batch 16384 --epochs 50 --patience 5 --seed 1 --threads 4 --device cuda \
  --lr-schedule cosine:3:0.1 --head direct-screlu \
  --output-dir data/halfkp-retrain/train
```

STCは、ryzen0の`/var/tmp/minase-halfkp`（masterと候補のブランチをgit bundleで移したクローン）から実行した。

```console
target/release/minase match run --run-dir data/matches/halfkp-retrain-stc --seed 3300000000 \
  --candidate commit:ae1b384c14c9395855cf388f01c4899bc08422b7 \
  --baseline commit:26c52daae1e14dbe819c8ca599d9df12d78f5e75 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

- 候補: `ae1b384c`（ブランチ`halfkp-retrain`）。master `26c52daa`に対し、`ActiveFeatureSet`を`HalfKpHmB8`に変え、`crates/minase/nets/nnue.bin`を今回学習したネット（SHA-256 `1af4c8f9…`）へ差し替えただけのコミットである。バイナリのSHA-256は`293b1f20…`。
- 基準: master `26c52daa`。埋め込みのネットは補助損失なしのP型（D0b、SHA-256 `046be0e6…`）。バイナリのSHA-256は`8be8e921…`。
- 規則セットは`L0,P0,R1,E0`、`USI_Hash`は両者256 MB、`Threads`は1。
- D0bの学習以後に学習器へ入った改名などの変更は、それぞれの照合記録では計算値を変えないものであり、`--buckets 1`の学習はD0bの学習器とビット単位で同じ結果を出す。ただし、同じ学習器でP型を学び直してD0bと一致することは、本測定では確かめていない。

## 環境

学習は測定機（Intel Core Ultra 7 265KF、20コア、メモリ30 GB、RTX 3060 Ti）で、2026年10月17日10時14分から10時48分に行った。同じ機械で、別のSPSA（同時対局数16）とlishogiのBotが動いていた。

STCは、ryzen0（AMD Ryzen Threadripper 2990WX、物理32コア、論理64コア、メモリ125 GB）で行った。測定機とは別の機械であり、ほかの負荷はなかった。
runnerはmaster `26c52daa`のビルド（SHA-256 `f85a95b0…`）である。
runnerはこのCPUの物理コア数を8と判定したので、自動で決まった同時対局数は7だった。

## 結果

### 学習

| 項目 | HalfKP-hm-b8（今回） | P型 D0b（参考） |
|---|---:|---:|
| エポック0の検証損失 | 0.4570753 | 0.4570753 |
| 最良の検証損失（最良エポック） | 0.4500218（1） | 0.4488929（2） |
| 停止エポックと停止理由 | 6、早期終了 | 7、早期終了 |
| エポック1の訓練損失 | 0.4467095 | 0.4475812 |
| 停止エポックの訓練損失 | 0.4436599 | 0.4460445 |
| 活性化の飽和率（acc≥1、最良エポック） | 26.1% | 19.0% |
| 量子化誤差（平均 / 最大、センチポーン） | 10.74 / 94.7 | 13.61 / 99.7 |
| 符号反転の割合 | 2.25% | 1.66% |
| 1エポックの学習時間 | 約215秒 | 73〜78秒 |
| 前処理から書き出しまでの経過時間 | 2,043秒 | 1,103秒 |

HalfKP-hm-b8の検証損失はエポック1で最良になり、その後は訓練損失が下がり続ける一方で単調に悪化した。
最良の検証損失は、D0bより0.0011高い。
1エポックの学習時間は、CPUスレッドを4本に絞り、SPSAと同居したため、D0bの記録と比べられない。

### STC

| 項目 | 値 |
|---|---:|
| 有効ペア数 | 87（破棄0） |
| ペンタノミアル度数（候補の得点0、0.5、1、1.5、2） | [66, 0, 21, 0, 0] |
| LLR | −2.540 |
| 判定 | `pending`（87ペアで中止） |
| 異常件数 | 不正着手、クラッシュ、応答タイムアウト、時間切れ、拒否着手のいずれも0（174局すべてが詰みで終局） |
| 累計実行時間 | 1,411秒（`summary.json`の`active_wall_time_ns`） |

候補は87ペアで1ペアも勝ち越さず、66ペアを両局とも失った。
得点率は約0.12であり、Eloに換算するとおよそ−340に相当する。
LLRがH0の境界（−2.944）に近づいた時点で、利用者の判断により測定を中止した。
中止はrunnerのプロセスへSIGINTを送って行ったが、`summary.json`は`invocation_active`が`true`のまま更新されなかったので、強制終了した測定として扱う。
実行ディレクトリはrsyncで測定機の`data/matches/halfkp-retrain-stc/`へ転送してある。

## 結論

現行の学習器で再学習しても、HalfKP-hm-b8は現行のP型に大差で負け、初回の比較での負けは学習条件の古さだけでは説明できない。

# 第3期の高速化 段階1の1つ目の測定単位（要素積とL1）の採否

## 目的

[第3期の設計書](../../plans/movegen-speedup-3.md)の段階1の1つ目の測定単位（NNUEの後段の要素積とL1を16ビットで計算する書き換え）を、親の基準と比べ、探索の結果が一致し、bench深さ16で速くなり、教師生成2局で0.5%を超えて遅くならないかを判定する。

## コマンドライン

手順と組数は[準備の記録](movegen-speedup-3-baseline.md)のとおりである。
探索の結果の一致は`verify_identical.py check`で、サイクル数の対比較は`measure.sh`と`paired.py judge`で確かめた。

```console
python3 verify_identical.py check --binary data/ms3/bin/s1a-e68001ee --net data/ms3/net.bin \
  --cwd data/ms3/cwd --baseline baseline-outputs.json
measure.sh data/ms3/bin/base-6cd40868 data/ms3/bin/s1a-e68001ee 40 4 pairs.jsonl
python3 paired.py judge --results pairs.jsonl
opcode_counts.sh <バイナリ>
```

組ごとの値は[付属の結果](movegen-speedup-3-stage1-pair-l1/pairs.jsonl)にある。

## エンジン

候補はブランチ`movegen-speedup-3`のコミット`e68001ee`、親は基準のmasterのコミット`6cd40868`である。
規則セットは`bench`が`JCSA`、教師生成が`lishogi`であり、ネットは標準の第3世代のA型のネットである。
ビルドはリポジトリのrelease設定である。

## 環境

測定機と固定したCPUは[準備の記録](movegen-speedup-3-baseline.md)と同じである。
2026年11月27日17時56分から18時04分に、Botのコンテナを止めて測った。同じ機械で、対局の測定、SPSA、教師生成、およびビルドは動いていなかった。

## 結果

探索の結果は基準と一致した。`verify_identical.py check`は4種類の出力すべてで一致し（`IDENTICAL`）、対比較の全組でも、組の中の2つの実行の`bench`の局面別の行と教師のファイルは一致した。
`cargo fmt --all --check`、`cargo clippy --release --all-targets`、および`cargo test --release`（1,989件成功、失敗0件）も通った。

| 負荷 | 組数 | 対数比の平均 | 両側95%信頼区間 | 片側95%の上端 | 速度比（両側95%信頼区間） | 命令数 | 分岐予測ミス |
|---|---:|---:|---|---:|---|---:|---:|
| bench深さ16 | 40 | −0.05930 | −0.06033〜−0.05827 | −0.05845 | 1.0611（1.0600〜1.0622） | −7.23% | −2.09% |
| 教師生成2局 | 4 | −0.05844 | −0.05960〜−0.05728 | −0.05758 | 1.0602（1.0589〜1.0614） | −6.72% | −1.78% |

サイクル数の中央値は、bench深さ16が親219.5億、候補206.9億、教師生成2局が親1,013.8億、候補956.0億である。

`LayerStackNetwork::dense`の機械語の命令数は、親の726から候補の695に減った。命令名ごとの出現数は[親](movegen-speedup-3-stage1-pair-l1/dense-opcodes-6cd40868.txt)と[候補](movegen-speedup-3-stage1-pair-l1/dense-opcodes-e68001ee.txt)の付属の集計にある。
候補の要素積は、両視点とも256ビット幅の16ビットの積`vpmullw ymm`で計算され、端数の128ビット幅の`vpmullw xmm`が残る。
L1の主ループは、16ビットの積和`vpmaddwd`を128ビット幅で4命令並べ、1反復で32個の積を処理する。各出力の最後に`vpshufd`による水平加算が残る。
8ビットを32ビットへ広げる命令（`vpmovsxbd`と`vpmovzxbd`）は親の39個から19個に減り、残りはL2と出力層の`affine`のものである。
入力の配列`[0_i16; 1024]`は、関数の入口で`memset`により2,048バイトが0で埋められる。標準のネットが使うのは512要素（1,024バイト）である。

## 結論

段階1の1つ目の測定単位は、探索の結果が一致し、bench深さ16の両側95%信頼区間の上端が0より小さく、教師生成2局の片側95%の上端が0.5%の退行の値（0.00499）以下なので、採用する。

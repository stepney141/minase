# 第3期の高速化 段階1の2つ目の測定単位（L2と出力層）の採否

## 目的

[第3期の設計書](../../plans/archived/movegen-speedup-3.md)の段階1の2つ目の測定単位（NNUEの後段のL2を入力ごとに並べた16ビットの重みの対で計算し、出力層を同じ反復で計算する書き換え）を、1つ目の単位を採用した親と比べ、探索の結果が一致し、bench深さ16で速くなり、教師生成2局で0.5%を超えて遅くならないかを判定する。

## コマンドライン

手順と組数は[準備の記録](movegen-speedup-3-baseline.md)のとおりである。

```console
python3 verify_identical.py check --binary data/ms3/bin/s1b-49c58646 --net data/ms3/net.bin \
  --cwd data/ms3/cwd --baseline baseline-outputs.json
measure.sh data/ms3/bin/s1a-e68001ee data/ms3/bin/s1b-49c58646 40 4 pairs.jsonl
python3 paired.py judge --results pairs.jsonl
opcode_counts.sh data/ms3/bin/s1b-49c58646
```

組ごとの値は[付属の結果](movegen-speedup-3-stage1-l2/pairs.jsonl)にある。

## エンジン

候補はブランチ`movegen-speedup-3`のコミット`c427f551`と同じコードであり、測定のバイナリは、作業用の連鎖のコミット`49c58646`（`crates/`の内容は`c427f551`と同一）からビルドした。親は1つ目の単位のコミット`e68001ee`である。
規則セットは`bench`が`JCSA`、教師生成が`lishogi`であり、ネットは標準の第3世代のA型のネットである。
ビルドはリポジトリのrelease設定である。

## 環境

測定機と固定したCPUは[準備の記録](movegen-speedup-3-baseline.md)と同じである。
2026年11月27日18時04分から18時11分に、Botのコンテナを止めて測った。同じ機械で、対局の測定、SPSA、教師生成、およびビルドは動いていなかった。

## 結果

探索の結果は基準と一致した。`verify_identical.py check`は4種類の出力すべてで一致し、対比較の全組でも、組の中の2つの実行の`bench`の局面別の行と教師のファイルは一致した。
`cargo fmt --all --check`、`cargo clippy --release --all-targets -- -D warnings`、および`cargo test --release`（1,990件成功、失敗0件）も通った。

| 負荷 | 組数 | 対数比の平均 | 両側95%信頼区間 | 片側95%の上端 | 速度比（両側95%信頼区間） | 命令数 | 分岐予測ミス |
|---|---:|---:|---|---:|---|---:|---:|
| bench深さ16 | 40 | −0.03381 | −0.03483〜−0.03278 | −0.03295 | 1.0344（1.0333〜1.0354） | −5.56% | +0.73% |
| 教師生成2局 | 4 | −0.03545 | −0.03918〜−0.03172 | −0.03269 | 1.0361（1.0322〜1.0400） | −5.15% | +0.48% |

サイクル数の中央値は、bench深さ16が親206.8億、候補200.0億、教師生成2局が親956.8億、候補924.3億である。

`LayerStackNetwork::dense`の機械語の命令数は、親の695から候補の650に減った。命令名ごとの出現数は[付属の集計](movegen-speedup-3-stage1-l2/dense-opcodes-49c58646.txt)にある。
L2の入力の対の積は、128ビット幅の16ビットの積`vpmullw xmm`で計算される。出力層の積和は256ビット幅の`vpmaddwd ymm`4命令で計算される。
8ビットを32ビットへ広げる命令は、親の19個から`vpmovsxbd`の5個に減った。

## 結論

段階1の2つ目の測定単位は、探索の結果が一致し、bench深さ16の両側95%信頼区間の上端が0より小さく、教師生成2局の片側95%の上端が0.5%の退行の値（0.00499）以下なので、採用する。
2つの単位がともに採用されたので、試作Eと同じ組み合わせを1つの単位として判定する手順は行わない。

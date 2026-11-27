# 第3期の高速化 段階5の診断（残る負荷）

## 目的

[第3期の設計書](../../plans/archived/movegen-speedup-3.md)の段階5として、段階1から段階4の採否が決まった後の採用版で関数別の内訳を取り直し、NNUEの後段に残る時間、駒コードの復号の連鎖が残っている箇所、および通常駒の捕獲可能升の走査の3点を1回だけ調べ、探索の結果を変えずに省ける処理で、どちらかの負荷の探索時間の0.5%以上を占める項目を測定単位として起こすかを決める。

## コマンドライン

採用版（ブランチ`movegen-speedup-3`の`426dc065`と同じコード。作業用の連鎖のコミット`524e049d`）を、[再診断](movegen-speedup-3-rediagnosis.md)と同じくフレームポインタを残してビルドし、[付属のスクリプト](movegen-speedup-3-stage5-diagnosis/profile.sh)でbench深さ16と教師生成2局のサイクルと分岐予測ミスの標本を採った。
採取の周期は、サイクルがbench深さ16で500,009、教師生成2局で2,000,003、分岐予測ミスがbench深さ16で5,003、教師生成2局で20,011である。
集計は[教師生成の高速化の付属資料](../teacher-generation-speedup/teacher-generation-speedup-engine/)の`inline_prof.py`で行った。

```console
CARGO_PROFILE_RELEASE_DEBUG=1 CARGO_TARGET_DIR=target/prof \
  RUSTFLAGS="-C target-cpu=native -C llvm-args=-inline-threshold=1000 -C force-frame-pointers=yes" \
  cargo build --release --locked -p minase --bin minase
profile.sh data/ms3/bin/prof-524e049d data/ms3/prof
python3 inline_prof.py <記録> <バイナリ> --top 60
python3 inline_prof.py <記録> <バイナリ> --callees 'LayerStackDense>::evaluate$'
python3 inline_prof.py <記録> <バイナリ> --callers '^\[libc'
python3 inline_prof.py <記録> <バイナリ> --callers 'piece_at$|PieceCode>::color$|PieceCode>::is_promoted$|PieceCode>::kind$|movement_profile$'
python3 inline_prof.py <記録> <バイナリ> --tree 'capture_targets_without_special$' --tree-depth 3
```

集計は[benchの内訳](movegen-speedup-3-stage5-diagnosis/profile-bench16.txt)、[benchの呼び出し元と呼び出し先](movegen-speedup-3-stage5-diagnosis/profile-bench16-breakdown.txt)、[教師生成の内訳](movegen-speedup-3-stage5-diagnosis/profile-teacher.txt)、[教師生成の呼び出し元と呼び出し先](movegen-speedup-3-stage5-diagnosis/profile-teacher-breakdown.txt)、および分岐予測ミスの[bench](movegen-speedup-3-stage5-diagnosis/branch-misses-bench16.txt)と[教師生成](movegen-speedup-3-stage5-diagnosis/branch-misses-teacher.txt)の内訳にある。

NNUEの後段については、設計書の段階5の1点目に従い、採用版に対する独立の3つの変種を作り、リリースのビルドの`LayerStackNetwork::dense`の機械語を[付属のスクリプト](movegen-speedup-3-baseline/opcode_counts.sh)と`objdump -d -M intel`で調べた。
変種の差分と命令名ごとの出現数は[付属の資料](movegen-speedup-3-stage5-diagnosis/variants/)にある。
変種は、L1を隣接する2つの積の和を固定の個数ずつ作る形にした`l1-256`、L1の15出力の2乗の活性化を`i32`のレーンで計算する`act-i32`、およびL2の対の積を32ビットの積和の形で足す`l2-madd`である。
`l1-256`は、16要素の塊と`[i32; 8]`の部分和（試行1）、32要素の塊と`[i32; 16]`の部分和（試行2、`l1-256.patch`）、32要素の塊の両半分で`[i32; 8]`を共有する形（試行3）の3通りを試した。

## エンジン

対象は採用版のコードであり、規則セットは`bench`が`JCSA`、教師生成が`lishogi`、ネットは標準の第3世代のA型のネットである。
変種の機械語はリポジトリのrelease設定のビルドで数えた。

## 環境

測定機と固定したCPUは[準備の記録](movegen-speedup-3-baseline.md)と同じである。
標本は2026年11月27日18時40分から18時41分に、Botのコンテナを止めて採った。同じ機械で、対局の測定、SPSA、教師生成、およびビルドは動いていなかった。

## 結果

### NNUEの後段

後段`LayerStackNetwork::dense`は、探索時間のうちbench深さ16で9.2%、教師生成2局で9.3%を占める。段階1の前の[試作の記録](../adhoc/nnue-dense-safe-vectorization-20261127.md)の17.7%（bench深さ16）から下がった。
内訳は次のとおりである。

| 部分 | bench深さ16 | 教師生成2局 |
|---|---:|---:|
| L1の積和（`evaluate`の中のclosure） | 4.74% | 4.99% |
| `evaluate`自身（要素積の書き込み、L2、出力層） | 2.18% | 2.16% |
| 2乗の活性化`square_activation` | 1.34% | 1.30% |
| 線形の活性化`activation` | 0.35% | 0.32% |
| 要素積`pair` | 0.31% | 0.25% |
| `libc`の呼び出し（`evaluate_at`と`evaluate_accumulator`から） | 0.45% | 0.56% |

L1の主ループは、128ビット幅の`vpmaddwd xmm`4命令で1反復32個の積を処理し、各出力の最後に部分和の結合3命令と水平加算6命令が残る。
入力の配列`[0_i16; 1024]`は、関数の入口の`memset`で2,048バイトが0で埋められる。標準のネットが使うのは512要素（1,024バイト）である。`libc`の呼び出しの時間にはこの`memset`が含まれるが、`libc`の中の関数は区別できていない。
2乗の活性化は、15個それぞれをスカラーの64ビットの積`imul`で計算している。

変種の`dense`の命令数と判定は次のとおりである。

| 変種 | `dense`の命令数 | 機械語 | 判定 |
|---|---:|---|---|
| 採用版 | 650 | L1は32積を`vpmaddwd xmm`4命令 | ― |
| `l1-256`（試行2） | 736 | 15出力のループは32積を`vpmaddwd ymm`2命令とAVX-VNNIの`vpdpwssd ymm`2命令で処理し、水平縮約が33命令になる | 起こさない |
| `l1-256`（試行1、試行3） | 677、731 | 128ビット幅のまま | 起こさない |
| `act-i32` | 640 | 15個の2乗を`vpmulld ymm`2命令で処理する | 起こす |
| `l2-madd` | 654 | 1反復64積を`vpmaddwd ymm`4命令と`vpdpwssd ymm`4命令で処理するが、各組に有効な積が1個だけ入り、L2は64積あたり26命令から30命令に増える | 起こさない |

`l1-256`は、設計書が測定単位として起こす条件（主ループが32個の積を256ビット幅の積和2命令で処理し、命令数が減ること）を、3通りのどれも満たさなかった。
`act-i32`は、y の絶対値を8,160で制限してから`i32`で2乗する。8,160² = 66,585,600を19ビット右シフトすると127になるので、|y| ≥ 8,160で元の式と同じく127に飽和し、|y| < 8,160では元の式と同じ値になる。この境界はコンパイル時の表明で固定した。
`act-i32`が省く処理は、探索時間の1.3%を占めるスカラーの2乗であり、0.5%の目安を超える。

入力の配列の0埋めは、`libc`の呼び出し全体でも0.45%と0.56%であり、`unsafe`を使わずに0埋めを省く方法（使う長さだけを埋める配列）は見つからなかったので、測定単位にしない。

### 駒コードの復号の連鎖

呼び出し元ごとの時間を、呼び出される関数ごとに合計すると次のとおりである。

| 関数 | bench深さ16 | 教師生成2局 | 最大の呼び出し元 |
|---|---:|---:|---|
| `Position::piece_at` | 0.86% | 0.90% | 手の順序付けの`piece_at_for_ordering`（bench深さ16で0.35%） |
| `PieceCode::color` | 0.68% | 0.89% | `Position::captured_squares`（同0.34%） |
| `PieceCode::kind` | 0.44% | 0.58% | 呼び出し元ごとに0.1%以下 |
| `movement_profile` | 0.34% | 0.41% | `capture_targets_without_special`（同0.13%） |
| `PieceCode::is_promoted` | 0.06% | 0.04% | ― |

`Position::piece_at`は盤の配列の1回の読み出しと空升の比較であり、`PieceCode::color`は空升と番兵の比較と1ビットの検査である。どちらも、段階2と段階3で表引きにした処理のような、範囲の検査や駒種の表引きの連鎖を持たない。
合計は0.5%を超えるが、呼び出し元ごとには0.35%以下であり、探索の結果を変えずに省ける処理を具体的に挙げられなかったので、測定単位にしない。

### 通常駒の捕獲可能升の走査

`capture_targets_without_special`は、探索時間のうちbench深さ16で3.94%（自身は0.56%）、教師生成2局で4.59%（同0.66%）を占め、その大半は遮蔽のない到達範囲との共通部分の検査`Bitboard::intersects`（1.84%と2.29%）である。
分岐予測ミスの標本では、この関数とその呼び出し先がbench深さ16で3.96%、教師生成2局で2.98%を占め、自身は0.70%と0.48%である。分岐予測ミスの大半は、手の順序付け`MovePicker::next`（自身で15.8%と12.7%）と`negamax`にある。
共通部分の検査を減らすには、検査を通る升の候補を先に絞る必要があるが、探索の結果と候補の順序を保ったまま絞る方式は見つからなかったので、測定単位にしない。

## 結論

段階5で起こす測定単位は、L1の2乗の活性化を`i32`のレーンで計算する1つ（`movegen-speedup-3-stage5-activation`）である。
L1の256ビット幅の積和、L2の32ビットの積和の形、入力の配列の0埋め、駒コードの復号の連鎖、および通常駒の捕獲可能升の走査は、測定単位にしない。

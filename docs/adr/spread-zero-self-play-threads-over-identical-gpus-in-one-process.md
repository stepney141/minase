# AlphaZero型探索の自己対局は、1つのプロセスの中でスレッドtの推論器をGPU t mod Nに置き、同じ機種のGPUだけの機械で動かす

## ステータス

承認済み。決定日は2026年11月10日。

## コンテキスト

強化学習の1サイクルは自己対局の生成と学習からなり、2回目の学習の20ブロック256チャネルのネットでは、ColabのG4で生成が約34分、学習が約19分と見込まれている（[2回目の実行の設計書](../plans/alphazero-rl-2.md)の「費用」の節）。
GPUを複数持つ機械を借りて1サイクルの料金を下げるには、学習だけでなく生成も全部のGPUを使う必要がある（[複数GPU化の設計書](../plans/alphazero-multi-gpu.md)の「目的」の節）。
生成器は1つのプロセスの中で複数の探索のスレッドを動かし、スレッドごとに推論器を1つ持ち、対局番号を全スレッドで共有するカウンタから取る（[葉をまとめて推論するADR](batch-zero-self-play-leaves-across-many-games.md)）。
塊の確定、生成を止める規則、および再開は、このカウンタと出力先の塊のファイルだけで決まる（[塊の確定のADR](commit-zero-self-play-in-fixed-game-chunks-up-to-the-first-prefix-reaching-the-target-positions.md)）。

TensorRTの実行プロバイダは、GPUの機種名とメモリの容量をキャッシュの鍵に入れ、作ったエンジンを再利用する。
`nvidia-smi`は`CUDA_VISIBLE_DEVICES`を無視し、ONNX Runtimeはこれに従うので、この変数を設定すると2つの見るGPUが食い違い得る。
エンジンの開発者は、2026年11月9日に生成も複数GPUにすると決め、方式はClaudeが提案して設計書の承認で決まった。

## 決定

生成器は必須の引数`--devices <N>`で使うGPUの枚数を受け取り、0から数えたスレッドtの推論器をCUDAの機器番号t mod Nに置く。
`--threads`はNの倍数でなければ起動を拒み、TensorRTとCUDAでは`nvidia-smi`の全GPUが同じ機種名と容量で、枚数がNと等しいことを確かめる。
TensorRTとCUDAの推論器は、`CUDA_VISIBLE_DEVICES`が設定されていれば（空の文字列を含む）起動を拒む。
`--devices`は`--threads`と同じく実行の条件として扱い、保存する生成条件には入れないので、塊、停止、再開、およびシードの規則は変わらない。

## 検討した代案

GPUごとに生成器のプロセスを起動し、塊を分け合う案は採らなかった。
今の生成器は1つの出力先を1つのプロセスだけが使うようにロックし、生成を止める規則は塊0から連続して確定した塊を見渡すので、プロセスの間で塊を分け合うには、塊の取り合いと停止の判定をファイルの上で作り直すことになるからである。

`CUDA_VISIBLE_DEVICES`で各プロセスに1枚ずつ見せる案は採らなかった。
`nvidia-smi`の照会と推論の機器の対応が環境変数に左右され、キャッシュの鍵と推論の機器を取り違え得るからである。

機種の混ざった機械で枚数の最も多い機種だけを使う案は採らなかった。
キャッシュの鍵を機種ごとに分けることになり、借りる機械を同じ機種に限っても困らないからである。

## 結果

同じ基本シードと対局番号の対局は、Nによらず同じシードを使うが、推論のバッチの組み方とFP16の丸めが変わり得るので、着手まで同じになるとは限らない。
作業機では、`--devices 1`の生成が改める前の生成器と同じ塊をバイト単位で書いた（[作業機の測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-workstation.md)）。
`minase dev nn-eval`と対局用のMCTSは機器0で推論し、`--nn-backend cpu`では`--devices`を1に限る。
同じ機種の別の番号のGPUでも同じTensorRTのエンジンを読めることを前提にしており、これは4枚の機械で確かめる。
実装は`crates/minase/src/bin/minase/zero_selfplay/`と`crates/minase/src/nn/tensorrt_cache.rs`にある。

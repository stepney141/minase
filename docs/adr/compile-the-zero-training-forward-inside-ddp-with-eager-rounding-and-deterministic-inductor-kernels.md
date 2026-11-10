# AlphaZero型探索の学習は、学習の順伝播をDDPの内側でtorch.compileし、Inductorのemulate_precision_castsとdeterministicを有効にする

## ステータス

承認済み。決定日は2026年11月10日。

## コンテキスト

[ふかうら王の学習](https://github.com/yaneurao/YaneuraOu/wiki/%E3%81%B5%E3%81%8B%E3%81%86%E3%82%89%E7%8E%8B%E3%81%AE%E5%AD%A6%E7%BF%92)は、dlshogiの学習器で`torch.compile`を使うと学習が約1.6倍速くなると紹介している。
[1回目の実行の設計書](../plans/archived/alphazero-rl.md)は2026年10月29日のエンジンの開発者の決定でこれを導入するとし、[複数GPU化の設計書](../plans/alphazero-multi-gpu.md)は、採否を3つの条件で決めるとした。
1つ目は初回のコンパイルを含めた1サイクルの学習の時間がコンパイルしない版より短いこと、2つ目は同じ機械で同じGPUの枚数のまま強制終了し、空のコンパイルのキャッシュで再開した学習が、途中で止めない学習とバイト単位で一致すること、3つ目はコンパイルの有無による勾配の相対誤差がbf16とfp32の間の相対誤差より小さいことである。

設計書のとおりにコンパイルすると、作業機のRTX 3060 Tiで、bf16の勾配の大半がNaNになり、再開した学習も重みまで食い違った（[作業機の測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-workstation.md)）。
前者は、Inductorが融合した演算の間のbf16の丸めを省くので、大域プーリングの`amax`の逆伝播で、計算し直した値と保存した最大値が一致しなくなったためと考えられる（推論）。
後者は、Inductorが縮約のカーネルの構成を実行時の計測で選び、プロセスごとに選択が変わり得るためと考えられる。
これらを防ぐ`emulate_precision_casts`と`deterministic`の2つの設定は、Claudeが着手後に補った。

## 決定

学習用の全出力を返す薄いラッパーの順伝播を`torch.compile(..., fullgraph=True)`の既定のモードでコンパイルし、Fabricの`setup(..., _reapply_compile=False)`でDDPの内側に置き、順伝播は`backward_pass_autocast="off"`の下で呼ぶ。
Inductorの`emulate_precision_casts`（eagerと同じbf16の丸めを残す）と`deterministic`（数値に影響する実行時の計測を省く）を、プロセス全体と`torch.compile`のオプションの両方で有効にする。
損失の計算、最終の重みの診断、およびONNXの書き出しはコンパイルしない。

## 検討した代案

コンパイルしない案は採らなかった。
2つの設定を加えた学習器は3つの条件をすべて満たし（[作業機の測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-workstation.md)、[vast.aiの測定](../measurements/alphazero-multi-gpu/alphazero-multi-gpu-vast.md)）、作業機の1枚では学習の時間を約1.11倍速くしたからである。

DDPの外側でコンパイルする案（Fabricの既定）は採らなかった。
PyTorch 2.11.0のDDPの`forward`には、コンパイルの対象から明示的に外した前処理があり、外側を1つのグラフにできる前提に立てないからである。

`mode="max-autotune"`は採らなかった。
dlshogiの開発者の[記事](https://tadaoyamaoka.hatenablog.com/entry/2026/05/09/210912)が手元でエラーになったと報告しており、実行時の計測が増えて再開の一致をさらに崩し得るからである。

2つの設定を、学習の順伝播を囲むコンテキストマネージャの中だけで一時的に有効にする案は採らなかった。
Inductorは逆伝播のグラフを最初の逆伝播のときにコンパイルし、Dynamoはグローバルの設定を元に戻すので、順伝播を囲むコンテキストマネージャでは逆伝播のカーネルに効かないからである。

## 結果

4枚のRTX 4090の機械では、初回のコンパイルを含めた1サイクルの学習の時間がコンパイルしない版の約0.99倍で、速くなる幅は小さかった（vast.aiの測定）。
コンパイルした逆伝播は勾配を最後にまとめて返すので、DDPの勾配の平均の通信が逆伝播の計算と重ならなくなることが、差を縮めていると考えられる（推論）。
初回のコンパイルは、最初の50回の更新の区間を毎秒約1,500局面に下げる。
作業機では、2つの設定による速さの低下は見られなかった。
`_reapply_compile`は名前が下線で始まる引数なので、Fabricの版を上げるときは、コンパイルしたラッパーがDDPの内側に置かれることを確かめ直す。
PyTorchの版を上げるときは、[教訓](../lessons/check-compiled-training-numerics-on-the-gpu-in-bf16.md)に従って、GPUのbf16の勾配と再開の一致を測り直す。
実装は`tools/train/zero/minase_zero/train.py`の`compile_training`と`compiled_forward_context`にある。

# AlphaZero型探索の学習は、Lightning FabricのDDPで、GPUの枚数によらず全体のバッチ1,024をミクロバッチ256の勾配累積で学び、BatchNormの移動統計はPyTorchとDDPの既定どおりに更新する

## ステータス

承認済み。決定日は2026年11月15日。
[最初のミクロバッチだけで移動統計を更新するADR](archive/train-zero-cycles-with-fabric-ddp-over-a-fixed-global-batch-of-1024.md)を置き換える。

## コンテキスト

置き換えたADRは、学習器を`torchrun`で起動し、Lightning Fabricの分散データ並列（DDP）で、GPUの枚数によらず全体のバッチ1,024をミクロバッチ256の勾配累積で学ぶと定めた。
同じADRは、GPU 1枚で勾配累積を行う場合と同じ移動統計（移動平均と移動分散）にするために、BatchNormの移動統計を各更新の最初のミクロバッチだけで更新し、ほかのミクロバッチでは逆伝播の後に元の値へ書き戻し、DDPの`broadcast_buffers`を切ると定めた。
2026年11月15日に学習の設定を先行実装のソースと論文で照合したところ、移動統計を最初のミクロバッチだけで更新する方式には先例がなかった（[学習レシピを先行実装に揃える設計書](../plans/archived/zero-recipe-precedents.md)）。
dlshogiの学習器は同期しないBatchNorm（`dlshogi/config.yaml`の`sync_batchnorm: false`）を、Gumbel dlshogiの学習器はPyTorchの`DistributedDataParallel`の既定を使い、どちらも各順伝播で移動統計を更新し、0番目のプロセス（ランク0）の重みを保存する（DeepLearningShogi、コミットa9e7e5f、Gumbel dlshogi、コミット5538f7bの`gumbel_dlshogi/train.py`）。
lc0の学習器も、分割したバッチのすべてで移動統計を更新する。
エンジンの開発者は同じ日に、先例のないminase独自の方式を破棄すると決めた。

## 決定

学習器は`torchrun`が起動したプロセスでだけ動き、Lightning FabricにDDPと集団通信を受け持たせ、学習ループ、リプレイバッファから抽出した局面の列、チェックポイント、および再開の照合は自前のまま保つ。
GPUの枚数をN、全体のバッチBを1,024、ミクロバッチmを256とし、k番目の更新は抽出の列sの区間s[kB:(k+1)B]の局面で行い、これをK=B/m個のミクロバッチ（0から数えた番号j）に分ける。
0から数えたGPU rは、j mod N = rとなるミクロバッチを順に処理して各損失にN/Kを掛け、自分が担当する最後のミクロバッチの逆伝播でだけ、全GPUの勾配を平均する。
BatchNormの移動統計は、PyTorchの既定どおり学習の各順伝播で更新し、DDPの`broadcast_buffers`は既定の`True`としてランク0の統計をほかのGPUへ配り、チェックポイントにはランク0のネットを保存する。
DDPの`find_unused_parameters`を`True`にして、最初の更新の後にDDPが勾配バケットを組み替える動作を止める。
各GPUのプロセスは自分のミクロバッチに入る局面だけを保持し、診断用の局面の保持、チェックポイントの保存、最終の診断、およびONNXと集計の書き出しは、ランク0のプロセスだけが行う。

## 検討した代案

移動統計を最初のミクロバッチだけで更新し、GPUの枚数によらず統計を揃える方式を保つ案は採らなかった。
チェスと将棋のエンジンに先例がないからである。

全GPUで統計を同期するSyncBatchNormは採らなかった。
dlshogiは同期しない設定で学び、Gumbel dlshogiも既定の同期しない扱いで学ぶからである。

学習ループをLightningのTrainerに任せる案、PyTorchのDDPを直接使う案、全体のバッチを256のまま各GPUに64ずつ分ける案、GPUごとのバッチを256に固定して全体のバッチを枚数とともに変える案、およびDDPの既定の`find_unused_parameters=False`でバケットを組み替えさせる案は、置き換えたADRと同じ理由で採らなかった。

## 結果

重みの1回の更新の間に、保存する移動統計が更新される回数はGPUの枚数で変わり、1枚ならミクロバッチ4つ分の4回、4枚ならランク0が処理する1回になる。
重みの更新の式はGPUの枚数によらず、Nによる違いは浮動小数点の加算の順序と移動統計の更新回数になる。
Nを変えて学んだネットの出力は、置き換えたADRのときと同じく一致しない。
試験では、CPUの1プロセスと2プロセスで求めたクリップ前の勾配が一致することと、1プロセスでは移動統計がミクロバッチごとに更新されることを確かめる。
Nは1、2、4に限られ、8枚の機械は、ミクロバッチの大きさか全体のバッチを変えない限り使えない。
実装は`tools/train/zero/minase_zero/train.py`と`data.py`にあり、学習率は[学習率のADR](keep-the-zero-learning-rate-constant-after-a-250-update-warmup-and-cut-it-tenfold-on-stalls.md)が定める。

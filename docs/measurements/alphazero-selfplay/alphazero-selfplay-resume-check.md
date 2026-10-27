# 自己対局生成器の途中終了からの再開

## 目的

[自己対局生成器の設計書](../../plans/archived/alphazero-selfplay.md)のフェーズ2として、作業機で生成を途中で強制終了し、同じ設定で起動し直したときに、確定済みの塊が変わらず、欠けた塊だけが生成し直されることを確かめる。

## コマンドライン

生成のコマンドは[run.sh](alphazero-selfplay-resume-check/run.sh)であり、2回とも同じ引数で起動した。
1回目は[first-run.sh](alphazero-selfplay-resume-check/first-run.sh)から起動し、標準エラーに最初の塊の確定を示す行が現れた時点で、プロセスを`SIGKILL`で止めた。
借りた機械が予告なく止まる場合に合わせて、終了処理の機会を与えないシグナルを選んだ。

```console
source tools/train/zero/env.sh   # bashで実行する
export XDG_CACHE_HOME=<キャッシュ>
minase data zero-selfplay --output-dir <出力先> \
  --nn-model zero-b10c128.onnx --nn-backend tensorrt \
  --generation-commit 4e149184b31ad6704f10ae734203a6b192ca0d2f \
  --seed 20261027 --games 24 --chunk-games 8 --simulations 32 \
  --playthrough-percent 100 --threads 2 --games-per-thread 8
```

24局を8局ずつ3つの塊に分け、投了は無効にした。
2回の起動の間に、1回目に確定した塊のファイルと生成条件の記録`config.json`について、検査和と更新時刻を記録した（[before.txt](alphazero-selfplay-resume-check/before.txt)、[after.txt](alphazero-selfplay-resume-check/after.txt)）。

## エンジン

生成器は、ブランチ`alphazero-selfplay`のコミット4e149184（レビューの指摘を直した最終のコミット）をリリースビルドした`minase`である。
ネットは、`tools/train/zero/feasibility/make_nets.py`が作るランダム初期化の10ブロック128チャネルのネット`zero-b10c128.onnx`（SHA-256は`61a62a623067f9328f1baa91700dc98c81b7c43146ef8448f8263cc570d01600`）である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）である。
推論は、ONNX Runtime 1.28.3のTensorRTの実行プロバイダ（TensorRT 10.14.1.48、FP16）で行った。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、メモリ30 GiB、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で実行した。
推論器を1つずつ持つスレッドを2本、スレッドごとの同時対局数を8とした。
測定の直前まで、作業機では別の作業が16スレッドを使っており、測定の直後の負荷平均は1分間で6.7、15分間で20.7であった。
生成器は`nice -n 10`で起動した。

## 結果

1回目は起動から5秒後、塊0の確定を示す行（[run1.err](alphazero-selfplay-resume-check/run1.err)）の直後に止まった。
この時点で18局が終局していたが、確定していたのは塊0だけであり、塊1と塊2は一時ファイル（`*.mnsd.tmp`）のまま残った。
同じネットと最大バッチのTensorRTのエンジンは、この測定の前に同じ手順を行った際にキャッシュに作られていたので、この5秒にエンジンの作成は含まれない。

2回目は塊1と塊2の16局だけを生成し、5.6秒で終わった（[run2.err](alphazero-selfplay-resume-check/run2.err)、[run2.out](alphazero-selfplay-resume-check/run2.out)）。
塊0の3つのファイル（投了以外の対局、投了の対局、および集計）と生成条件の記録`config.json`は、検査和と更新時刻が1回目の終了時と一致した。
2回目の終了後、出力先に一時ファイルは残らず、3つの塊とサイクルの集計`summary.json`がそろった。
3つの塊の投了以外の対局のファイルは、どれも`minase data selfplay inspect`の全項目の検査を通り、ヘッダの教師の種類はMCTS、教師の規模は32、シードは20261027、ネットの検査和は上のSHA-256であった。

24局はすべて王駒の捕獲で終わり（先手15勝、後手9勝）、手数の上限による打ち切りはなかった。
1局の平均は乱択の序盤を含めて108.0手、記録した探索局面は合計2,334であった。
投了は無効にしていたが、投了の条件の成立は数えており、条件を満たした対局はなかった。
2回目の起動の毎秒の局面数は271であったが、局数とバッチが小さいので、生成速度の見積もりには使わない。
レビューの指摘を直す前のコミット17f396f2で同じ手順を行った際も、塊0だけが確定した状態から欠けた2つの塊だけが生成し直され、3つの塊の対局のデータは今回とバイト単位で一致した（ヘッダの生成コミットを除く）。

## 結論

生成を途中で強制終了しても、確定済みの塊は変わらず、同じ設定で起動し直すと欠けた塊だけが生成し直された。

# NNUEは64 superbatchを余弦の学習率の予定で学び、学び終えた最後のネットを書き出す

## ステータス

承認済み。決定日は2026年11月14日。[NNUEの学習率をcosine annealingで減らす](archive/anneal-the-nnue-learning-rate-with-a-cosine-schedule.md)を置き換える。

## コンテキスト

[以前の決定](archive/anneal-the-nnue-learning-rate-with-a-cosine-schedule.md)は、学習率の倍率を完了した更新回数の関数とし、既定値を総長3エポック、下限0.1のcosine annealingとしていた。
当時の学習器は、学習の長さを周（エポック、教師の全訓練局面を1回ひととおり学ぶこと）で数え、検証損失が5周続けて改善しなければ止まり、検証損失が最良の周のネットを書き出していた。
この方式では、教師を37本から70本に増やすと、同じ周回数でも更新回数と学習率の進み方が約1.9倍変わる。

先行の学習器の調査（[学習レシピの調査](../research/nnue-training-recipes.md)）では、bullet、tatara、tanuki-、およびSaplingが、教師の量によらない単位superbatch（16,384局面のバッチ6,104回分の更新）で学習の長さを指定していた。
tataraとtanuki-は事前に決めた長さを学び終えた最後のネットを使い、nnue-pytorchの手引き、tanuki-の作者、およびtataraの作者は、検証損失の良し悪しが棋力と一致しないと書いていた。
検証損失で選んだネットを標準とする学習器は、調べた範囲ではminaseだけだった。
学習率の予定は、minaseと同じAdamWを使うStockfish、Viridithas、およびSaplingの25.5億局面のネットが、学習の終わりまで続く余弦にしていた。
2026年11月14日にエンジンの開発者は、minaseが独自に選んだ条件より、先行する複数のエンジンに共通する方法に合わせると決めた。
学習の長さは、先例のうち教師の量が最も近いSaplingの1.1億局面の学習の値として、エンジンの開発者が64 superbatchを選んだ。

## 決定

`minase-trainer nnue train`は、学習の長さを`--superbatches`で、1 superbatchの更新回数を`--batches-per-superbatch`（既定6,104）で受け取り、その積の回数の更新を学び終えた最後の重みを`nnue.bin`へ書き出す。
学習率の予定は`--lr-schedule`で必ず指定させ、総長と半減期の単位をsuperbatchとする。
標準レシピは64 superbatchを`cosine:64:0.027`（64 superbatchで初期値の0.027倍まで余弦で下げる）で学ぶ。
検証損失は`--validation-superbatches`の間隔と学習の終わりに測って記録するが、停止とネットの選択には使わない。
引数、保存と再開、および記録の仕様は`crates/minase-trainer/src/nnue/train/`のdocコメントにある。

## 検討した代案

周で学習の長さを数える方式は、教師の量で更新回数が変わり、調べた範囲でこの方式を使うのはやねうら王の旧学習器とminaseだけなので採らない。
検証損失で止めて最良のネットを書き出す方式は、上の調査と、中断した世代反復で検証損失が最良のネットと最後のネットの優劣が確かめられなかったこと（[記録](../measurements/nnue-generation-reset/nnue-generation-reset-gen2-cosine10.md)）から採らない。
学習の長さを40（bulletの例）や160（tataraの公開実験）とする案は、教師の量が大きく異なる例の値なので採らない。
学習率の下限を以前の0.1倍のままにする案は、先行例に同じ値がなく、以前の予定から引き継いだだけの値なので採らない。
Saplingの1.1億局面のネットと同じ階段（16 superbatchごとに0.3倍）は、終わりの倍率0.027は同じだが、階段を使う先行例がこのネットとbulletの例にとどまるので採らない。
nnue-pytorch、tatara、およびtanuki-に共通する1 superbatchごとに0.992倍の予定は、最適化手法がRangerで学習の長さが160〜800 superbatchの場合の値であり、64 superbatchでは約0.6倍までしか下がらないので採らない。
学習の新しい形を世代反復の前に中断した系列の教師で現行の形と対局で比べる案は、エンジンの開発者が採らなかった。

## 結果

70ファイルの教師（約1.9億局面）では、64 superbatchが約34周、390,656回の更新にあたる。
チェックポイントはsuperbatchの境界ごとに確定し、周の途中からでも止めずに通した学習と同じネットへ再開できる。
時間の上限で予定の更新に達しなかった学習は`nnue.bin`を書き出さず、終了コード1で終わる。
長い学習で強くなることはminaseではまだ確かめておらず、最初の確認は[世代反復](../plans/nnue-generation-iteration.md)の第1世代の判定になる。
エポック単位のチェックポイントを書いた変更前の学習器の保存物からは再開できない。

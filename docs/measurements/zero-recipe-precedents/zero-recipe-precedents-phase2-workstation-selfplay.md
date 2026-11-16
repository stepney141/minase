# 改めた自己対局の生成器の作業機での確認

## 目的

[学習レシピを先行実装に揃える設計書](../../plans/archived/zero-recipe-precedents.md)のフェーズ2で改めた生成器を使い、小さなネットによる自己対局を作業機で生成して、設計書の完了条件の5点（序盤の訪問回数による抽選、全探索局面の方策の教師、勝ちが確定した手の価値、手数の上限による引き分け、10局に1局の投了禁止）を確かめる。

## コマンドライン

作業ディレクトリは`tools/train/zero`とし、bashで`env.sh`を読み込んだ。
`<S>`は作業用のディレクトリ、`<W>`はworktreeのルートである。

```console
uv run --frozen python -m minase_zero init --blocks 2 --channels 32 --seed 5 --out-dir <S>/init-small --trainer-commit <コミット>
<W>/target/release/minase data zero-selfplay --output-dir <S>/sp --nn-model <S>/init-small/model.onnx --nn-backend cpu \
  --generation-commit <コミット> --start-position initial --seed 20261118 --games 100 --target-positions 1000000 \
  --chunk-games 10 --simulations 32 --resign-threshold 0.5 --threads 2 --devices 1 --games-per-thread 50
for k in 0 1 2 3 4 5 6 7 8 9; do
  <W>/target/release/minase data selfplay inspect <S>/sp/chunk-0000$k.mnsd --dump 1000 > <S>/dump-$k.txt
done
```

ランダムに初期化したネットの価値は0.5の近くに集まるので、投了の閾値に0.5を与えて、投了と投了禁止の対局の両方が起きるようにした。
各塊（`chunk-NNNNN.mnsd`）の全項目の表示（`--dump`）と生成器の集計`summary.json`の内容を、使い捨てのPythonのスクリプトで集計した。

## エンジン

生成器は、ブランチ`zero-recipe-precedents`のフェーズ1のコミット`5c60285b`にフェーズ2の変更を加えた作業ツリーからビルドした。
規則セットは`JCSA`（L0,P0,R1,E0）であり、ネットはランダムに初期化した2ブロック32チャネルのネット（SHA-256は`b089cfe344c9ff89cfecf68ddb1944b53e787d28757d8aff1f9700cd00ec8417`）である。

## 環境

作業機のCPUはIntel Core Ultra 7 265KF（20コア20スレッド）であり、推論はONNX RuntimeのCPUの実行プロバイダで行った。
持ち時間つきの対局の測定が並行して走っていたので、`nice -n 19`で実行した。
生成器のスレッドは2つであったが、ONNX RuntimeのCPUの推論が演算の内部で複数のスレッドを使い、生成の52.5秒の間に約12コアを使った。

## 結果

100局を生成し、訓練局面は2,281、探索した手番も2,281であった。
探索した2,281項目は、すべて方策の候補と根の総訪問回数を持っていた。

投了禁止の対局は対局番号1、11、21、31、41、51、61、71、81、および91の10局であり、集計の`resign_calibration_games`もこの10局を対局番号の順に持っていた。
残る90局はすべて投了で終わり、投了した局面の手数は最小で10手、投了した局面で記録した価値はすべて0.5より小さかった。
投了禁止の10局は、王手限定の3手以内の勝ちによる打ち切りで終わった。

記録した価値が65,535（勝ちの確定）の局面は2つあり、手数はどちらも30手より前（16手目と11手目）であった。
手数の上限の3,000手に達した対局はなく、集計の`draw_reason_max_ply`は0であった。

序盤の30手の指し手が訪問回数による抽選であることは、この出力からは確かめられなかった。
教師データの形式MNSDは根の辺ごとの訪問回数を持たず、ランダムに初期化したネットの方策の教師は、訪問した手と訪問しない手の区別を表さないからである。
指した手が方策の教師の最大の手と一致する割合は、30手より前で0.033、30手以降で0.064であった。

## 結論

全探索局面の方策の教師と10局に1局の投了禁止は、作業機の出力で確かめた。
序盤の抽選、勝ちが確定した手の価値、および手数の上限による引き分けは、この出力では確かめられないので、生成器の単体試験（`explorative_actions_use_all_visits_at_29_and_final_selection_at_30`、`decision_uses_exploration_at_29_and_gumbel_scale_one_at_30`、`proven_values_override_stale_q_in_both_directions_including_approximation`、`mate_and_max_ply_draws_count_results_and_all_training_positions`、`inference_batches_contain_multiple_games_and_max_ply_draws_keep_records`）で確かめた扱いとする。

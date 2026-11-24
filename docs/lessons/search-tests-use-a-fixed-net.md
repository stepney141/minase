# 探索のテストは学習済みの埋め込みネットに依存させない

## 症状

学習率スケジュールの計画で、段階ゲートのために埋め込みのNNUE`crates/minase/nets/nnue.bin`を別の学習結果に差し替えたところ、探索の単体テストのうち、SEE枝刈りの発動条件を検査するテストが2件と、静止探索の手数上限を検査するテストが2件、ネットごとに異なる組合せで失敗した。
枝刈りの規則も静止探索の規則も変えていないのに、`cargo test`の通らないコミットになった。

## 原因

探索のテストの115か所が`crate::eval::weights()`で埋め込みの学習済みネットを読み、合成局面をそのネットで評価した値から導いた期待（捕獲が損と判定されるか、読む捕獲の数、停止の閾値）を固定していた。
合成局面は数枚の駒しか持たず、学習済みネットの評価値はそのような局面では学習のたびに大きく変わるので、テストの主張とは無関係な理由で落ちる。

## 以後の規則

探索のテストは、学習済みの本番のネットを読まず、`search/alphabeta/tests/mod.rs`の`test_network()`が返す固定ネットで書く。
このネットは、コードで組み立てるA型のLayerStack（`eval::nnue::network::test_util::fixed_layer_stack()`）であり、駒割のPSQTに決まった後段を足したものである。
期待値は評価値から計算で導くか、評価値に依存しない形にし、学習で変わる重みに結びつけない。
ネットのファイルの読み込みと識別子を確かめるテストも、本番のネットではなく、固定ネットを書き出したファイル`nets/test-layer-stack.bin`で書く。
本番のネットは実行ファイルに埋め込まず、リポジトリにも置かないので（[ADR](../adr/search-with-an-evalfile-nnue-whose-structure-is-chosen-on-load.md)）、テストからは読めない。

## 出典

- [plans/nnue-lr-schedule.md](../plans/archived/nnue-lr-schedule.md)（段階ゲートのためのネットの差し替え）
- `crates/minase/src/search/alphabeta/tests/mod.rs`の`test_network`と`crates/minase/src/eval/nnue/network/test_util.rs`の`fixed_layer_stack`

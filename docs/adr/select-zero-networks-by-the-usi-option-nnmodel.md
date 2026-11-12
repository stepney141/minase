# AlphaZero型探索のネットはUSIのオプション`NnModel`のファイルで指定し、`isready`で読み込む

## ステータス

承認済み。決定日は2026年11月12日。

## コンテキスト

AlphaZero型探索のMCTS（モンテカルロ木探索）が局面の評価に使うニューラルネット（以下ネット）は、ONNX形式のファイルとして作業機のNASに置き、コミットしない。
これまでは、エンジンの起動引数`--nn-model <パス>`でファイルを指定し、起動時に1回だけ読んでいた（[ネットを起動引数で指定する決定](archive/select-zero-networks-by-startup-argument-and-embed-only-the-judged-one.md)）。
その決定が起動引数を選んだ理由は、対局ハーネス`minase match run`に、比べる2つのエンジン（候補と基準）のそれぞれへ任意のUSIのオプションを送る手段がなかったことである。
この制約は、ハーネスに`--candidate-option`と`--baseline-option`が加わって解消した（[オプションを送る決定](pass-usi-options-to-match-engines-and-record-them-in-the-manifest.md)）。

[起動引数と診断ツールをUSIへ移す計画](../plans/archived/cli-to-usi.md)は、起動後のプロトコルで渡せる情報をプロトコルへ集める。
推論の設定のうち、実行プロバイダ（推論を実行する基盤）、バッチの大きさ、およびキャッシュの場所は、すでにUSIのオプション`NnBackend`、`NnBatchSize`、`NnCacheDir`で渡している。
ネットの指定だけが起動引数に残っていた。
ネットをオプションで指定する方式には先例があり、StockfishのUCIのオプション`EvalFile`、将棋のエンジンdlshogiの`DNN_Model`、およびチェスのエンジンlc0の`WeightsFile`がこれに当たる。各先例のソースの該当箇所は、本決定が置き換えた上記の決定に挙げてある。

## 決定

起動引数`--nn-model`を削除し、MCTSで起動したエンジンは、USIのオプション`NnModel`でONNXのファイルのパスを受け取る。
ネットは`isready`で読み込み、`NnModel`が未設定なら、エラーの行を出して終了コード1で終わる。
読み込みに成功した`isready`は、`readyok`の直前に`info string network_sha256 <64桁の16進数>`を出す。この値は、推論に渡すバイト列のチェックサム（SHA-256）である。
`NnModel`の値を変えると、読み込み済みのネット、推論器、およびMCTSが対局の間に持ち越す探索木を捨て、次の`isready`まで探索と診断を拒否する。
オプションの宣言と、未設定の場合の拒否の文言は、`crates/minase/src/search/mcts/engine.rs`と`crates/minase/src/protocol/usi.rs`のdocコメントにある。

## 検討した代案

起動引数のまま残す案は採らなかった。
起動引数を選んだ理由がなくなっており、推論の設定の入口が起動引数とオプションの2つに分かれたままになるからである。

`setoption`を受けた時点でネットを読み込む案は採らなかった。
読み込みの失敗を`setoption`の応答で知らせる標準の手段がUSIになく、推論器の作成は実行プロバイダなどのほかのオプションがそろってからでないと行えない。
準備の失敗を`isready`で知らせる既存の扱いに合わせる。

`NnModel`を変えても探索木を保つ案は採らなかった。
探索木の節点は、前のネットの評価と事前確率を持っているからである。

## 結果

推論の設定の入口は、USIのオプションに1本化された。
対局ハーネスでは、MCTSを`--candidate-args '--search mcts'`と`--candidate-option 'NnModel=<パス>'`の組で指定する。
パスは実行条件の記録`manifest.json`に残り、ネットの内容は、エンジンが返すチェックサムをハーネスが記録して照合する（[チェックサムを照合する決定](record-and-verify-the-network-checksum-reported-by-match-engines.md)）。
USIの文字列のオプションは、`value`の後の語を1つの空白でつないで値にするので、連続する空白やタブを含むパスは指定できない。この制約は`NnCacheDir`と同じである。

本決定が置き換えた決定のうち、次の2点は変えていない。
1点目に、ファイルを読めない場合と推論器を作れない場合は、別のネットや実行プロバイダへ切り替えずにエラーで終了する。
2点目に、ネットを実行ファイルへ埋め込むのは、到達の判定（MCTSがαβ探索と同等以上に強いかの判定）に使うネットに限る。埋め込みのネットを持つビルドはまだなく、`NnModel`の指定は必須である。

自己対局生成器`minase data zero-selfplay`は、エンジンのセッションではなく、データ一式を生産するサブコマンドなので、自身の引数`--nn-model`でネットを受け取り続ける。
ネットを読み込む処理は、どちらも`crates/minase/src/nn/onnx.rs`の`Network`にある。

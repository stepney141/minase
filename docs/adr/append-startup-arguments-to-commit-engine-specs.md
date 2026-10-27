# 対局ハーネスの`commit:`のエンジンには`--candidate-args`と`--baseline-args`で起動引数を足し、実行条件に記録する

## ステータス

承認済み。決定日は2026年10月27日。

## コンテキスト

対局ハーネス`minase match run`の`commit:`の指定は、コミットをビルドしたエンジンを`--protocol usi --rules <規則>`の起動引数で起動し、ほかの引数を受け付けなかった。
MCTSは起動引数`--search mcts`と`--nn-model <ネットのファイル>`で選ぶので（[探索方式の選択のADR](select-the-search-method-at-startup-through-a-closed-enum.md)、[ネットの指定のADR](select-zero-networks-by-startup-argument-and-embed-only-the-judged-one.md)）、このままではハーネスでMCTSを測れない。
[CONTRIBUTING.md](../../CONTRIBUTING.md)は、ハーネスへ機能比較のスイッチを足すことを禁じ、棋力に影響し得る変更をコミット対コミットの対局で判定すると定めている。
エンジンの開発者は、[探索方式の共存とMCTSの設計書](../plans/archived/alphazero-mcts.md)でこの拡張を決めた。

## 決定

`minase match run`に`--candidate-args`と`--baseline-args`を加え、空白で区切った起動引数を、その側の`commit:`のエンジンの`--protocol usi --rules <規則>`の後に足す。
`commit:`以外の形式のエンジン（`random`、起動コマンド、`cecp:`）にこの引数を与えた場合は、起動の前に入力エラーとする。
足した起動引数は`manifest.json`のエンジンの同一性の欄`args`に記録し、再開時の同一性の照合に含める。
記録の形式が変わるので、実行ディレクトリの形式の版を5から6へ上げ、版5の読み込みや変換は作らない。

## 検討した代案

対象のコミットを固定したworktreeで自前でビルドしたバイナリを、起動コマンドの形式で指定する案は採らなかった。
ハーネスの変更は要らないが、コミットとバイナリの対応をハーネスが検証しなくなる。

起動コマンドの形式にも`--candidate-args`を足せるようにする案は採らなかった。
起動コマンドの形式はもともと任意の引数を書けるので、同じことを2通りに書けるだけになる。

この拡張は、エンジンの起動方法の指定であり、ハーネスが差分の内容を知る機能比較のスイッチではない。
個々の変更の採否には、この拡張を使う場合も、従来どおりコミット対コミットのGSPRTを使う。

## 結果

版5の実行ディレクトリは、版6のハーネスでは再開も`minase match report`の集計もできず、その測定を開始したハーネスで扱う。
起動引数に書いた相対パスは、ハーネスを起動した作業ディレクトリから解決される。
ハーネスはネットのパスだけを記録し、ネットの内容は照合しないので、強化学習の実行ではネットをサイクル番号つきのファイル名で保持して上書きしない。
実装は`crates/minase/src/harness/player.rs`と`crates/minase/src/bin/minase/match_runner/`にある。

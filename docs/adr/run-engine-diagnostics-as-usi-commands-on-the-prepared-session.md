# エンジンの実装を検証する診断はUSIの独自コマンドにして準備済みのセッションで実行し、実験の管理とデータの生産はサブコマンドに残す

## ステータス

承認済み。決定日は2026年11月17日。[エンジンの実装を検証する診断はUSIの独自コマンドにし、実験の管理とデータの生産はサブコマンドに残す](archive/run-engine-diagnostics-as-usi-commands-and-keep-experiment-tools-as-subcommands.md)を置き換える。

## コンテキスト

minaseは、補助ツールを実行ファイル`minase`のサブコマンドとして持つ（[補助ツールをサブコマンドにする決定](run-developer-tools-as-engine-subcommands.md)）。
このうち検証と診断の群`dev`には、合法手の経路数を数える`perft`、固定局面で探索の速度を測る`bench`、ニューラルネット（以下ネット）の出力を固定局面で書き出す`nn-eval`、その固定局面集を生成する`nn-fixture`、およびランダム対局で不変条件を検査する`random-play`があった。
これらはエンジンとは別のプロセスとして動くので、エンジンのセッションの設定を使えなかった。
`perft`は、USIの`position`で復元した局面をそのまま数えられず、局面を文字列に書き出して引数で渡す必要があった。
`nn-eval`は、対局に使う推論器とは別に、引数から推論器を組み立て直していた。
`nn-eval`の引数のうち、実行プロバイダ、バッチの大きさ、キャッシュの場所は、エンジンがすでに持つUSIのオプションと同じ内容であり、`random-play`の`--rules`もオプション`RuleSet`と重複していた。

診断をプロトコルのコマンドとして持つ方式には先例がある。
Stockfishは`bench`、`d`、`eval`をUCIのコマンドとして受け、perftを`go perft <深さ>`として受ける。
やねうら王も`bench`、`d`、`eval`をUSIのコマンドとして受け、`go`の引数として`perft`を受ける。
minase自身も、局面、評価、置換表を表示する`d`、`eval`、`tt`をUSIの独自コマンドとして持っている（[ADR](show-position-evaluation-and-tt-as-usi-commands.md)）。
診断をUSIへ移す判断は[起動引数と診断ツールをUSIへ移す計画](../plans/archived/cli-to-usi.md)で下した（[以前の決定](archive/run-engine-diagnostics-as-usi-commands-and-keep-experiment-tools-as-subcommands.md)）。
以前の決定の時点では、αβ探索の`bench`は実行ファイルに埋め込んだNNUEのネットを使い、規則の設定も`isready`も要求しなかった。
その後、ネットは埋め込まず、オプション`EvalFile`で指定した外部のファイルを`isready`で読むようになった（[外部ファイル化のADR](use-an-nnue-read-from-evalfile-as-the-only-search-evaluation.md)）。
PV-MCTSで起動したエンジンの`bench`は、ネットを使わないハッシュの評価器で探索するようになった（[PV-MCTSのbenchのADR](run-bench-with-the-launched-search-and-detect-pv-mcts-regressions-with-a-hash-evaluator.md)）。
この改訂は[NNUEの外部ファイル化の設計書](../plans/archived/nnue-external-network-file.md)で下した。

## 決定

配置の基準を次のとおりとする。
実行中のエンジンのセッションについて、構成の設定、状態の観測、または計算の実装の検証を行う操作は、USIへ置く。
プロセスの起動条件、別のエンジンの起動、実験全体の管理、およびデータ一式の生産と集計は、起動引数とサブコマンドに置く。

この基準に従い、`perft`、`bench`、`nn-eval`、`nn-fixture`、`random-play`をUSIの独自コマンドにし、対応するサブコマンドを削除する。
PV-MCTS（モンテカルロ木探索）で起動したエンジンの`eval`は、αβ探索が使う評価関数NNUEの内訳ではなく、セッションの推論器によるネットの出力を表示する。
対局測定の`match`、係数調整の`spsa`、学習データの`data`、および校正用のランダム着手エンジン`dev usi-random`は、サブコマンドのまま残す。

診断のコマンドは、探索していないときに同期で実行し、出力の各行に`info string`を付けて、正常に終わったときだけ`info string end`で終える。
引数の誤りはエラーの行を出してセッションを続け、実行時の失敗はエラーの行を出して終了コード1で終わる。
各コマンドの引数と契約は、[USIの独自拡張の手引き](../guides/usi-extensions.md)と`crates/minase/src/protocol/usi.rs`のdocコメントにある。

## 検討した代案

「標準入力からコマンドとして呼べるものはすべてUSIへ移す」という基準は採らなかった。
どの処理にも当てはまるので、配置を決める基準にならない。
`match`、`spsa`、`data`をUSIのコマンドにすると、引数の検査と`--help`、スクリプトが成否の判定に使う終了コード、および長時間の処理の間にUSIの入力を処理する能力を失う。
これらはエンジンの機能ではなく、エンジンを使う道具であり、移しても渡す情報は減らない。

Stockfishのように、起動引数の列を1つのプロトコルのコマンドとして実行して終了する方式は採らなかった。
起動引数として解析できなかった入力をプロトコルのコマンドとして解釈することになり、起動引数の打ち間違いを引数の誤りとして検出できなくなるからである。

診断を探索と同じ別スレッドで実行し、`stop`を受け付ける案は採らなかった。
診断は対局中に送るものではなく、停止の経路を持つ費用に見合う用途がない。

すべての失敗でセッションを続ける案は採らなかった。
スクリプトが終了コードで失敗を検出できなくなるからである。

perftを`go perft <深さ>`の形にする案は採らなかった。
`go`の引数の検査は深さ1以上を要求し、探索の係数を調整するためのビルドでは最初の`go`で係数を固定するので、探索のための検査と副作用がperftにも及ぶ。

CECPには診断のコマンドを加えなかった。
開発用GUIと対局ハーネスはminaseとUSIで接続し、既存の`d`、`eval`、`tt`もUSIだけに置いているからである。

αβ探索の`bench`だけ別にネットを指定する引数を持たせる案は、同じセッションの中にネットの指定の経路を2つ作るので採らなかった。

## 結果

`perft`と`random-play`はセッションの規則を使い、`perft`は`position`で設定した局面をそのまま数える。
`nn-eval`はセッションの推論器を使うので、対局に使う設定と同じ経路の出力を検証できる。
αβ探索の`bench`と`eval`は、`isready`で準備したネットを使い、準備の前に受けたらエラーの行を返す。
`bench`はセッションの設定によらずbench専用の局面、規則、および置換表を使うが、`isready`を通すには規則の設定`RuleSet`が要るので、`bench`の前に`RuleSet`、`EvalFile`、および`isready`を送る。
`nn-fixture`と、PV-MCTSで起動したエンジンの`bench`は、規則の設定も`isready`も要求しない。

利用する側のスクリプトは、`minase --protocol usi`を起動して標準入力へコマンドと`quit`を渡し、終了コードが0であることと、終端の行（`info string end`）があることの両方で成功を判定する。
終了コードだけでは、引数の誤りを検出できない。
過去のコミットをビルドして使う`scripts/bench_compare.py`は、変更以後のコミットだけを扱い、旧い呼び出し方へ切り替える分岐を持たない。

診断の処理は、実行ファイルの側からライブラリの側（`crates/minase/src/diagnostics/`）へ移した。
移す前後で、perftの件数、benchの局面ごとのノード数、固定局面集のバイト列、ネットの出力、およびランダム対局の集計が一致することを確かめた（[検証の記録](../measurements/cli-to-usi/cli-to-usi-verification.md)）。

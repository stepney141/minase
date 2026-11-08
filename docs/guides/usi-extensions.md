# USIの独自拡張

本書は、minaseを`--protocol usi`で起動したときに使えるUSI（Universal Shogi Interface）のコマンドのうち、USI原典の仕様と異なる点と、minaseが独自に追加したコマンドだけをまとめる。
USI原典のコマンド体系と各コマンドの基本的な使い方は、[USI原典仕様とlishogi系拡張の調査](../research/protocols/usi-lishogi.md)と、そこに挙げた原典にあり、本書では説明しない。
エラーの文言、値の境界、および探索中の扱いを含む各コマンドの契約は、`crates/minase/src/protocol/usi.rs`のモジュールと各処理関数のdocコメントが定める。
本書はその要点と、詳しい記述の場所を示す。
起動引数は、リポジトリの[README](../../README.md)の「Running the engine」にある。

## 中将棋の表記

升名、指し手、およびSFEN（局面の文字列表記）は、lishogiの中将棋の表記に従う。
文法と典拠は[調査メモの「lishogi系の中将棋表記」](../research/protocols/usi-lishogi.md#lishogi系の中将棋表記)の節にあり、ここでは標準の将棋のUSIとの違いだけを述べる。

升名は、筋の数字`1`〜`12`と段の英字`a`〜`l`をつなげて書く（例: `6f`、`12a`）。
指し手は升名を2個または3個つなげて書き、駒打ちの形式はない。
3個の形式は、獅子、角鷹、および飛鷲の2段階移動を移動元、経由升、到達升の順に書く。
居喰いとじっとは、どちらも到達升を移動元と同じ升にした3個の形式で書く。
移動元と到達升が同じ2個の形式は受け付けない。
成りは末尾に`+`を付けて表す。
受信時は末尾の`=`と`?`も不成として受け付けるが、minaseが送る不成には接尾辞を付けない。
受信時は、経由升に相手の駒がない3個の形式を経由升のない着手と同じに扱う。
そのため、じっとの経由升にはどの空升を書いてもよく、2升先への跳びを空の経由升つきで書いても受け付ける。
指し手の解析規則は`crates/minase-core/src/notation/usi.rs`の`parse`と`text_generated`のdocコメントにある。

`position startpos`は中将棋の初期局面を表す。
`position sfen`は、`盤面 手番 獅子を取った升 次の手数`の4欄、または末尾に成りを保留している駒の升の欄を加えた5欄の拡張SFENを受け付ける。
第3欄は、標準の将棋の持ち駒欄に代えて、直前の着手で獅子以外の駒が獅子を取った升を書く欄であり、先獅子（chu-shogi.md第15条）の判定に使う。
第5欄はminase独自の欄で、規則P1、P2、またはP5の下で成りを保留している駒の升をコンマ区切りで書き、4欄の入力では`-`とみなす。
欄の文法は`crates/minase-core/src/notation/sfen.rs`の`parse_extended_sfen`のdocコメントにある。
SFENは千日手の判定に使う局面の履歴を含まない。
着手列のない`position sfen`で途中局面から始めた対局では、その局面から履歴を数え始める。

## コマンドの一覧

次の表は、minaseが受け付けるGUIからのコマンドと、minaseでの扱いの要点である。
表で標準どおりとしたコマンドは、USI原典の定めに従って動く。

| コマンド | minaseでの扱い |
|---|---|
| `usi` | 標準どおり。宣言するオプションは「オプション」の節にある。 |
| `isready` | 標準どおり。MCTS（モンテカルロ木探索）では推論器をここで作り、失敗するとエラーを1行出してエンジンを終了する。 |
| `setoption` | minase独自のオプションを持つ。 |
| `usinewgame` | 省略してよい。 |
| `position` | 中将棋の表記を使い、終局した対局では受け付けない。 |
| `go` | 未知の引数と矛盾する引数を拒否し、`go mate`には`checkmate notimplemented`を返す。 |
| `stop` | 標準どおり。探索していないときは応答せずに無視する。 |
| `ponderhit` | 標準どおり。先読み中でなければエラーを返す。 |
| `gameover` | 探索中なら探索の結果を捨て、`bestmove`を返さない。 |
| `quit` | 探索中なら探索の結果を捨てて終了する。 |
| `debug`、`register` | 応答せずに無視する。 |
| `moves`、`state`、`d`、`eval`、`tt` | minase独自の照会であり、「独自のコマンド」の節にある。 |
| 上記以外 | `info string error: unknown command <第1語>`を返す。 |

## 標準コマンドとの違い

### エラーの通知

USI原典は解釈できないコマンドを無視するよう定めるが、minaseは受け付けなかったコマンドを`info string error: <内容>`の1行で知らせる。
受け付けなかったコマンドは、対局の局面と設定を変えない。
`go`を拒否した場合も`bestmove`は返らないので、GUIや対局用のプログラムは`info string error:`の行を見て応答待ちを打ち切る必要がある。

探索中に届いた`go`は、`info string error: go is already running`を返して拒否する。
探索中に届いた`go`、`stop`、`ponderhit`、`gameover`、および`quit`以外のコマンドは、`isready`と`setoption`も含めて、探索が終わってから受信順に処理する。
したがって、探索中に送った`setoption`は次の探索から反映される。

### position

`usinewgame`は省略してよい。
Lishogi-Botは`usinewgame`を送らないので、minaseは対局の開始を待つ状態で最初に受け付けた`position`で、`RuleSet`で指定した規則をその対局に確定させる（[ADR](../adr/commit-pending-rules-on-first-position.md)）。

最後の着手で対局が終わる`position`は受け付けるが、その対局は終局した状態になる。
終局した対局では、`gameover`か`usinewgame`を受けるまで、次の`position`と`go`をエラーで拒否する。
終局とは別に、`position`を拒否した後は、次の`position`を受け付けるまで`go`も拒否する。
拒否の文言と、前回の着手列との差分だけを適用する仕組みは、`UsiProtocol::handle_position`のdocコメントにある。

### go

`go`の引数は`depth`、`nodes`、`movetime`、`btime`、`wtime`、`binc`、`winc`、`byoyomi`、`ponder`、および`infinite`を受け付ける。
未知の引数、同じ引数の重複、および`infinite`と他の制限との併用は拒否する。
制限のない`go`と、制限のない`go ponder`も拒否する。
`byoyomi`が0で`movetime`も指定しない`go`では、相手の残り時間を目安時間の補正に使うので、手番でない側の`btime`または`wtime`を省略すると拒否する。
引数の検査規則は`parse_go_config`のdocコメントにある。

`go mate`は詰み探索を実装していないので、USI原典の定めに従って`checkmate notimplemented`を返す。

`--search mcts --nn-model <ネットのパス>`で起動したMCTSでは、`nodes`の値はシミュレーションの回数を表し、αβ探索の着手の適用回数とは意味が異なる。
MCTSは正常に始まった`go`のたびに、この違いを`info string nodes count MCTS simulations, not alpha-beta move applications`の1行で知らせる。

### gameoverとquit

探索中に`gameover`または`quit`を受けると、minaseは探索を止めて結果を捨て、`bestmove`を返さない。
将棋所は、先読み中に相手が投了すると`stop`を送らずに`gameover`を送るので、minaseはこれを対局待ちへ移る通知として扱う。
`gameover`の引数（`win`、`lose`、`draw`）は使わない。

### bestmoveとinfo

minaseは`bestmove`の直前に必ず`info string stop <理由>`の1行を出す。
理由は、指定深さの完了なら`depth`、ノード数の上限なら`nodes`、時間制限のうち目安時間なら`soft`、上限時間なら`hard`、`stop`などの外部からの停止なら`external`である。
対局用のハーネスは、この行から手ごとの停止理由を記録する。

完了した探索の評価値が`-ResignValue` cp以下なら、`bestmove resign`を返す。
予想手は、GUIが先読みを有効にしているかどうかにかかわらず、読み筋の2手目が合法で、その手で対局が終わらない場合だけ`bestmove <着手> ponder <予想手>`の形で付ける（[ADR](../adr/take-ponder-move-from-the-pv.md)）。

`info`の`score cp <値>`は、探索の内部の評価値を換算せずに出す。
`score mate <手数>`は、詰んだ後に続く詰まされた側の1手と王駒を取る1手を除いた、詰みまでの手数を表す。
手数が0の場合も勝敗の符号を付けて`mate +0`、`mate -0`と書く。
完了していない反復の途中結果を指し手に採用したときは、`info string partial`の行に続けて、完了した深さに1を足した値を深さとする`info`の行を出し、評価値に`lowerbound`を付けて下界であることを示す。
評価値の換算と出力の条件は、`score_text`、`write_info`、および`write_final_info_if_deeper`のdocコメントにある。

## オプション

minaseは`usi`への応答で次のオプションを宣言する。
既定値と範囲は`usi`への応答を正とし、この表と食い違うときは応答に従う。
オプション名は大文字と小文字を区別しない。
ただし`Tune_<名前>`は区別する。
宣言していない名前の`setoption`は、`USI_Ponder`を含めて応答せずに無視する。
`USI_Ponder`を宣言しないのは、先読みの有無で時間配分を変えないためである（[ADR](../adr/measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md)）。

| 名前 | 型と範囲 | 既定値 | 意味 |
|---|---|---|---|
| `RuleSet` | string | 起動時の`--rules`の正準表記 | 規則セット（chu-shogi.md第33条）。規則コードのコンマ区切り、`engine-default`、または`lishogi`で指定し、次の対局から反映される。 |
| `USI_Variant` | string | `chushogi` | 変種名。`chushogi`だけを受け付け、それ以外の値はエラーにする。Lishogi-Botが対局ごとに変種名として送る値を受けるための宣言である。 |
| `USI_Hash` | spin、1〜65,536 | 256 | αβ探索では置換表、MCTSでは探索木の大きさ（MiB）。 |
| `Threads` | spin、αβ探索は1〜256、MCTSは1だけ | 1 | 探索スレッドの数。 |
| `ResignValue` | spin、1〜99,999 | 20,000 | 投了の閾値（cp）。完了した深さが1以上で、評価値が`-ResignValue`以下なら投了する。反復による勝敗を表す専用の評価値の帯は投了の判定から除く（[ADR](../adr/decide-resignation-in-the-usi-layer.md)）。 |
| `ByoyomiMargin` | spin、0〜60,000 | 30 | 秒読みの締切の前に残す余裕で、単位はmsである（[ADR](../adr/give-the-byoyomi-margin-as-a-millisecond-usi-option.md)）。 |
| `DrawValueBlack` | spin、−28,999〜28,999 | 0 | 探索を始める局面（根）が先手番のときに引き分けへ与える評価値。単位は根の手番側から見たcpである。 |
| `DrawValueWhite` | spin、−28,999〜28,999 | 0 | 探索を始める局面（根）が後手番のときに引き分けへ与える評価値。単位は根の手番側から見たcpである。 |
| `NnBackend` | combo、`tensorrt`、`cuda`、`cpu` | `tensorrt` | MCTSだけが宣言する。推論の実行プロバイダ。 |
| `NnCacheDir` | string | `$XDG_CACHE_HOME/minase/tensorrt`（環境変数が空なら空） | MCTSだけが宣言する。TensorRTのキャッシュを置くディレクトリ。空の文字列は`<empty>`と書く。 |
| `NnBatchSize` | spin、1〜1,024 | 256 | MCTSだけが宣言する。1回の推論に送る葉の数の上限。 |
| `PolicyTemperature` | spin、10〜1,000 | 140 | MCTSだけが宣言する。探索木の事前確率を作るときに方策のlogitを割る温度の100倍で、140なら温度1.4を表す。 |
| `Tune_<名前>` | spin、係数ごと | 係数ごと | `--features tuning`でビルドしたαβ探索だけが宣言する。探索と時間管理の係数であり、最初の`go`の後は変更を拒否する（[SPSAの手引き](spsa.md)）。 |

引き分けの評価値は、規則R1の千日手のように探索が引き分けと判定する局面で使う（[ADR](../adr/set-the-draw-value-by-usi-option-signed-from-the-root-side.md)）。
`NnBackend`、`NnCacheDir`、または`NnBatchSize`を変えると、次の`isready`で推論器を作り直す。
各オプションの受理で置換表や探索木を消すかどうかと、拒否の文言は、`UsiProtocol::handle_setoption`のdocコメントにある。

## 独自のコマンド

minaseは、USI原典にもlishogiの実装にもない5つの照会コマンドを持つ。
どれも対局の状態を変えず、探索中に送ると探索が終わってから処理する。
`moves`と`state`は開発用GUIが規則を再実装せずに対局を進めるための照会で、`d`、`eval`、および`tt`は調査用の表示である。
Lishogi-Botとの接続ではこれらを使わない。

### moves

```text
moves
```

現局面の全合法手を1行`moves <着手1> <着手2> ...`で返す。
指し手は`position`と同じ表記で、2段階移動、居喰い、じっと、および成りも含む。
着手の順序に意味はない。
対局中でなければ`info string error: moves requires an active game`を返す。

### state

```text
state
```

適用中の規則、盤面、および対局の状態を、1行`state rules <規則> board <盤面> <手番> status <状態>`で返す。
`<規則>`は規則コードの正準のコンマ区切りで、`<盤面> <手番>`はSFENの最初の2欄である。
`<状態>`は`ongoing`、`win <black|white> <理由>`、または`draw <理由>`であり、理由の語は[調査メモの「stateコマンド」](../research/protocols/usi-lishogi.md#stateコマンド)の節にある。
SFENの2欄は先獅子、成りの保留、および千日手の履歴を含まないので、GUIは`board`から対局を復元せず、着手列を保持して`position`を送り直す。
USIにはエンジンから終局の裁定を知らせる標準の手段がないので、minaseを審判として使うGUIはこの照会で終局を確かめる。
対局中と終局後に応答し、対局の開始前にはエラーを返す。
対局の開始前とは、起動直後、および`usinewgame`か`gameover`の後で`position`を受け付けるまでの間である。

### d

```text
d
```

盤面と局面の内部状態を、`info string`を付けない素の行で表示する。
盤面に続けて、5欄の拡張SFEN、手番、次の手数、対局の状態、獅子を取った升、成りを保留している駒の升、および2つのZobristキーを1行ずつ出し、最後の行は`rights-zobrist`で始まる。
表示する拡張SFENは、そのまま`position sfen`や他の道具に渡せる。

### eval

```text
eval
```

現局面のNNUE（評価関数のニューラルネット）による手番側の評価値と、升ごとの線形の寄与を12行12列の表で表示する。
各行に`info string`を付け、最後の行を`info string end`とする。

### tt

```text
tt
```

αβ探索の置換表に残る現局面と、全合法手の子局面の項目を表示する。
項目は、深さ、境界の種類、評価値、最善手、および静的評価である。
`eval`と同じく各行に`info string`を付け、最後の行を`info string end`とする。
MCTSで起動したminaseでは`info string error: tt requires --search alphabeta`を返す。

`d`、`eval`、および`tt`は対局中と終局後に応答し、対局の開始前にはエラーを返す。
出力の詳しい読み方と、不具合の調査での使い方は[デバッグの手引き](debugging.md)にある。
3つを独自コマンドにした理由は[ADR](../adr/show-position-evaluation-and-tt-as-usi-commands.md)にある。

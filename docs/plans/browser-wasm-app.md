# ブラウザ内で動く対局アプリ（minase-chushogi-wasm）の設計書

## 先に読む要約

本計画は、開発用GUI minase-gui の人間対AIの画面を、サーバーを持たない静的なWebアプリとして新しい別リポジトリ `minase-chushogi-wasm` へ切り出し、GitHub Pages で公開する。
minase-gui では、サーバー上のRust製のランチャーが、画面とminaseのプロセスの間でUSI（将棋エンジンとGUIが1行ずつの文字列で指示と応答をやり取りする通信規約）の行を中継し、盤のクリックに応じた移動先の計算などの規則の照会にも、規則のライブラリminase-coreを使って答えている。新しいアプリでは、minaseとminase-coreをWebAssembly（以下wasm、ブラウザが実行できる機械語の形式）にコンパイルし、エンジンはブラウザの中の別スレッドであるWeb Worker（以下Worker）の上で、規則の照会は画面の中で動かす。
GitHub Pages は、スレッド間の共有メモリ（SharedArrayBuffer）をブラウザに許させる応答ヘッダーを返せず、共有メモリがなければwasmで複数のスレッドを使えないので、エンジンは1スレッドで探索する。1スレッドのWorkerは、探索の関数が戻るまで次のメッセージを受け取れないので、そのままでは探索中に届く停止（`stop`）、先読みの的中（`ponderhit`）、および終局（`gameover`）の指示を処理できない。
そこで、JSPI（WebAssembly JavaScript Promise Integration、wasmの実行を途中で一時中断してブラウザのイベントループへ戻し、後で再開できる機能）を使い、探索を一定の実時間ごとに一時中断して、その間に届いた行を再開後に受け付けた順に処理する。JSPIを持たないブラウザでは、対局を始めずに理由を表示する。
評価関数は、外部ファイルから読む現行のNNUE（着手ごとに差分で更新できる小さなニューラルネットの評価関数で、本計画ではその重みのファイルをネットと呼ぶ。使うのは入力特徴集合がA型のネット、[NNUEの評価関数の手引き](../guides/evaluation-nnue.md)）と、minaseに埋め込まれたPST＋FM（駒と升ごとの点数表に2駒の組の補正を足す評価関数で、2026年10月12日まで探索の既定だったもの、[PSTとFMの評価関数の手引き](../guides/evaluation-pst-fm.md)）の2つから選べる。
minase本体にはブラウザで動かすための継ぎ目（スレッド、時計、入力の待ち方、ネットの読み込みを外から与えられるようにする関数の境界）だけを入れてcrates.ioへリリースし、新リポジトリはその版に固定する。
最大のリスクは、探索の深い再帰がブラウザのスタックの上限に収まるかと、iPhoneのメモリに収まるかの2点なので、画面の移植より先に実機で確かめる。
完了条件は、JSPIを持つブラウザ（Chrome、Edge、Android版Chromeの137以降、Firefox 153以降、Safari 27以降とiOS 27以降）で、GitHub Pagesの公開URLから、両方の評価関数を使い、minase-gui の人間対AIの画面の機能（スレッド数の設定を除く）で対局を最後まで指せることである。

## 状態

起案。2026年11月25日に起案した。
次の一手は、レビューの後に利用者の着手の指示を受けて、フェーズ0の準備に着手することである。

## 目的

- 利用者がブラウザを開くだけで、端末の中だけで動くminaseと中将棋を指せるようにする。サーバーの計算資源を使わず、同時接続数の上限もなくす。
- minaseの探索、評価、およびUSIの状態遷移の正をminase本体の1か所に保ったまま、ブラウザという新しい実行環境を加える。

## 適用範囲

本計画は、次の作業を行う。

- minase本体（`crates/minase`）では、ホストに依存する依存クレートとモジュールをCargoのフィーチャで切り分ける。あわせて、スレッドを使わない探索の実行入口、時計と探索中の入力照会の注入、USIの状態遷移を入力1行ずつ進める公開API、バイト列からのネットの読み込み、Web版の設定、および埋め込みのPST＋FMを探索の評価に使う経路を加える。
- minaseのCIに、`wasm32-unknown-unknown` 向けのビルド検査を加える。
- 新リポジトリ `minase-chushogi-wasm` を作り、エンジン用と規則照会用の2つのwasm、エンジンのWorker、minase-gui から移植した人間対AIの画面、およびGitHub Pagesへの配備を置く。
- 新しい継ぎ目を含むminaseをリリースし、新リポジトリをその版に固定する。

次の事項は扱わない。

- minase-gui は変更しない。移植の元にした minase-gui のコミットを新リポジトリのREADMEに記録し、以後の変更を自動では同期しない。
- AI対AIの画面、HaChu（H. G. Muller作の中将棋エンジンで、通信規約CECPで接続する）との対局、およびオフライン動作（Service Worker によるPWA）は移植しない。
- 置換表を共有する複数のスレッドで並列に探索する方式（Lazy SMP）はWeb版では使わない。スレッド数は1に固定し、設定画面に出さない。
- JSPIを持たないブラウザ向けの代替のバイナリや縮退した動作は作らない（「設計判断」の節）。
- PST＋FMを、native（OS上で動く通常のminase）の実行ファイルの評価関数としては提供しない。
- AlphaZero型探索（PV-MCTS）とONNX Runtimeによるニューラルネットの推論は、Web版に含めない。
- 1命令で複数のデータを処理するwasmのSIMD命令は使わない。必要性を示す測定がないからである。

## 依存関係

### minase-gui の人間対AIの画面

minase-gui（`../minase-gui`、本リポジトリ直下にシンボリックリンクがある）は、Rust製のランチャー `src/main.rs`（axum）と、TypeScriptの画面 `web/src`（ビルドツールのVite、将棋盤を描くshogiground、グラフを描くchart.js）からなる。人間対AIの画面は経路 `/play` で開く。
画面がサーバーに頼る接合面は4つある。

| 画面の箇所 | 現在の相手 | 役割 |
|---|---|---|
| `shared/session.ts` | WebSocket | エンジンとUSIの行をやり取りする |
| `shared/engines.ts` の `fetchEngines` | HTTP `/engines` | 選べるエンジンの一覧を得る |
| `shared/destinations.ts` | HTTP `/destinations` | 盤上の駒をクリックしたときに移動先の升を示す |
| `shared/position-preview.ts` | HTTP `/position` | 開始局面と着手列から各手の局面を作る |

対局の進行、時計、先読み（ponder、相手の手番の間に相手の予想手を仮定して考えること）、MultiPV（複数の読み筋の同時表示）、および排出の制御は `shared/game.ts` の `GameController` が担い、ここはエンジンとUSIの行だけでやり取りする。
ここでいう排出とは、探索を止めた後や対局を閉じた後に、`isready` を送って `readyok` までの応答を読み捨てることである。`GameController` は、読み捨てるべき `readyok` の残り数を先手と後手の席ごとに数え、その数の `readyok` を受けるまで応答を捨てる。
`/destinations` と `/position` の計算は、ランチャーの `destination_squares` と `position_preview` がminase-coreの公開APIで行っている。`/destinations` は「その駒の利きから味方の駒がいる升を除く」だけを返し、手番や同一局面の反復は見ない。`/position` は、拡張SFEN（中将棋向けに広げた局面の文字列表記）の解析、着手の逐次適用、終局手の拒否、および「開始局面の次の手数＋着手数≦9,999」の上限を検査する。
ランチャーは、エンジンの起動時に `USI_Hash`（置換表の大きさ、MiB）と `Threads` を設定し、`EvalFile`（ネットのファイルのパス）の変更と、上限を超える `USI_Hash` と `Threads` を拒否し、上限の値を画面へ渡す役割も担っている。

### minase本体でブラウザを妨げる箇所

`wasm32-unknown-unknown` では、`std::thread::spawn` と `std::time::Instant::now` は実行時にpanicし、`std::fs` は常にエラーを返す（[rustcのプラットフォーム文書](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)）。
minaseでこれらに当たる箇所は次のとおりである。

| 箇所 | 内容 |
|---|---|
| `crates/minase/Cargo.toml` の依存 | `fs2`、`mimalloc`（Cのビルドが要る）、`clap`、`ort`、`procfs`（Linux限定）が、ライブラリのビルドにも入る |
| `search/handle.rs` の `SearchHandle::spawn` | `Threads=1` でも探索を `thread::spawn` で別スレッドに出し、停止要求を `AtomicBool` で受ける |
| `search/alphabeta/team.rs` の `run_worker_team` | 主ワーカーを含む全ワーカーを、8 MiBのスタックを持つスレッドで `thread::scope` の中で動かす |
| `search/clock.rs` の `MonotonicClock` | `Instant::now` を使う |
| `protocol/usi.rs` の `run_channel` | 標準ライブラリのチャネル `mpsc` の `recv`、`try_recv`、`recv_timeout` で入力を待ち、探索の終わりを `join` で待つ |
| `search/alphabeta/engine.rs` の `Engine::prepare` | `EvalFile` が空なら拒否し、`std::fs` でネットを読み、置換表が未確保なら既定の256 MiBで確保する |
| `nn/` と `search/mcts/` | PV-MCTS とONNX Runtimeによる推論を実装し、`search/searcher.rs`、`search/handle.rs`、`protocol/usi.rs` から参照される |

`minase-core` は通常の依存を持たず、ブラウザで使えないAPIを呼んでいない。規則照会用のwasmはこれだけに依存する。

### 探索のスタック

minaseのreleaseビルドの探索は、探索木の深さ1手（1 ply）あたり約8.2 KiBのスタックを使い、探索の延長で最大手数 `MAX_PLY`（256）の近くまで達すると約2.1 MiBを要する。
2026年11月のSPSAの調整では、`Threads=1` の探索がRustのスレッドの既定の2 MiBを超えて異常終了し、主ワーカーも8 MiBのスレッドで動かす修正を入れた（[測定記録](../measurements/adhoc/nnue-gen3-search-retune.md)の「クラッシュ」の節、`team.rs` の `WORKER_STACK_BYTES`）。
ブラウザでは、スタックは2つに分かれる。配列のようにアドレスを取る局所変数は、wasmの線形メモリ上にRustが置くスタック（シャドースタック）に載り、その大きさはリンク時に指定できる（rustcの既定は1 MiB）。それ以外の局所変数と呼び出しのフレームはブラウザのエンジンが管理するスタックに載り、アプリからは大きさを変えられない。
JSPIで中断できる呼び出しは、V8（ChromeとEdge）では既定984 KiB（64ビットのAndroidでは952 KiB）の別のスタックで、SpiderMonkey（Firefox）では既定1 MiBの別のスタックで走る。JavaScriptCore（Safari）は別のスタックを持たず、Workerのスタックの上で走らせて、中断のときにフレームをヒープへ退避する。
探索の約8.2 KiBのうちどれだけがエンジンの管理するスタックに載るかは、ビルドして測るまで分からない。8.2 KiBのすべてがそちらに載れば、約245 plyの再帰は1 MiBのスタックに収まらない。
収まらなかった場合は、溢れたのがシャドースタックかエンジンの管理するスタックかと、どのフレームが大きいかを調べてから対策を決める。シャドースタックならリンク引数で大きくする。エンジンの管理するスタックなら、大きいフレームの局所変数をply別にあらかじめ確保したヒープの領域へ移す。後者は探索の挙動を変えない変更なので、nativeでノード数の一致を確かめ、同じブラウザで検証4をやり直して取り入れる。この対策でも収まらなければ、本計画を中断して方式を見直す。

### 関係するADR

本計画に関係する既存のADR（設計判断の記録、[docs/adr/](../adr/README.md)）は次のとおりである。

- [探索はEvalFileのNNUEだけを使う](../adr/search-with-an-evalfile-nnue-whose-structure-is-chosen-on-load.md)は、PST単独、PST＋FM、および駒得評価v0（学習を始める前の手書きの評価関数）を評価関数として探索から呼ばないと決めている。本計画はWeb版に限ってこの決定を改めるので、完了時にその範囲を書き換える新しいADRを書く。
- [探索の駒の価値はv0の駒得表から取る](../adr/take-search-piece-values-from-the-v0-material-table.md)は、SEE（駒の取り合いの損得の静的な見積もり）と枝刈りが使う駒の価値を定める。これは評価関数ではないので、上のADRと矛盾せず、Web版でも変えない。
- [プロトコルの各モジュールから探索を呼ぶ](../adr/call-search-from-protocol-modules.md)は、`run_channel` の経路を台本（あらかじめ決めた入力の列を流す試験）では検査できないと書き、実バイナリでの時計つきの `go` と `stop`、`go infinite`、探索中の `quit` の確認を求めている。本計画の行単位の公開APIは、この決定を保ったまま駆動の単位を分ける。
- [ネットはファイル全体のSHA-256で識別する](../adr/identify-nnue-networks-by-the-sha256-of-the-whole-file.md)に従い、ブラウザに渡したネットも同じ値で照合する。
- [先読みの後の予算は的中の時刻から測る](../adr/measure-budget-from-ponderhit-and-add-a-quarter-after-pondering.md)は、先読みから続く探索の持ち時間の起点を最初の的中の時刻とする。
- [秒読みの余裕はミリ秒のUSIオプションで与える](../adr/give-the-byoyomi-margin-as-a-millisecond-usi-option.md)は、秒読みの期間のエンジンが「秒読み−`ByoyomiMargin`」まで考えることを定める。
- [Threadsの既定値は1](../adr/keep-default-threads-at-one.md)によれば、nativeでは `Threads=2` が `Threads=1` に対して+62.1 Elo（勝率の差を表す尺度）だった。Web版はこの差を諦める。
- [探索の調整用の係数はtuningビルドだけで公開する](../adr/expose-tunable-coefficients-only-in-tuning-builds.md)は、フィーチャ `tuning` を付けたビルドだけが探索の係数を `Tune_<名前>` のUSIオプションとして受け付けることを定める。検証4で使う。
- [ライブラリはMIT、エンジンはGPL](../adr/license-library-under-mit-and-engine-under-gpl.md)と[評価ネットはCC BY 4.0](../adr/license-evaluation-networks-under-cc-by-4-0.md)が、新リポジトリのライセンスと表示を決める。
- [crateごとに版数を付けて手作業で公開する](../adr/release-each-crate-separately-and-publish-by-hand.md)に従ってminaseをリリースする。

## 設計判断

| 論点 | 採る方式 | 棄却した代案と理由 |
|---|---|---|
| ターゲット | `wasm32-unknown-unknown`、stable Rust、共有メモリなしとする | `wasm32-unknown-emscripten` はpthreadsを使うとCOOPとCOEPの応答ヘッダー（ページを他オリジンから隔離して共有メモリを許すための指定）が必須で、Emscriptenのツールチェーン（emsdk）とJSの接着コードの固定が増える。`wasm32-wasip1` は時計が使えるがブラウザ側にWASI（wasmにOSの機能を与える標準のインターフェース）の実装が要り、探索中に入力が届かない問題は残る。 |
| 探索中の入力 | JSPIで探索を一時中断し、Workerのイベントループへ戻す | SharedArrayBuffer上の停止フラグは、隔離のないページでは作れない。coi-serviceworker（Service Workerで隔離の応答ヘッダーを付ける回避策）は、初回の再読み込みが要り、Firefoxのプライベートモードでは動かず、最終コミットが2023年12月で未解決の不具合がある。Binaryen Asyncify（コンパイル後のwasmを書き換えて中断を可能にする道具）は、[作者の解説](https://kripken.github.io/blog/wasm/2019/07/16/asyncify.html)によればコードの大きさが圧縮前で5割ほど増え得るので、JSPIで足りる以上は採らない。探索を明示的な状態機械に書き直す案は、nativeの探索の最内周を変えて棋力と速度の測定をやり直すことになる。`Worker.terminate()` は、置換表、USIの初期化、規則、設定を失うので通常の停止に使えない。 |
| 対応ブラウザ | JSPIが既定で有効なブラウザだけとする。Chrome、Edge、Android版Chromeは137以降、Firefoxは153以降、Safariは27以降（iOS 27以降）。画面は `WebAssembly.Suspending` と `WebAssembly.promising` の有無で判定し、なければ対局を始めず理由を表示する | 縮退モード（先読みを無効にし、中止をWorkerの作り直しで実現する）は、作り直しでUSIの初期化と規則が失われ、`GameController` の再初期化の経路はCECP専用であり、2つ目の対局モードは検証の組合せを倍にする。利用者が2026年11月25日にJSPI必須を選んだ。 |
| スレッド | Web版はスレッド数を1に固定し、USIで `Threads` を宣言せず、設定画面に出さない | wasmのスレッドは、nightlyのRustと共有メモリが必須で、上の方式と両立しない。`Threads` を最小1最大1で宣言すると、移植する設定画面がその値を持つ欄を表示してしまう。 |
| 評価関数 | A型NNUE（外部ファイル）と、埋め込みの `nets/pst.bin` をそのまま使うPST＋FMの2つとし、Workerの作成時に1回選ぶ | `pst.bin` は、[ファイル形式](../../crates/minase/src/eval/pst/format.rs)の式でFMの次元を32としたときのファイル長930,548バイトと一致し、FMを含む。FMを除いたPST単独は、埋め込みファイルの一部を捨てた別の評価関数になる。PST＋FMは駒の数に応じた序中盤と終盤の点数の補間とFMを持つが、A型の駒と升の点数の部分はこれらを表せないので、PST＋FMをA型の形式に変換する案も採らない。利用者が2026年11月25日にPST＋FMを選んだ。 |
| PST＋FMの組み込み方 | 既定で無効のフィーチャ `embedded-pst-search` の下で、探索の評価源にPST＋FMの枝を加え、Web版の入口からだけ選べるようにする。nativeの実行ファイルはこのフィーチャを有効にしない | nativeのUSIオプションにすると、CLI、CECP、教師生成、および対局比較の「どの評価で測ったか」の契約まで変わる。NNUE用とPST用でwasmを分けると、成果物と検証が2組になる。 |
| minase本体の切り分け | ホストに依存する依存クレートとモジュールを既定のフィーチャへ移し、実行ファイルに `required-features` を付ける。新リポジトリは `default-features = false` で依存する | 探索の各所に `target_arch = "wasm32"` の分岐を置く案は、nativeで同じ構成を試験できない。探索と評価を新しいcrateへ抽出する案は、`pub(crate)` の可視性の境界、診断、および試験を広く変更することになる。探索のソースを新リポジトリへ写す案は、正が2つになる。 |
| JSとの接着の置き場所 | 新リポジトリの薄いcrateに置き、minase本体にはJSとWorkerの知識を入れない | minaseに `crates/minase-wasm` を置く案は、wasm-bindgenとJSの事情をminaseのワークスペースと `Cargo.lock` に持ち込む。利用者は新リポジトリ1つだけである。 |
| wasmとJSの境界 | 版を固定したwasm-bindgenを使い、JSPIの配線には `#[wasm_bindgen(jspi)]` と `#[wasm_bindgen(suspending)]`（0.2.128で加わった実験的な機能）を使う | 素のimportとexportは、ポインタと長さ、解放の責任、メモリ拡張後の参照の取り直し、および中断のたびのシャドースタックの退避を、新リポジトリが自分で保守することになる。wasm-bindgenはシャドースタックの退避と復元を行う（[wasm-bindgenのJSPIの文書](https://wasm-bindgen.github.io/wasm-bindgen/reference/jspi.html)）。フェーズ2の検証が通らず、原因がwasm-bindgenにあると分かった場合は、方式を見直して本書を改訂する。 |
| エンジンと規則照会 | 2つのcdylib（他の言語から読み込めるライブラリとしてビルドするRustのcrate）に分ける。規則照会用は `minase-core` だけに依存し、ランチャーの `destination_squares` と `position_preview` を移植して、画面のメインスレッドから直接呼ぶ | 1つにまとめると、先読み中はエンジンのWorkerが塞がるので、盤のクリックと局面の編集に答えられない。規則照会を専用のWorkerに置く案は、検証7の基準を超えたときに採る。 |
| ネットの配布 | CIがHugging Face Hubの固定リビジョンからA型ネットを取得し、長さとSHA-256を照合してPagesの成果物に置く。A型が選ばれたときだけWorkerが同じオリジンから取得する。キャッシュにはHTTPキャッシュだけを使う | ネットをwasmへ埋め込む案は、PST＋FMだけを使う利用者にもネットを転送させ、ネットを更新するたびにwasmのキャッシュも無効にし、CC BY 4.0のネットとGPLのエンジンを1つのバイナリに結合することの解釈の問題も生む。実行時にHugging Faceから取得する案は、外部の配信網（CDN）とオリジン間の取得の許可（CORS）の挙動に依存する。ブラウザ内のデータベース（IndexedDB）やオリジン専用のファイル領域（OPFS）に保存する案は、数十MB以上のネットを毎回取得しないための仕組みであり、3,573,868バイトのネットにはHTTPキャッシュで足りる。 |
| 中止と再戦の区切り | 移植した `GameController` で、実際の対局を閉じる経路（利用者の中止、投了、時間切れ、および終局。`#cancel` と `#finishWithGameover`）では、先読みの有無によらず `gameover` の後に `isready` を送り、既存の排出の数え方で `readyok` までの応答を読み捨てる。予想局面が終局だったときに同じ対局を続けるための `gameover` には `isready` を加えない | 出力に対局の世代番号を付けて画面で捨てる案は、`GameController` が数える `readyok` の一部を先に捨ててしまい、次の対局の `readyok` を前の対局の分として消費させる。現行は先読みのない対局を閉じるときに `isready` を送らないので、Workerから送り出し済みの前の対局の `bestmove` が次の対局へ混入し得る。 |
| 固定の単位 | 開発中は新リポジトリがminaseの作業ブランチを git の `rev` で参照し、完了前に継ぎ目を含むminaseをcrates.ioへリリースして `=X.Y.Z` に固定する | 利用者が2026年11月25日に、リリースの版で固定することを選んだ。サブモジュールは、通常の依存として使うだけなら更新の手順が増える。 |
| ライセンス | 新リポジトリはGPL-3.0-or-later | 画面が組み込むshogigroundとエンジンがGPL-3.0-or-laterである。移植するminase-guiのコードの権利者は利用者だけである（2026年11月25日の利用者の回答）。 |
| 公開の形 | `https://stepney141.github.io/minase-chushogi-wasm/` そのものを対局画面にする | minase-gui の `main.ts` は `location.pathname` が `/play` かを見て振り分けるが、Pagesのプロジェクトサイトは `/<リポジトリ名>/` の下に置かれるので、この分岐は使えない。単画面なので経路を持たない。 |

## Workerとエンジンの間のやり取り

### 構成

Workerは、受信箱（画面から受け取ったUSIの行と、その受領時刻の列）をJSの側に持つ。時刻はすべて `performance.now()` のミリ秒で表す。
エンジンのwasmが呼ぶホストの関数は4つである。

| 関数 | 種類 | 役割 |
|---|---|---|
| `now_ms()` | 通常のimport | 現在の時刻を返す。探索の時計はこれを使う |
| `emit_line(line)` | 通常のimport | USIの応答を1行、画面へ送る |
| `take_line()` | 通常のimport | 受信箱の先頭の行と受領時刻を取り出す。空なら空を返す |
| `yield_to_host(ply)` | JSPIの中断するimport | Workerの次のタスクまで制御を譲り、その間に届いた `message` を受信箱へ積ませてから再開する。引数は照会した時点の探索の手数（ply）であり、通常のWorkerは使わず、検証4の試験用のWorkerが記録する |

`yield_to_host` は、`MessageChannel` の自分宛てのメッセージが届くまで待つPromiseを返す。`Promise.resolve()` を待つだけではWorkerの `message` イベントが処理されず、`setTimeout(0)` は繰り返すと4 ms以上に引き延ばされるからである。
エンジンのwasmが公開するのは、初期化の関数と、JSPIで包んだ駆動の関数 `drive()` だけである。Workerは、`message` を受けると行を受信箱へ積み、エンジンが実行も中断もしていなければ `drive()` を呼ぶ。`drive()` は、受信箱が空になり、かつ実行中の探索がなくなる（探索がないか、結果を保留して待っている）まで行を処理して戻る。
エンジンが実行中か中断中の間は、Workerは行を受信箱へ積むだけで、同じエンジンのインスタンスの関数を一切呼ばない。中断中のエンジンは探索器とUSIの状態を借用しており、別の呼び出しがそれらを変更すると状態が壊れるからである。

### 照会の位置と時刻

主ワーカー（探索の結果を返す役のワーカー）は、4096ノードごとに時計を確かめる関数 `check_time`（間隔は `search/alphabeta/searcher.rs` の `STOP_CHECK_INTERVAL`）を呼ぶ。その呼び出しで、前回の照会から照会の間隔として決めた実時間が過ぎていれば `yield_to_host` を呼ぶ。
この照会は、`check_time` の冒頭の2つの早期の戻り（持ち時間の上限がない探索、および先読みの的中前）より前に置く。早期の戻りの後に置くと、`go infinite` への `stop` と、`go ponder` への `ponderhit` が処理されないからである。
行が届いてから処理されるまでの待ちは、Workerが走り続けている限り、届いた時点から次の照会までの時間であり、およそ「照会の間隔＋4096ノードを探索する時間」が目安になる。これは保証ではない。`MessageChannel` で譲ったタスクより先に画面からの `message` が処理される保証はなく、背面のタブでWorker自体が止まる時間も含まないからである。
`ponderhit` の的中の時刻には、Workerが行を受け付けた受領時刻を使う。画面の時計は `ponderhit` を送った直後から進むので、エンジンが再開して行を処理した時刻を使うと、その差の分だけ持ち時間を使い過ぎる。画面が送ってからWorkerが受け付けるまでの待ちは残り、これは `ByoyomiMargin`（秒読みの残り時間から通信の遅れに備えて差し引く余裕、nativeの既定は30 ms）で吸収する。
照会の間隔とWeb版の `ByoyomiMargin` は、フェーズ2で遅延を測って決め、Web版の設定に書く。

### 探索中に届く行の処理の順序

`GameController`（minase-gui `web/src/shared/game.ts`）が探索中のエンジンへ送る制御の行は、先読みが外れたときの `stop`、先読みが当たったときの `ponderhit`、および対局を閉じるときの `gameover` の3種類である。`stop` に対して返る `bestmove` は `GameController` が読み捨てる。
`GameController` は `info string error:` で始まる行を受けると対局を失敗として止めるので、エンジンは行を1回だけ、受け付けた順に処理する。

探索中の行の扱いは、現行の `protocol/usi.rs` の `handle_searching_line` の契約を変えない。`stop` は探索を止めて結果を返す決定、`gameover` と `quit` は探索を止めて結果を捨てる決定であり、`ponderhit` はその場で処理し、2つ目の `go` は `go is already running` で拒否する。それ以外の行（`isready`、`setoption`、`position` など）は、minaseの中の保留の列に積み、探索を終えた後に受け付けた順に処理する。`gameover` も保留の列に積まれ、探索を終えた後に対局待ちの状態への移行として処理される。
再開した探索は、受信箱から1行ずつ取り出してこの処理へ渡す。止める決定をした時点で受信箱からの取り出しをやめ、残りの行は受信箱に残す。探索の関数が戻ったら、結果と探索器を回収し、`bestmove` を返すか捨てるかを決定どおりに完了してから、保留の列、次に受信箱の順に、待機中の状態で行を処理する。
探索が自然に終わっても、`go ponder` で的中も停止もまだ受けていないとき、および `go infinite` で停止をまだ受けていないときは、結果を保留して待機中の状態には戻らない（`ActiveSearch::AwaitingStop`）。このとき探索の関数は戻っているので、受信箱が空なら `drive()` も戻り、次に届いた行で保留中の状態を処理する。

次の行の列について、期待する結果を試験で確かめる。最初の5つは `GameController` が実際に作り得る列であり、最後の2つは現行のUSIの契約を確かめる列である。表の `go ponder` と `go` は略記であり、試験で流す行には探索の制限を付ける（結果を保留させる列では `go ponder depth 1`、実行中の探索に届ける列では `go ponder btime 0 wtime 0 byoyomi 1000` など）。制限のない `go ponder` は、現行のminaseが入力の誤りとして拒否するからである。

| 行の列 | 期待する結果 |
|---|---|
| `go ponder` の直後に `stop` | `bestmove` を1回だけ返す。探索が自然に終わって結果を保留していたなら保留した結果を、そうでなければ止めた探索の結果を返す |
| `go ponder` の後に `ponderhit` | 探索が実行中なら同じ探索を的中後の持ち時間で続け、`bestmove` を1回返す。自然に終わって結果を保留していたなら、その結果を1回返す |
| `go ponder` の後に `stop`、`gameover`、`isready` | `stop` に対する `bestmove` を1回返してから `gameover` を処理し、`readyok` を1回返す |
| `go ponder` の後に `gameover`、`isready` | 探索を止めて結果を捨て、`bestmove` を返さず、`readyok` を1回返す |
| 時計つきの `go` の後に `gameover`、`isready` | 探索を止めて結果を捨て、`bestmove` を返さず、`readyok` を1回返す |
| `go infinite` の後に `isready`、`stop` | `bestmove` を1回返してから `readyok` を1回返す |
| 探索中の2つ目の `go` | `go is already running` で拒否し、実行中の探索は続く |

`gameover` を含む列では、`readyok` の後に、`position` より先に `go depth 1` を送ると `go requires an active game` で拒否され、続けて `position startpos` と `go depth 1` を送ると探索が始まることも確かめる。`position` は対局中でも受理されるので、`position` の後の `go` が通るだけでは、`gameover` で対局待ちの状態へ移ったことを確かめられないからである。

## minase本体の継ぎ目

| 継ぎ目 | 形 |
|---|---|
| フィーチャ | `fs2`、`mimalloc`、`clap`、`ort`、`procfs` を任意の依存にし、PV-MCTS（`nn` と `search/mcts`）、対局ハーネス、および実行ファイルだけが使う開発用の診断とともに既定のフィーチャへ入れる。A型NNUEとαβ探索とUSIはフィーチャなしで常にコンパイルする。`search/mod.rs` の `MULTI_PV_LIMIT` は `nn::POLICY_SIZE` を参照しているので、同じ値35,856の定数にし、既定のフィーチャで両者が等しいことを確かめる単体試験を置く。nativeの `Searcher::{AlphaBeta, Mcts}` と起動時の選択は変えない |
| ネットの読み込み | バイト列から `NetworkFile`（ネットとファイル全体のSHA-256の組）を作る関数を加える。`Network::decode(&bytes)` は既にある |
| 実行入口 | 主ワーカーの探索を、呼び出したスレッドの上で走らせ、進捗を通知し、結果と探索器を返す関数として切り出し、Web版の入口だけがそれを直接呼ぶ。nativeは `Threads=1` を含めて従来どおり `SearchHandle::spawn` と `run_worker_team` の8 MiBのスレッドを通す |
| 進捗 | 反復深化の進捗の送り先（現在は `mpsc::Sender<SearchEvent>`）をコールバックにする。nativeは従来どおりチャネルへ流す |
| 時計 | Web版の入口が、ホストの `now_ms` を時計として受け取る。`go` を受け付けた時刻を原点にし、的中の時刻も同じ時計の値で表す |
| 照会 | 「照会の位置と時刻」の節のとおり、主ワーカーの時計の確認で、照会の期限が来たときにホストの照会関数を、その時点の探索の手数を添えて呼ぶ |
| USIの公開API | `protocol/usi.rs` の待機中と探索中の行の処理と、探索中の状態（`ActiveSearch::Running` と、自然に終わった結果を保留する `ActiveSearch::AwaitingStop`）を、入力1行と探索のイベント1件で駆動できる公開APIにする。現在の `start_go` は探索器の所有権を探索へ渡し、`finish_search` と `discard_search` は `recv` と `join` で探索の終わりを待つ。これを、「止めて結果を返す」「止めて捨てる」の決定と、探索器の回収および `bestmove` の出力または破棄に分け、探索の内側の照会から呼んでも自分自身を待たない形にする。nativeの `run_channel` は待ち方だけを担う |
| Web版の設定 | Web版の入口は、評価源、`USI_Hash` の既定値と上限、照会の間隔、および `ByoyomiMargin` の既定値を設定として受け取る。USIの初期化（`usi` に対して `option` の行で設定項目を宣言し、最後に `usiok` を返す往復）では、`USI_Hash` をこの既定値と上限で、`ByoyomiMargin` をこの既定値で宣言し、`Threads` と `EvalFile` を宣言しない。置換表が未確保のまま `isready` または `go` を受けたときは、256 MiBではなくこの既定値で確保する。範囲外の `setoption` と未宣言のオプションは、確保の前にエラーとして拒否する。ランチャーが担っていた検査はこの宣言と受理で行い、WorkerではUSIの行を解釈しない。Workerセッションは `GameController.engineStarted` に、`USI_Hash` の上限と、スレッド数の上限として1を渡す |
| PST＋FMの評価源 | `alphabeta::Engine` が持つネットを、準備済みの評価源（探索が使う評価関数と重みの組で、NNUEのファイルまたはPST＋FM）にする。評価源は、累算器（着手ごとに差分で更新する評価の中間値）の更新、探索の開始局面の検査、`Searcher` が保持する参照、および駒取りや成りを伴わない手（静かな手）の並べ替え（`ordering.rs` の `quiet_move_delta`）のすべてに渡す。PST＋FMの差分更新は既存の `PstAccumulator`、静かな手の差分は既存の `Pst::quiet_move_pst_delta` を使う。`Engine::prepare` は、Web版の入口で評価源が与えられていれば `EvalFile` を求めず、`isready` を繰り返しても同じ評価源を使う。nativeで `EvalFile` が空のまま `isready` を受けたときの拒否は変えない。これらはフィーチャ `embedded-pst-search` の下に置き、`eval::pst` 自体は常にコンパイルする。PST＋FMのセッションは `network_sha256` を報告せず、NNUEの内訳を表示する診断コマンド `eval` はエラーを返す |

PST＋FMを、`EvalFile` の偽の値や、ネットの読み込みに失敗したときの代わりにはしない。
A型が選ばれたとき、Web版の入口は、渡されたネットのファイル全体のSHA-256を期待値と照合し、一致しなければUSIの初期化を完了させずにエラーを返す。画面はその理由を表示して対局を始めない。
期待値は、取得元のURL、ファイル長とともに、新リポジトリの1つの設定ファイルに書き、CIの取得と照合、およびWorkerの初期化の両方がそこから読む。現在の値は次のとおりで、minase-gui の `Dockerfile` と同じである。

| 項目 | 値 |
|---|---|
| 取得元 | `https://huggingface.co/stepney141/minase-nets/resolve/31694812f777ed24fa04410f7bf40211463508a3/nnue/a/minase-nnue-sfnn_a_256x16x32_psqt-lishogi-20261124-41502087.nnue` |
| ファイル長 | 3,573,868バイト |
| SHA-256 | `415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9` |

## メモリ

minaseは、置換表とは別に、探索の履歴表を `Engine::new` で確保する。1スレッド分の大きさは、配列の長さから計算すると次のとおりである。

| 表 | 大きさの式 | 約MiB |
|---|---|---:|
| continuation history | `4 × (29×144)² × 2バイト`（`search/alphabeta/history.rs`） | 133.0 |
| continuation correction | `30×144×29×144 × 2バイト`（`search/alphabeta/correction.rs`） | 34.4 |
| pawn history | `1024 × 2×29×144 × 2バイト`（`history.rs`） | 16.3 |
| その他の補正表、capture history、butterfly history | | 1.9 |

合計は約186 MiBになる。中将棋も指せるオンライン対局サイトlishogiは、iOSでwasmのメモリの上限を自分で256 MiBに抑えている。iOSのSafariがwasmに与える上限の公式な値は見つからず、iPhoneでこの大きさが確保できるかは分かっていない。
利用者は2026年11月25日に、iPhoneを含むスマートフォンを保証の対象に含めると決めた。フェーズ1でiPhoneの実機の起動の可否とピークメモリを測る。
起動できない、または対局を最後まで指せない場合、履歴表を小さくする変更は探索の挙動を変えるので、本計画の中では行わない。その場合は本計画を待機中にして、履歴表の省メモリ化の設計書を別に起案し、棋力の測定を経て取り入れてから再開する。

`tt.rs` の `resize` は新しい置換表を確保してから古い表を捨て、wasmの線形メモリは一度広げると縮まない。`GameController` は対局ごとに `USI_Hash` を送り直すので、`USI_Hash` の上限をH MiBとすると、線形メモリは「置換表以外のピーク＋2H MiB」まで広がり得る。
`USI_Hash` の既定値と上限は、既定値16 MiB、上限32 MiBから測り始め、フェーズ2でiPhoneの実機のピークメモリを見て確定する。
これらの値は新リポジトリの1つの設定に書き、Workerの初期化でエンジンへ渡すとともに、Workerセッションが `GameController.engineStarted` に上限として渡す。画面の設定欄の既定値は `GameController` がUSIの初期化で宣言された既定値から決めるので、USIの初期化での宣言、設定欄の既定値、`setoption` を受けたときの上限の検査、および未設定のときの置換表の確保の4つが同じ値に揃う。

## 画面の移植

minase-gui の `web/src` のうち、次のものを移植する。

| モジュール | 扱い |
|---|---|
| `human-vs-ai/` の `page`、`board-input`、`input-machine`、`move-candidates` | 移植する。ヘッダーのAI対AIの画面へのリンクを除く |
| `shared/` の盤、局面編集、開始局面、設定画面、規則の設定、状態表示、棋譜、探索情報、評価値グラフ、接続ログ、ヘッダー、対局の要約、状態、型、DOMの補助、`styles.css`、および `pieces/ryoko_1kanji/` | 移植する。設定画面からスレッド数の欄を除く |
| `shared/game.ts` | 移植する。USI、時計、先読み、および排出の契約を残し、CECPの分岐（初期化の往復、`memory` と `ping`、時間の換算、再起動）を削る。対局を閉じる経路は「設計判断」の「中止と再戦の区切り」の行のとおりに改める |
| `shared/session.ts`、`shared/socket.ts` | 移植しない。同じ形（`controller` と `close()`）のWorkerセッションで置き換える |
| `shared/destinations.ts`、`shared/position-preview.ts` | 関数の型を保ち、中身を規則照会用のwasmの呼び出しにする |
| `shared/engines.ts` | 型を残し、`fetchEngines` を「A型NNUE」と「PST＋FM」の2つの固定の選択肢にする。評価関数は既存のエンジン選択の画面で選び、選んだ評価関数でWorkerを作る |
| `shared/cecp.ts`、`ai-vs-ai/` | 移植しない |
| `main.ts` | 経路による振り分けをやめ、対局画面だけを組み立てる。JSPIがなければ対局画面の代わりに理由を表示する |

規則セットの初期値は、ランチャーの起動引数 `--rules` に当たるアプリの定数で渡す。minaseは規則セットの既定値を持たないからである（[ADR](../adr/require-the-rule-set-through-the-protocol-and-keep-no-default.md)）。
対局中の合法手と終局は、これまでどおりエンジンの応答に頼り、規則照会用のwasmで審判を二重に行わない。

Vitestの単体試験のうち、`human-vs-ai/`、規則、開始局面、盤、棋譜、グラフ、および探索情報の表示の試験は移植する。`game`、`state`、`settings-panel`、`ponder` の試験は、USIのエンジン1台の場合を取り出して移す。そのうえで、先読みのない対局を閉じたときも `isready` で区切ることと、予想局面が終局のときの `gameover` には `isready` を送らないことの試験を足す。WebSocketとCECPとAI対AIの試験、およびランチャーのE2E試験は移さない。
`destinations.test.ts` と `position-preview.test.ts` は `fetch` を差し替えてHTTPの形を確かめる試験なので、そのままでは規則の計算を検査しない。minase-gui の `src/main.rs` の試験にある入力と期待値（成りの保留、終局手、手数の上限を含む）を、規則照会用のwasmに対する試験として移す。
`web/test/fixed-usi*.ts` の台本（決まった応答を返す偽のUSIエンジン）は、Worker用の偽のエンジンに作り替える。台本は対局制御の試験であり、本物の探索へ割り込めることの証拠にはならないので、実際のwasmを実際のブラウザで動かす試験を別に置く（「検証」の節）。

## ライセンスとソースの提供

新リポジトリはGPL-3.0-or-laterとする。GPLv3の第6条(d)は、対応するソースを別のサーバーに置くことを認めるが、実行物の隣に明確な案内を置くことを求めるので、画面から「この版のソース」として新リポジトリのコミットとminaseの版へたどれるようにする。
画面には、`minase-core` とchart.jsのMITの表示、A型ネットのCC BY 4.0の表示（作者、ライセンスへのリンク、取得元のURLとリビジョン、改変していないこと）、および駒の画像のCC BY-SA 4.0の表示（minase-gui `web/src/pieces/ryoko_1kanji/LICENSE.md`）を置く。

## ビルドと配備

新リポジトリは、`cargo build --locked --release --target wasm32-unknown-unknown`（2つのcdylib）、wasm-bindgen-cliによるJSの接着コードの生成（`--target web`）、Viteによる画面のビルド、およびGitHub Pagesへの配備を、GitHub Actionsの1つのワークフローで行う。
固定するものは、`rust-toolchain.toml`（Rustの版と `wasm32-unknown-unknown`）、`Cargo.lock`、Nodeの版と `package-lock.json`、同じ版のwasm-bindgenのcrateとCLI（版が違うとCLIがエラーで止まる）、およびGitHub Actionsのコミットである。
リリースのプロファイル（リンク時最適化のLTOと、コード生成の分割数のcodegen-units）と、エンジン用のwasmのシャドースタックを8 MiBにするリンク引数は、新リポジトリのワークスペースの設定に書く。Cargoは依存先の `[profile]` を無視するからであり、8 MiBはnativeの探索スレッドの `WORKER_STACK_BYTES` と同じ値である。
Viteの `base` は `/minase-chushogi-wasm/` とし、Workerは `new Worker(new URL("./engine.worker.ts", import.meta.url), { type: "module" })` の形で書く。minase-gui の `vite.config.ts` はランチャーへの埋め込み用なので写さない。
配備のジョブだけに `pages: write` と `id-token: write` の権限を与える。
ネットの取得では、「minase本体の継ぎ目」の節の設定ファイルの取得元から取得し、長さとSHA-256が一致しなければビルドを失敗させる。開発中の手元のビルドも、同じ取得と照合の手順でネットを置く。
minaseのCIには、`wasm32-unknown-unknown` 向けのビルド検査を加える。minaseの `.cargo/config.toml` は全ターゲットに `target-cpu=native` を付けるので、このジョブでは環境変数 `RUSTFLAGS` を設定して設定ファイルの `rustflags` を上書きする（環境変数が設定ファイルより優先される）。

## 実装フェーズ

minase側の作業は作業ブランチ `browser-wasm-app`（worktree `data/worktrees/browser-wasm-app/`）で行う。
フェーズ2が通らなければ、画面の移植へ進まず方式を見直して本書を改訂する。

### フェーズ0: 準備

利用者がGitHubにリポジトリ `stepney141/minase-chushogi-wasm` を作り、PagesのソースをGitHub Actionsに設定する。
ローカルでは `../minase-chushogi-wasm` に複製し、本リポジトリ直下に同名のシンボリックリンクを置く。
検証に使う端末（Linuxのデスクトップ、macOS、Androidの実機、iPhoneの実機）と、それぞれのOSとブラウザの版を、利用者と確認して記録する。
完了条件は、空のページがPagesの公開URLで表示されることである。

### フェーズ1: 同期の探索入口

minaseでは、フィーチャの切り分け、バイト列からのネットの読み込み、時計の注入、およびスレッドを使わない実行入口を入れ、CIに `minase-core` と `minase --no-default-features` のwasmのビルド検査を加える。
新リポジトリでは、版を固定したツールで2つのcdylibをビルドし、エンジンのwasmが公開する同期の探索の関数を、A型ネットを与えてWorkerから直接呼ぶ。USIの行はまだ通さない。
完了条件は、隔離のないページで、「検証」の節の対象ブラウザのそれぞれで、置換表1 MiB、深さ6の同期探索が合法な最善手を返し、初期化直後のピークメモリを記録できることである。iPhoneで初期化できなければ、「メモリ」の節に従って本計画を待機中にする。

### フェーズ2: USIの駆動、照会、および評価源

minaseでは、USIの行単位の公開API、照会、Web版の設定、およびPST＋FMの評価源を入れ、CIのwasmのビルド検査に `embedded-pst-search` を加える。
新リポジトリでは、受信箱とJSPIの照会を配線し、時刻つきのUSIの台本（決まった時刻に決まった行をWorkerへ送る試験用のページ）で実際のwasmを駆動する。
照会の間隔、Web版の `ByoyomiMargin`、および `USI_Hash` の既定値と上限を、この段階の測定で決める。
完了条件は、「検証」の節の1〜5が通ることである。

### フェーズ3: 画面

人間対AIの画面を移植し、4つの接合面を差し替え、単体試験と台本を移す。
完了条件は、「検証」の節の6〜9が通ることである。

### フェーズ4: リリースと公開

minaseの作業ブランチをmasterへ統合し、[リリースの手引き](../guides/release.md)に従ってminaseをリリースする。新リポジトリの依存をその版の `=X.Y.Z` に固定し、ネットの取得と照合、ライセンスとソースの表示を含めてPagesへ配備する。
完了条件は、「検証」の節の10が通り、「完了条件」の節の文書の作業を終えることである。

## 検証

対象ブラウザは、LinuxのChrome、Edge、Firefox、macOSのSafari、AndroidのChrome、およびiPhoneのSafariの6つとし、フェーズ0で記録した版で行う。
iPhoneのSafariのメモリは、macOSのSafariのWeb Inspectorを実機につないで測る。
測定の結果は、`docs/measurements/browser-wasm-app/` に1測定1ファイルで記録する。

1. **ビルドと評価源**：minaseで、`cargo check --target wasm32-unknown-unknown -p minase-core`、`cargo check --lib --target wasm32-unknown-unknown --no-default-features -p minase`、および同じコマンドに `--features embedded-pst-search` を付けたものが通る。nativeの既定のビルド、`cargo clippy`、`cargo test`、および `cargo test --features embedded-pst-search` が通る。後者には、探索が使うPST＋FMの差分更新の値が、各局面で全計算の `eval::pst::evaluate` と一致する試験を含める。この試験は、FMの補正が0でない局面から、通常の手、成り、獅子の2回の取り（獅子が1手で2つの駒を取る指し方）、null move（手番だけを相手に渡す探索上の仮の手）、および着手を戻して別の手を指す手順をたどる。
2. **nativeの退行**：作業ブランチの分岐元のコミットを、照合の参照、速度の基準、および直前の版の3つに与えて `scripts/bench/bench_compare.py` を `--depth 5`、A型ネットで実行し、局面ごとのノード数、最善手、評価値が一致する（終了コード0）。同じ実行は、直前の版に対する速度の比（親比）と、直前の版を繰り返し測ったときのばらつきの幅（分解能）を出力するので、両方を記録し、親比が「1−分解能」を下回らない。PV-MCTSは[デバッグの手引き](../guides/debugging.md)のPV-MCTSのbenchの比較で出力が一致する。nativeの実行ファイルで、`Threads` を1と2の両方にして、時計つきの `go` が自然に終わる場合、時計つきの `go` と `stop`、`go infinite` と `stop`、`go ponder` と `ponderhit`、および探索中の `quit` を流す。進捗の `info` 行の数は比べず、`stop` と `ponderhit` に対して `bestmove` が1回だけ返ること、`quit` では着手を返さずに終了することが、分岐元と同じである。
3. **探索中の行の順序**：「探索中に届く行の処理の順序」の表の各列を、nativeの単体試験で、行と探索のイベントの順序を固定して確かめる。探索中の状態が `Running` の場合と `AwaitingStop` の場合の両方を含める。同じ列を時刻つきのUSIの台本で、対象ブラウザのそれぞれの実際のwasmに流し、両方の評価関数で、応答の重複と欠落がない。
4. **深い再帰**：[測定記録](../measurements/adhoc/nnue-gen3-search-retune.md)の「クラッシュ」の節で異常終了した条件を出発点にする。条件は、`data/spsa/nnue-gen3-search-retune` に残る反復2でSPSAが係数を負の向きへずらした値θ−と、その局の474手目の局面である。`tuning` フィーチャを付けた、エンジンの試験用のwasmで、照会の間隔を0（4096ノードごとの時計の確認のたびに照会する）にし、`USI_Hash` をWeb版の上限にして、この条件を `go depth 19` まで探索する。試験用のWorkerは照会のたびに、その時点の探索の手数（ply）を記録する。照会は4096ノードごとにしか起きないので、深い経路を通っても200 ply以上の位置で照会が起きるとは限らない。起きなければ、同じ局の前後の局面と同じ係数で、200 ply以上の位置で照会が起きる局面を探し、見つかった局面と係数を測定記録に書いて固定する。合格の条件は、対象ブラウザのそれぞれで、異常終了せず、200 ply以上の位置での照会が少なくとも1回あり、その探索が最後まで進み、深さ19の反復までのノード数が、同じ条件で `tuning` を付けた通常の（8 MiBのスタックの）nativeのビルドと一致することである。ノード数が一致しなければ、32ビットのwasmと64ビットのnativeの違いなどの原因を調べて本書を改訂する。溢れた場合は「探索のスタック」の節の対策に進む。
5. **資源と遅延**：iPhoneのSafariで、初期化、両方の評価関数、`USI_Hash` の上限での再設定の繰り返し、および評価関数を替えてのWorkerの作り直しを時刻つきの台本で行い、wasmの線形メモリの大きさとページ全体のメモリを分けて記録する。対象ブラウザのそれぞれで、`stop` を送ってから `bestmove` を受けるまでの時間を測る。また、残り持ち時間0、秒読み1000 msの `go ponder` を流し、探索の途中で `ponderhit` を送って、送信側の同じ時計で「`bestmove` を受けた時刻−`ponderhit` を送った時刻−（1000 ms−`ByoyomiMargin`）」の正の部分を締切の超過分として測る。Web版の `ByoyomiMargin` は超過分の最大値を上回る値に決める。
6. **画面の機能**：両方の評価関数で、規則セットの選択、開始局面の編集、先後の選択、3つの時間方式（1手の固定時間、秒読みのない持ち時間、秒読みつきの持ち時間）、先読み、MultiPV、棋譜、評価値グラフ、および過去局面の探索情報が動く。この組合せは移植した単体試験と台本で確かめ、実際のwasmでは対象ブラウザのそれぞれで両方の評価関数の対局を1局ずつ最後まで指す。iPhoneのSafariでは、先読みとMultiPVを併用した対局と、評価関数を替えた再戦を最後まで指せる。検証5で締切の超過分が最も大きかった端末で、先読みを有効にした秒読み1秒の対局を20局指し、画面の時計で時間切れがない。
7. **規則照会と再戦**：minase-gui の `src/main.rs` の試験から移した入力と期待値の試験が、規則照会用のwasmに対して通る。実際に約1万手を適用する合法な棋譜（開始局面の次の手数を1とし、手数の上限まで着手を並べたもの）のプレビューで、iPhoneのSafariのメインスレッドが1回の呼び出しで塞がる時間が200 ms以内である。超えた場合は規則照会を専用のWorkerへ移す。中止の直後の再戦（先読みの有無の両方、先読みのない対局の時間切れの直後、およびエンジンが最初の `readyok` を返す前の中止を含む）で、前の対局の応答が次の対局へ混入しない。
8. **ネットの照合**：A型ネットと同じ長さで、復号はできるがSHA-256の異なるファイル（重みの1バイトを書き換えたもの）をWorkerに渡すと、USIの初期化が完了せず、画面が理由を表示して対局を始めない。
9. **非対応のブラウザと背面のタブ**：JSPIを無効にしたChromeで開くと、対局画面の代わりに理由が表示される。対局中にタブを背面にし、持ち時間を過ぎてから戻すと、画面の時計の規則どおりに時間切れで終局し、その後に届いた `bestmove` は `GameController` の排出で読み捨てられ、着手にもエラーにもならない。
10. **公開**：Pagesの公開URLを、Service Workerなしで初めて開き、対象ブラウザのそれぞれで両方の評価関数の対局と再戦ができ、画面からソースとライセンスの表示へたどれる。

## 完了条件

- 「検証」の節のすべてが通る。
- 新リポジトリが、crates.ioでリリースしたminaseの版に固定され、Pagesの公開URLで公開されている。
- [設計書の書き方](README.md#完了時の移し替えと凍結)に従い、完了後も有効な内容を移す。PST＋FMをWeb版の探索に使う範囲を定める新しいADRと、ホストに依存する層をフィーチャで切り分ける決定のADRを書き、継ぎ目の契約（照会の位置、行の処理の順序、Web版の設定）をminaseのdocコメントへ移す。AGENTS.md の「関連プロジェクト」の節に新リポジトリを加える。
- 本書の「状態」を完了の段落に書き換え、測定記録へのリンクを置き、`docs/plans/archived/` へ移してリンクを直し、docs/ROADMAP.md の状態表と現在地を更新する。

## 参考資料

- minase-gui のコミット `6c348c2` の `src/main.rs`、`web/src/shared/game.ts`、`web/src/shared/state.ts`、`web/src/shared/settings-panel.ts`、`web/src/shared/session.ts`、`Dockerfile`。
- JSPIの対応状況：[webstatus.dev](https://webstatus.dev/features/wasm-jspi)、[Chrome 137のリリースノート](https://developer.chrome.com/release-notes/137)、[Firefox 153のリリースノート](https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Releases/153)、[Safari 27のリリースノート](https://developer.apple.com/documentation/safari-release-notes/safari-27-release-notes)、[V8のJSPIの解説](https://v8.dev/blog/jspi)。
- JSPIのスタック：V8の `src/common/globals.h` の `V8_DEFAULT_STACK_SIZE_KB` と `src/flags/flag-definitions.h` の `wasm_stack_switching_stack_size`、SpiderMonkeyの `modules/libpref/init/StaticPrefList.yaml` の `javascript.options.wasm_cont_stack_size`、WebKitの `Source/JavaScriptCore/runtime/JSPIContext.h`。
- wasm-bindgenのJSPI対応：[wasm-bindgenの文書](https://wasm-bindgen.github.io/wasm-bindgen/reference/jspi.html)。
- Asyncifyの負担：[作者の解説](https://kripken.github.io/blog/wasm/2019/07/16/asyncify.html)。
- 隔離のない静的なホストで、探索が周期的にイベントループへ戻る先例：nmrugg/stockfish.js のコミット `0dc3d0a` の単一スレッド版（[README](https://github.com/nmrugg/stockfish.js/blob/0dc3d0ac03705353dad6659c0dad4597cdee4021/README.md)、[`src/emscripten/extern-post.js`](https://github.com/nmrugg/stockfish.js/blob/0dc3d0ac03705353dad6659c0dad4597cdee4021/src/emscripten/extern-post.js)）。Asyncifyで約45 msごとにイベントループへ戻り、`Threads` を1に固定する。
- 規則ライブラリをエンジンと別のwasmにする先例：Fairy-Stockfish のコミット `5283883` の [`src/Makefile_js`](https://github.com/fairy-stockfish/Fairy-Stockfish/blob/52838835f7234e29fb48fd88eb39fb3c7e46fcfa/src/Makefile_js#L27)（ffish.js）。
- lichessのブラウザ内エンジン：lila のコミット `d65f2c8` の [`ui/lib/src/ceval/engines/engines.ts`](https://github.com/lichess-org/lila/blob/d65f2c8eada202f2d6e491a5620af9fe127f8654/ui/lib/src/ceval/engines/engines.ts#L228-L245)。共有メモリのないブラウザでは単一スレッドの旧エンジンへ切り替え、ネットはJSから渡す。
- lishogiのブラウザ内解析：lishogi のコミット `268c10c` の [`ui/ceval/src/util.ts`](https://github.com/WandererXII/lishogi/blob/268c10c7f11d64ad306090c397bb38319c8d8cd3/ui/ceval/src/util.ts) は中将棋をブラウザ内解析の対象から外す。同じコミットの [`ui/ceval/src/ctrl.ts`](https://github.com/WandererXII/lishogi/blob/268c10c7f11d64ad306090c397bb38319c8d8cd3/ui/ceval/src/ctrl.ts#L124-L129) は、wasmのメモリの上限をiOSで4096ページ（256 MiB）にしている。
- GitHub Pagesの制約：[GitHub Pagesの上限](https://docs.github.com/en/pages/getting-started-with-github-pages/github-pages-limits)、[独自のワークフローによる配備](https://docs.github.com/en/pages/getting-started-with-github-pages/using-custom-workflows-with-github-pages)。
- CC BY 4.0：[法的条文](https://creativecommons.org/licenses/by/4.0/legalcode.en)。

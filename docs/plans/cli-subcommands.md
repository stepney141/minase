# 補助ツールのサブコマンド化の設計書

## 先に読む要約

エンジンのcrate`minase`は、エンジン本体の実行ファイル`minase`のほかに、対局測定、係数調整、学習データの生成、および検証と診断のための補助ツールを10個の実行ファイルとして持つ。
`cargo install minase`はcrateの実行ファイルをすべて導入するので、エンジンだけを使いたい利用者の環境にも、開発者向けの10個の実行ファイルが入る。
本書は、10個の補助ツールを実行ファイル`minase`のサブコマンドへ移し、crateが作る実行ファイルを`minase`の1つだけにする。
サブコマンドは目的別の群に分け、たとえば対局測定の`match_runner`は`minase match run`、検証用の`perft`は`minase dev perft`になる。
エンジンとしての起動方法`minase --protocol <usi|cecp> --rules <規則>`は変えない。
対局ハーネスは過去のコミットをビルドしてこの形で起動するので、起動方法を変えるとコミット対コミットの棋力測定が境界をまたいで成り立たなくなるからである。
移動は処理の中身を変えない再配置であり、既存の試験が全件通り、固定シードの各ツールの出力が移動の前後で一致し、エンジンの探索のノード数が一致することを完了条件とする。

## 状態

起案。
2026年10月5日に起案した。
対象とするツールの範囲（10個すべて）、群に分けた命名、版数の扱い（エンジン2.1.0、互換性を壊す変更として扱わない）、およびバイナリ名で呼び出すスクリプトの書き換えは、同日に利用者が決定した。
本書は可読性とコードとの整合の2系統のレビューを経ている。
次の一手は、第1フェーズのコードの移動に着手することである。

## 目的

`cargo install minase`で導入される実行ファイルをエンジン本体の1つにし、開発者向けのツールが利用者の`PATH`へ散らばらない状態にする。
あわせて、補助ツールの呼び出し方を`minase <群> <名前>`の1つの規則にそろえ、`minase --help`から全ツールを辿れるようにする。

## 適用範囲

対象は、`crates/minase/src/bin/`の11個の実行ファイル、`crates/minase/src/harness/player.rs`の`random`指定の解決、実行ファイル名で呼び出す試験、スクリプト、および文書である。
crate`minase`は、`crates/minase/src/lib.rs`を根とするライブラリ部と、`crates/minase/src/bin/`の実行ファイル群からなる。
ライブラリ部は探索、評価、プロトコル、対局ハーネス（`harness/`）、学習データの共有コード（`datagen/`）、および学習の記録形式（`training/`）を公開しており、各実行ファイルはこれを呼び出す薄い入口である。
ライブラリ`minase-core`と、`minase`のライブラリ部の公開APIは変えない。

旧実行ファイルと新しい呼び出しの対応は次のとおりである。

| 旧実行ファイル | 新しい呼び出し | 群の意味 |
|---|---|---|
| `minase --protocol … --rules …` | 変更なし | エンジン |
| `match_runner` | `minase match run` | 対局測定 |
| `match_report` | `minase match report` | 対局測定 |
| `spsa_runner` | `minase spsa` | 係数調整 |
| `selfplay_gen` | `minase data selfplay` | 学習データ |
| `lishogi_import` | `minase data lishogi` | 学習データ |
| `bench` | `minase dev bench` | 検証と診断 |
| `perft` | `minase dev perft` | 検証と診断 |
| `random_play` | `minase dev random-play` | 検証と診断 |
| `pst_probe` | `minase dev pst-probe` | 検証と診断 |
| `usi_random` | `minase dev usi-random` | 検証と診断 |

新しい呼び出しの後に続く引数と内側のサブコマンドは、旧実行ファイルのものをそのまま使う。
たとえば`match_runner … gsprt --max-pairs 3000`は`minase match run … gsprt --max-pairs 3000`に、`selfplay_gen inspect <ファイル>`は`minase data selfplay inspect <ファイル>`に、`spsa_runner apply …`は`minase spsa apply …`になる。

次のものは対象外とする。
各ツールの引数、出力の書式、終了コード、エラーの文言、および保存形式は変えない。
ただし、clapが生成する使い方の行とヘルプに現れるプログラム名は、新しい呼び出しの名前に変わる。
また、実行ファイル自身のSHA-256を記録する箇所（「実行ファイルのSHA-256の意味の変化」の節）は、記録される値が変わる。
測定記録（`docs/measurements/`）、監査報告（`docs/audits/`）、および調査記録（`docs/research/`）は当時の記録なので、旧実行ファイル名を書き換えない。

## 依存関係

着手前に、進行中の対局測定、SPSA、および学習データの生成がないことを確かめる。
対局測定とSPSAの再開は、`manifest.json`に記録したrunner（対局を進める実行ファイル）自身のSHA-256を検査する。
学習の実行（`tools/train`の`workflow.py`）の再開は、準備時に記録した`target/release/selfplay_gen`のパスとSHA-256を検査する。
作業の途中で実行ファイルが入れ替わると、どちらも再開できなくなるからである。
境界より前に準備した学習の実行（`tools/train`の`prepare`を済ませたもの）は、準備したコミットのスクリプトで完了させる。

起案中の[合法手生成と利き計算の高速化（第3期）](movegen-speedup-3.md)は、`scripts/bench_compare.py`の照合参照コミットに第2期の最終の採用版を指定している。
照合参照コミットは、候補と局面別のノード数、最善手、および探索値が一致するはずの、探索木が同じ版である。
第2期の後に探索係数の再調整（[search-revival-spsa.md](search-revival-spsa.md)）で探索木が変わったので、現在のmasterから作る候補は第2期の最終の採用版とは一致せず、第3期は着手時に照合参照コミットを選び直す必要がもともとある。
本書の後の`bench_compare.py`は境界より前のコミットを測れないので、選び直す参照は境界以後のコミットとする。
本書は探索木を変えないので、境界以後のコミットを参照にしても照合の意味は変わらない。

## 設計判断

### サブコマンドへ移す（別crateへ分けない）

利用者の決定により、10個の補助ツールをすべて実行ファイル`minase`のサブコマンドへ移す。
補助ツールが使う対局ハーネス、学習データの共有コード、学習の記録形式、および統計は、すでにエンジンのcrateのライブラリ部にある（[実行ファイルと対局ハーネスの再編](bin-harness-layout.md)）ので、サブコマンドはこれらをそのまま呼び出せる。
この方式では、ビルドの対象が`--bin minase`の1つになり、全ツールを`minase --help`から辿れる。
代償は、エンジンの実行ファイルに補助ツールのコードが加わることと、ツールの実行ファイルのSHA-256がエンジンの変更でも変わること（「実行ファイルのSHA-256の意味の変化」の節）である。

棄却した代案は2つある。
1つ目は、補助ツールを公開しない別crateへ移し、そのcrateから`minase`のライブラリ部を依存として使う案である。
ライブラリ部は必要なモジュールをすでに公開しているので（`crates/minase/src/lib.rs`）、ライブラリを割らずに実現でき、エンジンの実行ファイルも小さいまま保てる。
一方で、実行ファイルは10個のまま残り、呼び出し方の規則も統一されない。利用者は、サブコマンドで1つの入口にまとめる方式を選んだ。
2つ目は、各実行ファイルに`required-features`を付けて`cargo install`の対象から外す案である。変更は最小だが、ツールを使うたびに`--features`の指定が要り、呼び出し方の規則も統一されない。

エンジンのコマンドライン引数をサブコマンドとして受ける先例として、Fairy-Stockfishは起動引数の`bench`をエンジン本体の1回限りのコマンドとして実行する（`src/uci.cpp`の389行と419行）。

### エンジンの起動はサブコマンドなしの形のまま残す

対局ハーネスの`commit:`指定は、指定コミットを`cargo build --release --bin minase`でビルドし、`--protocol usi --rules <規則>`を付けて起動する（`crates/minase/src/harness/commit.rs`）。
エンジンを`minase usi …`のようなサブコマンドへ移すと、runnerがコミットごとに起動の形を知る必要が生じ、境界をまたぐコミット対コミットの測定が成り立たない。
そこで、サブコマンドを指定しない呼び出しをエンジンの起動とし、`--protocol`と`--rules`の必須はサブコマンドがないときだけ課す。
エンジンの引数とサブコマンドを同時に指定した呼び出し（`minase --protocol usi match run …`など）は、どちらの意味か決まらないのでエラーとする。
clap（コマンドライン引数の解析ライブラリ）では、この2つの規則を最上位のコマンドの設定`subcommand_negates_reqs = true`（サブコマンドがあれば必須の検査を外す）と`args_conflicts_with_subcommands = true`（最上位の引数とサブコマンドの併用を拒む）で表す。
ただし、必須の検査を外しても、値の型が`Option`でない欄は解析後の変換で欠落がエラーになる。
そこで、`--protocol`と`--rules`の欄は`Option`型に`required = true`を付けて宣言し、エンジンの起動へ振り分けた後で値を取り出す。
`minase`を引数なしで起動した場合と、`--protocol`または`--rules`を欠いた場合は、現在と同じくエラーで終了する。
lishogi Botの配備（別リポジトリ`minase-lishogi-bot`のDockerfile）はすでに`cargo install minase --bin minase`でエンジンだけを導入しており、本書の影響を受けない。

### 群は目的で分け、1つしかない群は作らない

群は、対局測定（`match`）、学習データ（`data`）、および検証と診断（`dev`）の3つとする。
SPSAの係数調整は該当するツールが`spsa_runner`の1つだけなので、群を作らずに`minase spsa`とする。
`minase tune spsa`のように群の段を1つ増やしても、同じ群に区別する相手がおらず、呼び出しが長くなるだけだからである。
`spsa_runner`は、内側のサブコマンドなしで調整セッションを実行し、`apply`と`params`を内側のサブコマンドとして持つ。
本書の後も、`minase spsa --run-dir …`が調整セッション、`minase spsa apply …`と`minase spsa params …`が反映と係数表の出力であり、この構造は変えない。
複数語の名前はclapの既定に合わせてケバブケース（`random-play`）とする。

### コードの配置

各ツールのモジュールは、`crates/minase/src/bin/<旧名>/`または`crates/minase/src/bin/<旧名>.rs`から`crates/minase/src/bin/minase/<旧名>/`または`crates/minase/src/bin/minase/<旧名>.rs`へ、名前を変えずに移す。
モジュール名を旧実行ファイル名のまま残すのは、`git log --follow`で履歴を辿れるようにし、モジュール内部のパスの書き換えを最小にするためである。
`crates/minase/src/bin/minase/main.rs`は、最上位の引数の定義とサブコマンドへの振り分けだけを持ち、現在のエンジン起動の処理は`crates/minase/src/bin/minase/engine.rs`へ移す。
各ツールの`main`関数は、解析済みの引数を受け取る関数に改め、引数の解析以後の処理と終了コードの決め方は変えない。
`pst_probe`が`ExitCode`を返し、`usi_random`が`io::Result`を返すといった、ツールごとの終了の仕方もそのまま保つ。
各ツールの引数の構造体は、最上位のサブコマンドの列挙子（`Match(MatchCommand)`など）の中身としてそのまま使い、定義を書き直さない。
clapでサブコマンドの中身に置けるのは`Args`を実装した型であり、各ツールの構造体が持つ`#[derive(Parser)]`は`Args`の実装も生成するからである。

解析の後で引数の組合せを検査してclapの書式でエラーを出す箇所（`match_runner`の`--ponder`の検査、`random_play`、および`perft`）は、現在はツール単独の`Arguments::command()`からエラーを作っており、使い方の行に旧実行ファイル名が出る。
これらは最上位のコマンドから該当するサブコマンドを辿った`Command`でエラーを作るように改め、使い方の行を新しい呼び出しにそろえる。
clapの引数エラーの終了コード2は変えない。

`#[global_allocator]`は1つの実行ファイルに1つしか置けず、置いたアロケータはその実行ファイルの全処理に適用される。
そこで、`main.rs`のmimallocだけを残し、`bench`、`perft`、`selfplay_gen`、および`lishogi_import`の宣言を削除する。
これにより、現在mimallocを使っていない`match_runner`、`match_report`、`spsa_runner`、`random_play`、`pst_probe`、および`usi_random`もmimallocで動くようになる。
アロケータは出力と乱数列に関わらないので、固定シードの出力の一致は変わらない。

### `random`指定の解決

対局ハーネスの`random`指定は、現在はrunnerと同じディレクトリの`usi_random`を起動する（`crates/minase/src/harness/player.rs`の`resolve_player`）。
本書の後は、runner自身（`std::env::current_exe()`）を引数`dev usi-random`で起動する。
`random`を指定した側の`manifest.json`の欄（たとえば基準側なら`baseline.identity.sha256`）には、`usi_random`のSHA-256に代わって、runnerと同じ`minase`のSHA-256が記録される。

### 実行ファイルのSHA-256の意味の変化

実行ファイル自身のSHA-256を記録する箇所は3つある。
対局測定とSPSAの`manifest.json`の`runner.sha256`は、再開時の一致検査の対象である（[SPRTの手引き](../guides/sprt.md)の「実行ディレクトリと再開」）。
`selfplay_gen rescore`は、出力のMNRSのヘッダに自身のSHA-256（`binary_sha256`）を書く（`crates/minase/src/bin/selfplay_gen/rescore.rs`）。
`workflow.py`は、生成器と診断の実行ファイルのSHA-256を来歴として記録し、再開時に照合する。
本書の後は、どのツールもエンジンと同じ実行ファイルになるので、探索や評価だけを変えたコミットでもこれらの値が変わる。
したがって、測定、付け直し、または学習の実行の途中で作業ツリーの`minase`を再ビルドすると、その実行を再開できなくなる。
これには、実行ファイルを専用のworktreeに固定する既存の運用（[生成用バイナリをworktreeに固定する教訓](../lessons/pin-generation-binary-to-worktree.md)）で対処し、検査の規則は変えない。

### スクリプトは境界以後のコミットだけを扱う

実行ファイル名で呼び出すスクリプトは、新しい呼び出しへ書き換える。
対象は、`scripts/bench_compare.py`、`tools/train/src/minase_train/workflow.py`とその試験`tools/train/tests/test_workflow.py`、および`tools/train`のうち`pst_probe`を起動する診断（`tools/train/src/minase_train/diagnostics/comparison.py`）である。
`scripts/clock_profile.py`と`scripts/match_cost_profile.py`は実行ディレクトリを読むだけで実行ファイルを呼ばないので、説明文の名前だけを書き換える。

`bench_compare.py`と`workflow.py`は、指定されたコミットをビルドしてツールを起動する。
書き換えた後のスクリプトは、どのコミットでも`cargo build --release --bin minase`でビルドし、新しい呼び出しで起動する。
境界より前のコミットの`minase`は`dev`などのサブコマンドを知らないので、起動時に引数のエラーで終了する。
コミットの内容を見て旧実行ファイルへ切り替える分岐は設けない。
境界をまたぐ比較が必要になった場合は、境界以後のコミットを基準に選び直すか、境界より前のコミットのスクリプトで旧側を測る。
これは、規則コードP0とE0を導入する前のコミットをプリセット名で指定する既存の運用（[SPRTの手引き](../guides/sprt.md)の「エンジンの指定方法」）と同じく、境界の扱いを実行者の責任とする判断である。

`workflow.py`の来歴の記録（`generator_sha256`と`probe_sha256`）は、`target/release/minase`のSHA-256を記録するように改める。
生成器を固定したworktreeでビルドした実行ファイルのハッシュ値を記録するという契約は変わらない。

### 版数と互換性

利用者の決定により、本書の変更はエンジン2.1.0の`feat`として扱い、互換性を壊す変更（`!`と`BREAKING CHANGE:`）とはしない。
エンジンとしての起動引数は変わらず、消える実行ファイルはすべて開発者向けのツールだからである。
一方、CONTRIBUTING.mdは「コマンドライン引数」の互換性を壊す変更に`!`を求めている。
この語はエンジンに限定されていないので、補助ツールの実行ファイルが消えて呼び出し方が変わる本書の変更も、文字どおりには該当すると読める。
そこで、同節の「コマンドライン引数」を「エンジンの起動引数」と改め、補助ツールのサブコマンドとその引数は互換性の対象外であることを明記する。
`minase-core`には変更がないので、ライブラリの版数は上げない。

## 実装フェーズ

### 第1フェーズ　コードの移動と振り分け

10個のツールのモジュールを`crates/minase/src/bin/minase/`の下へ移し、`main.rs`に最上位の引数とサブコマンドの振り分けを置き、エンジン起動の処理を`engine.rs`へ移す。
`#[global_allocator]`を`main.rs`の1つにまとめ、`harness/player.rs`の`random`指定を`current_exe`の`dev usi-random`へ改める。
`CARGO_BIN_EXE_match_runner`など旧実行ファイルを起動する試験（`crates/minase/tests/`の`match_runner.rs`、`lishogi_import.rs`、`debugging_tools.rs`）を、`CARGO_BIN_EXE_minase`と新しい呼び出しへ改める。
エンジンの起動引数を検査する既存の試験に、サブコマンドがあるときは`--protocol`と`--rules`を要求しないこと、およびエンジンの引数とサブコマンドを同時に指定するとエラーになることの試験を加える。
`spsa_runner`の既存の試験はツール単独の引数の型を解析しているので、最上位から`minase spsa …`、`minase spsa params …`、および`minase spsa apply …`を解析できることの試験を加える。
完了条件は、「検証」の節の試験と出力の一致がすべて成り立つことである。

### 第2フェーズ　スクリプトの書き換え

「スクリプトは境界以後のコミットだけを扱う」の節に挙げたスクリプトと試験を書き換える。
`tools/train`の既存の試験は、ビルドを偽の関数に置き換え、Rustの診断も模擬で代えているので、全件通っても実行ファイルへ渡す引数の誤りを検出できない。
そこで、ビルドのコマンドと、生成、検査、および診断の起動コマンドの引数の列を照合する試験を加える。
さらに、第1フェーズの後の`target/release/minase`を実際に起動し、小さな生成、その`inspect`、および`pst-probe`の診断を通す確認を1回行う。
完了条件は、`tools/train`の試験が全件通り、この起動の確認が成功し、`bench_compare.py`が境界以後の2つのコミットを比較して終了コード0で完走することである。

### 第3フェーズ　文書の書き換え

リポジトリ直下のREADME.md、CONTRIBUTING.md、AGENTS.md、`crates/minase/README.md`、`tools/train/README.md`、および`docs/`のうち`docs/measurements/`、`docs/audits/`、`docs/research/`を除く文書について、旧実行ファイル名による呼び出しを新しい呼び出しへ書き換える。
完了済みの設計書も、現行の手順や契約として読まれる記述は書き換える。
たとえば[実行ファイルと対局ハーネスの再編](bin-harness-layout.md)の「実行ファイル名は外部から参照されるので変えない」という記述は、本書へのリンクを付けて現在の配置に合わせる。
`cargo run --release --bin match_runner -- …`の形の標準コマンドは`cargo run --release --bin minase -- match run …`へ改める。
README.mdの実行ファイルの一覧表は、新しい呼び出しの一覧へ改める。
CONTRIBUTING.mdには「版数と互換性」の節で定めた改訂を加える。

## 検証

基点コミットは、`cli-subcommands`ブランチを切ったmasterのコミットとする。
各コードのコミットで、`cargo fmt --check`、`cargo clippy --all-targets`（警告0件）、`cargo test`、`cargo test --features tuning`、および`cargo test --features search-stats`を実行し、全件通ることを確かめる。
`search-stats`機能を有効にしたときだけコンパイルされるコードと試験が`bench`にあるので、この機能の試験も移動の対象に含める。
単体試験は旧実行ファイルごとの試験バイナリから`minase`の1つの試験バイナリへ移るので、試験名の先頭に`match_runner::`のようなモジュールパスが付く。
試験の取りこぼしと重複を見つけるため、`cargo test`の出力にある試験名から先頭のモジュールパスを除いた部分を集め、その集合が基点と一致することを確かめる。
SPSAの模擬比較の測定記録（`docs/measurements/spsa-gain-simulation.md`）は`--exact tests::simulation::gain_simulation`という試験パスを記録しているが、本書の後のパスは`spsa_runner::tests::simulation::gain_simulation`になる。
測定記録は当時のコミットでの手順なので書き換えない。

固定シードの出力の一致は、[実行ファイルと対局ハーネスの再編](bin-harness-layout.md)の「検証」の節の4つの比較を、旧実行ファイルの代わりに新しい呼び出しで行う。
すなわち、対局ハーネスの固定シードの`elo --pairs 4`、保存済みの実行ディレクトリに対する`match_report`、固定シードの小さな自己対局データの生成と`inspect`、および試験用のNDJSONに対する棋譜の取り込みである。
除外する欄は同節のとおりとし、`manifest.json`の`runner.sha256`と`baseline.identity.sha256`は、本書で実行ファイルが入れ替わることによって必ず値が変わるので除く。
ただし、同節の`match_report`用の実行ディレクトリの作り方は、両エンジンに引数のない`target/release/minase`を指定しており、そのままではエンジンを起動できない。
本書では、両エンジンに`"target/release/minase --protocol usi --rules engine-default"`を指定して作る。
これに加えて、次の4つを基点の旧実行ファイルと比べる。

1. `perft`の初期局面の深さ3の各深さの経路数と、`--divide`の出力。毎回出力される`elapsed:`と`nodes/second:`の行は除き、指し手ごとの経路数と合計を比べる。
2. `random_play`の固定シードの短い実行の標準出力。経過時間の行を除く。
3. `pst_probe`に、`tools/train`の診断（`tools/train/src/minase_train/diagnostics/comparison.py`）と同じく`--pst`に重みファイル、`--positions`に局面ファイルを与えたときのJSON出力。重みはリポジトリの`crates/minase/nets/pst.bin`、局面は前段の自己対局データの生成の比較で作ったMNSDを使う。
4. `usi_random`へ`usi`、`setoption name Seed value 1`、`isready`、`position startpos`、`go`を送ったときの応答。`usi_random`はシードを起動引数ではなく`Seed`オプションで受ける。

各コマンドの正確な引数は、基点で実行できることを確かめてから確定し、同じ引数を各コミットで使う。

エンジンの探索は変えないが、実行ファイルに補助ツールのコードが加わるため、リンク後のコードの配置が変わる。
そこで、基点の`bench`と本書の後の`minase dev bench`を深さ5、`Threads=1`で実行し、局面別のノード数、最善手、および探索値が全行で一致することを確かめる。
NPSは、同じP-coreへ`taskset`で固定して両者を交互に5回ずつ実行し、中央値の比を記録する。
比が0.98を下回った場合は統合を止め、原因を調べたうえで利用者に判断を求める。
あわせて、基点と本書の後の`target/release/minase`のファイルサイズを記録する。

文書の書き換えの後に、対象とした文書に旧実行ファイル名による呼び出し（`--bin match_runner`など10個の名前を`--bin`の引数にするもの、`target/release/<旧名>`、および旧名で始まるコマンド行）が残っていないことを`grep`で確かめる。
`cargo package -p minase --list`に`src/bin/`の下の旧実行ファイルのファイルがなく、`cargo install --path crates/minase --root <一時ディレクトリ>`が`bin/minase`だけを導入することも確かめる。

## 完了条件

「検証」の節の確認がすべて成り立ち、3つのフェーズの成果をmasterへ統合した時点で、完了（採用）とする。
NPSの比が0.98を下回った場合は、利用者の判断が出るまで完了としない。
エンジン2.1.0のリリースは、CONTRIBUTING.mdの手順に従って本書とは別に行う。

## 参考資料

- [実行ファイルと対局ハーネスの再編](bin-harness-layout.md)：現在の実行ファイルの配置と、出力の一致による検証の手順。
- [ライブラリとエンジンのcrate分割](crate-split.md)：`minase`と`minase-core`の分割と、crates.ioへの公開。
- [SPRTの手引き](../guides/sprt.md)：`commit:`指定のビルドと起動、および再開時の一致検査。
- Fairy-Stockfish（手元の複製`Fairy-Stockfish-66shogi-mod`）の`src/uci.cpp`：起動引数を1回限りのコマンドとして実行する処理。

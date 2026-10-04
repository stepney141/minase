# ライブラリとエンジンのcrate分割の設計書

## 先に読む要約

minaseは、中将棋の合法手生成ライブラリと対局エンジンを1つのcrate（Rustのパッケージ単位）に同居させており、ライセンスを持たず、`publish = false`によってcrates.ioへの登録も止めている。
本書は、このcrateを、規則と合法手生成を担うライブラリ`minase-core`と、探索、評価、プロトコル、および実行ファイルを担うエンジン`minase`の2つに分け、両者を`crates/`の下に並べる。
ライブラリはMITライセンスとし、利用者が条件をほとんど負わずに組み込めるようにする。
エンジンはGPL-3.0-or-later（GNU一般公衆利用許諾書第3版以降）とし、Stockfishややねうら王のようなGPLのエンジンを、表現の流用まで含めて参考にできるようにする。
1つのライセンスでは「自由に使ってもらう」と「GPLの既存実装を参考にする」を両立できないため、2つの望みが向く先のモジュールでcrateとライセンスを分けることで両立させる。
分割は処理の中身を変えない移動であり、深さ6のベンチの局面ごとの探索結果、全試験の名前と結果、および`perft`（指定した深さまでの合法手の経路数を数える検査）の値が分割の前後で一致することを完了条件の中心に置く。
あわせて、crates.ioへの登録を妨げている欠落（ライセンスと説明文の欄、配布物の範囲、リリース手順）を解消し、両crateの`cargo package`が検証ビルドまで通る状態で完了とする。
crates.ioへの実際の登録は取り消せない操作なので、本書の範囲に含めず、完了後に利用者が実行する。

## 状態

起案。
2026年10月3日に起案した。
設計判断はすべて確定し、判断待ちはない。
次の一手は、masterから作業ブランチを切り、フェーズ1（RULES.mdの移動と文書中の引用の処置）に着手することである。

## 目的

ライブラリとエンジンに別々のライセンスを与え、両方をcrates.ioへ登録できる状態にする。
ライブラリは、中将棋の規則を外部のプログラムが条件をほとんど負わずに使えることを目的とし、エンジンは、GPLの先行実装を参考にしやすいことを目的とする。
この2つの目的は本プロジェクトの2つの目的（合法手生成ライブラリと対局エンジン）にそのまま対応する。

## 適用範囲

対象は、`src/core/`、`src/notation/`、`src/rng.rs`、`src/test_util.rs`の`crates/minase-core/`への移動、それ以外の`src/`、`nets/`、`tests/`の`crates/minase/`への移動、ライブラリを参照するエンジン側の`use`文、Cargo.tomlとワークスペースの設定、ライセンスファイル、`tests/`の結合試験の振り分け、release.tomlとcliff.toml、CONTRIBUTING.mdのリリース手順、RULES.mdの`docs/`への移動、`scripts/`と`tools/`にあるパスの参照、および`docs/`と`AGENTS.md`にあるパスの参照である。

関数、型、定数、および試験の名前は変えない。
処理の内容、引数、処理の順序、およびインライン属性も変えない。
ただし、crateをまたぐ参照に必要な可視性の変更（`pub(crate)`から`pub`への格上げ）は行う。

crates.ioへの登録、GitHubのリポジトリ設定、および過去のタグ（v1.0.0〜v1.3.0）の扱いは対象外とする。
過去の版はライセンスを持たないまま残る。

## 依存関係

本書は、`src/core/`の再編（[core-layout.md](core-layout.md)）と実行ファイルと対局ハーネスの再編（[bin-harness-layout.md](bin-harness-layout.md)）の完了を前提とする。
ライセンスの選択は、2026年9月28日の第三者コード混入監査（[audits/third-party-code-provenance-2026-09-28.md](../audits/third-party-code-provenance-2026-09-28.md)）の所見に基づく。
この監査は、ライブラリへ移す`src/core/`と`src/notation/`に第三者のコードの複製を検出していない。
ただし、監査の「検出力と限界」の節が述べるとおり、公知の数式や広く使われる定数に依存する翻案は機械的手法では拾えず、未検出は混入がないことを意味しない。

## 設計判断

### ライブラリとエンジンの境界

ライブラリ`minase-core`には、`src/core/`（盤、駒、局面、合法手生成、規則セット、審判、直前局面生成）、`src/notation/`（SFENとUSI・CECPの指し手表記）、`src/rng.rs`（xorshift乱数）、および`src/test_util.rs`（試験用の局面生成）を置く。
エンジン`minase`には、`src/eval/`、`src/search/`、`src/protocol/`、`src/training/`、`src/datagen/`、`src/harness/`、`src/stats.rs`、および全ての実行ファイルを残す。

この境界は、現行のモジュールの依存関係から決まる。
`src/core/`は、試験と整合検査を除くと`notation`と`rng`だけを参照し、`notation`は`core`だけを参照する。
`rng`は局面のZobristハッシュの初期化（`src/core/position/zobrist.rs`）が実行時に使うので、ライブラリ側に置く必要がある。
一方、`eval`、`search`、`protocol`は`core`を参照するが、`core`はこれらを参照しない。
したがって、境界をここに引けば循環は生じない。

`notation`をエンジン側に残す代案は棄却した。
`core`の整合検査（`invariants`フィーチャ）が診断にSFENを出力するため循環が生じ、またSFENを読み書きできない規則ライブラリは外部の利用者にとって使いにくいからである。
`protocol`（USIとCECPのエンジン側の通信）は`search`に依存するため、エンジン側に残す。

### ライセンスの割り当て

ライブラリはMIT、エンジンはGPL-3.0-or-laterとする。
Apache-2.0との二重ライセンスは利用者の決定により採らない。
MIT OR GPLのような二重ライセンスは、受け取った側がMITを選べる以上GPL由来の表現を含められないため、参考にしやすさの目的を満たさず、棄却した。
全体をMITにして「着想は参考にし、表現は写さない」を規則とする代案は、写してよいかを1件ずつ監査する負担がエンジン側に残るため、棄却した。

エンジンのcrateがMITのライブラリに依存することは許される。
組み合わせて配布するエンジンの実行ファイル全体は、GPL-3.0-or-laterの条件で配布する。
エンジンの履歴にある時間管理の係数の翻案（監査の所見F3）は、GPLのエンジン側にあるので、ライセンス上の矛盾を生じない。

ライセンスの本文は、ライブラリのディレクトリに`LICENSE-MIT`、エンジンのディレクトリに`COPYING`（GPL-3.0の全文）として置く。
両crateのライセンスは、各crateのディレクトリの中だけに及ぶ。
crateの外にある`docs/`、`tools/`、および`scripts/`は、利用者の決定によりライセンスを付けず、著作権者がすべての権利を留保する。
リポジトリのルートにはライセンスファイルを置かない。
このため、GitHubはリポジトリのライセンスを検出せず、ライセンスの区分はルートのREADME.mdだけが示す。
他者は、`tools/`の学習ツールと`scripts/`のスクリプトを読むことはできるが、複製して使うことや改変して再配布することはできない。
著作権者の表記は`stepney141`とする。
リポジトリのルートのREADME.mdには、`crates/minase-core/`がMIT、`crates/minase/`がGPL-3.0-or-laterであり、それ以外のファイルにはライセンスを付けないことを1段落で書く。

### リポジトリの配置

利用者の決定により、リポジトリのルートをパッケージを持たないワークスペースのルート（仮想マニフェスト）とし、エンジンとライブラリを`crates/`の下に並べる。
ルートの`[workspace]`には`resolver = "3"`を明記する。
仮想マニフェストではメンバーのeditionからresolverが推定されず、省略すると旧resolverが開発依存のフィーチャを通常のビルドへ統合し、通常のビルドでは`invariants`を無効にするという現行の契約が崩れるためである。

```text
Cargo.toml                 [workspace] だけを持つ仮想マニフェスト
Cargo.lock
README.md                  リポジトリの説明とライセンスの区分
docs/  tools/  scripts/
crates/minase/
  Cargo.toml
  COPYING                  GPL-3.0の全文（crateの配布物に同梱）
  README.md
  src/                     eval、search、protocol、training、datagen、harness、stats、bin
  nets/                    エンジンが埋め込む評価関数の重み
  tests/                   エンジンの結合試験と試料
crates/minase-core/
  Cargo.toml
  LICENSE-MIT
  README.md
  CHANGELOG.md
  src/                     core、notation、rng、test_util の各モジュールをcrateの直下へ
  tests/                   ライブラリだけを使う結合試験と試料
```

この配置では、crateのディレクトリがそのままライセンスの境界になり、各crateの配布物はそのディレクトリの中だけで完結する。
エンジンをルートのパッケージとし、ライブラリだけを`crates/minase-core/`に置く代案は棄却した。
この代案ではエンジンのパッケージのディレクトリがライブラリ、`docs/`、`tools/`を内側に含むため、MITのディレクトリがGPLのパッケージの中に入れ子になり、配布物の`include`も除外の列挙に頼ることになるからである。
参考にしたrshogi（[参考資料](#参考資料)）も、10個のcrateを`crates/`の下に並べる仮想マニフェストをとる。

`target/`はワークスペースのルートに作られるので、実行ファイルのパス`target/release/minase`は変わらない。
仮想マニフェストのルートでパッケージを指定しない場合、`default-members`を設けなければ全てのメンバーが対象になるので、`cargo build --release --bin minase`と`cargo run --release --bin match_runner`は`-p`なしでエンジンの実行ファイルを見つける。
`default-members`は設けない。
したがって、対局ハーネスの`commit:`指定（`src/harness/commit.rs`）は、分割前後のどちらのコミットにも同じコマンドでビルドでき、手引きの標準コマンドも変わらない。

エンジンがルートから`crates/minase/`へ移ることで、次のパスが変わる。

- `spsa_runner`の`apply`に渡す`--source`の値は`crates/minase/src/search/alphabeta/params.rs`になる。`spsa_runner`の試験が`include_str!`で読む相対パスは、探索と同じcrateの中で動くので変わらない。
- `nets/`は`crates/minase/nets/`へ移す。crateの配布物はパッケージのディレクトリより外のファイルを含められないので、ルートに残すと公開できない。学習ツール（`tools/train/src/minase_train/workflow.py`）が生成器のディレクトリから`nets/pst.bin`を読む箇所と、その試験（`tools/train/tests/test_workflow.py`）が試験用の重みを書く位置は新しいパスへ改め、分割前のコミットを生成器に指定する使い方は扱わない。
- `tests/fixtures/`は、試料を使う試験とともに`crates/minase/tests/`または`crates/minase-core/tests/`へ移す。`scripts/fetch_lishogi_replays.py`の出力先も改める。
- `CARGO_MANIFEST_DIR`からの相対で`target/`の下に試験用の一時ディレクトリを作る試験（`src/bin/spsa_runner/tests.rs`）は、crateのディレクトリの中に`target/`を作ってしまうので、ワークスペースの`target/`を指すように改める。

ルートの`CHANGELOG.md`は、エンジンの変更履歴としてルートに残す。

ライブラリ内では、`crate::core::`と`crate::notation::`の2階層を残さず、`core`の各モジュール（`board`、`position`、`movegen`など）と`notation`をcrateの直下に置く。
`minase_core::core::position`のように同じ語が重なるのを避けるためである。
`src/lib.rs`が現在ルートで再公開している項目（`Position`、`Move`、`Rules`、`parse_sfen`など）は、`minase-core`のルートで同じ名前のまま再公開する。

### エンジンからライブラリへの参照

エンジンは`minase_core::`のパスでライブラリを参照し、ライブラリの型をエンジンのルートから再公開しない。
再公開すると旧パスの互換層が残り、利用者がどちらのcrateの型を使うべきかが曖昧になるからである。
このため、エンジンの公開APIから`minase::Position`などが消え、エンジンの版数を上げる理由になる（[版数とタグ](#版数とタグ)）。

### 版数とタグ

両crateは別々の版数を持ち、別々に公開する。
この方式はrshogiと同じであり、rshogiはエンジン全体のリリースに`vX.Y.Z`のタグを打ち、ライブラリ`rshogi-core`をcrates.io上で0.x系の別系列として独立に公開している。

エンジン`minase`は、現行の`vX.Y.Z`のタグの系列を引き継ぎ、分割後の最初のリリースを2.0.0とする。
分割によってライブラリの型の公開パスがすべて`minase::`から`minase_core::`へ移り、エンジンの公開APIとの互換性が失われるので、semver（意味的バージョニング）に従って主版数を上げる。
CONTRIBUTING.mdの規約に従い、公開パスを移すコミットにはtypeの直後に`!`を付け、本文に`BREAKING CHANGE:`フッターを書く。
ライブラリ`minase-core`は0.1.0から始める。
`src/core/`は2026年9月27日の再編で公開パスを変えたばかりであり、semverの0.x系はAPIが安定していないことを利用者に示すからである。
ライブラリのタグは、cargo-releaseの既定値に従い`minase-core-vX.Y.Z`とする。
cargo-releaseは、リポジトリのルート以外に置かれたcrateのタグに既定で`{{crate_name}}-`の接頭辞を付けるため、エンジンのタグも何も指定しなければ`minase-v1.4.0`の形になる。
エンジンには`tag-prefix = ""`を指定し、現行の`vX.Y.Z`の系列を保つ。

エンジンはライブラリに`minase-core = { version = "0.1.0", path = "../minase-core" }`の形で依存する。
crates.ioへの公開時には`path`が除かれて`version`だけが残るため、ライブラリをエンジンより先に公開する必要がある。

### リリース手順と変更履歴

cargo-releaseを`-p`（`--package`）で1つのcrateずつ実行する。
release.tomlには`publish = false`を加え、crates.ioへの公開は、タグの確認とpushの後に利用者が`cargo publish -p <crate>`で行う。
現行の手順がpushを利用者に残している（`push = false`）のと同じく、取り消せない操作を自動化しないためである。

変更履歴は、エンジンをルートの`CHANGELOG.md`、ライブラリを`crates/minase-core/CHANGELOG.md`に分ける。
cliff.tomlの`tag_pattern`は現在`v[0-9].*`であり、行頭に固定されていないため、`minase-core-v0.1.0`のようなライブラリのタグにも一致してしまう。
エンジンの変更履歴の区切りがライブラリのタグで切れないよう、`^v[0-9]`へ改める。
タグのパターンは区切りを決めるだけで、収録するコミットを選ばない。
そこで、エンジンの変更履歴は`--include-path 'crates/minase/**'`、ライブラリの変更履歴は`--include-path 'crates/minase-core/**'`と`--tag-pattern '^minase-core-v'`を指定したgit-cliffで生成し、各crateの変更履歴にはそのcrateのディレクトリに触れたコミットだけを載せる。
両方に触れたコミットは、両方の変更履歴に載る。
release.tomlの`pre-release-hook`は、cargo-releaseが渡す環境変数`CRATE_NAME`によってこの2通りを切り替える。
cargo-release 1.1.6の実装（`src/steps/hook.rs`）では、フックは各パッケージのディレクトリで実行されるので、出力先は環境変数`CRATE_ROOT`と`WORKSPACE_ROOT`を基準に指定する。

### 配布物の範囲

各crateの`include`で配布するファイルを限定する。
エンジンは`src/`、`nets/`、`tests/`、`README.md`、および`COPYING`、ライブラリは`src/`、`tests/`、`README.md`、`CHANGELOG.md`、および`LICENSE-MIT`とする。
`docs/`は両crateのディレクトリの外にあるので、配置そのものによってどちらの配布物にも入らない。
`docs/`には、監査の所見F2のパッチとF4の引用のようにGPLのコードを再現した文書があるので、この分離は意図したものである。
`include`を明示するのは、crateのディレクトリに将来置かれた作業ファイルが配布物に混入するのを防ぐためである。
エンジンの`nets/pst.bin`は本体のコードが`include_bytes!`で埋め込むので、除くと検証ビルドが失敗する。
`tests/`の試料は試験だけが読むので、`cargo package`の検証ビルドは試料の欠落を検出しない。
試料を同梱するのは公開されたcrateで試験を再実行できるようにするためであり、同梱の確認は`--list`の出力で行う（[検証](#検証)の5）。

両crateに`description`、`license`、`repository`、および`readme`の欄を設ける。
`repository`は`https://github.com/stepney141/minase`とする。

### RULES.mdの置き場所

利用者の決定により、RULES.mdを`docs/`の下へ移す。
docs/README.mdは`docs/`の直下に置く文書をREADME.mdとROADMAP.mdに限るため、新しいディレクトリ`docs/rules/`を設けて`docs/rules/RULES.md`とし、docs/README.mdの分類表に行を加える。
ライブラリのREADME.mdとrustdocは、RULES.mdをGitHubのURLで参照する。
crateの配布物はパッケージのディレクトリより外のファイルを含められないため、RULES.mdはcrates.ioには同梱されない。

RULES.mdを参照する109ファイルのうち、`[...](RULES.md)`の形のリンクと`AGENTS.md`の`@RULES.md`は新しいパスへ書き換える。
「RULES.md第15条」のように文書名として言及する本文は、ファイル名が変わらないので書き換えない。

RULES.mdは`docs/`の一部としてライセンスを持たない。
規則そのものは事実と着想であって著作物ではないので、他者がRULES.mdを読んで自分の実装を書くことは妨げられない。
妨げられるのは、RULES.mdの文章の転載と改変である。

### 試験と整合検査

`test_util`は、ライブラリの試験が23ファイル、エンジンの試験が約11ファイルで使う。
また、探索の試験（`src/search/alphabeta/tests/captures.rs`と`ordering.rs`）は、合法手生成の試験モジュール`core::movegen::tests`にある`capture_test_positions`、`capture_test_rules`、および`CAPTURE_EDGE_SFENS`を直接使う。
`core::movegen::tests`は`#[cfg(test)]`の下にあり、エンジンから依存crateとしてビルドしたライブラリには存在しないので、可視性を変えるだけではエンジンの試験がコンパイルできない。
そこで、この3項目を`test_util`へ移し、ライブラリに`test-util`フィーチャを設けて、`test_util`を`#[cfg(any(test, feature = "test-util"))]`の下で公開する。
ライブラリ自身の試験では`cfg(test)`により、エンジンの試験では開発依存（dev-dependency）で有効にした`test-util`により、同じモジュールが使える。
エンジン側に複製する代案は、2つの複製が乖離する危険があるため棄却した。

整合検査の`invariants`フィーチャは、ライブラリとエンジンの両方に置く。
エンジンの`invariants`は`minase-core/invariants`を有効にし、探索側の検査も有効にする。
現行のCargo.tomlは、自分自身を`invariants`付きで開発依存に加える方法で`cargo test`の整合検査を常に有効にしている（[debugging-tools.md](debugging-tools.md)）。
両crateで同じ方法をとる。
この開発依存はcrates.ioへの公開時に除かれるため、公開されたcrateで`cargo test`を実行しても整合検査は有効にならない。

`tests/`の結合試験は、次の2条件をともに満たすものだけを、試料とともに`crates/minase-core/tests/`へ移す。
1つ目は、ライブラリへ移るモジュールの項目だけを使うことである。
2つ目は、`CARGO_BIN_EXE_*`によるエンジンの実行ファイルの起動を含まないことである。
`lishogi_replay.rs`と`predecessor_search.rs`はこの条件を満たす見込みである。
`io_log.rs`と`match_runner.rs`は`CARGO_BIN_EXE_*`でエンジンの実行ファイルを起動し、`lishogi_import.rs`と`debugging_tools.rs`は`training`または`protocol`を使うので、エンジン側に残す。

`[lints]`の設定は`[workspace.lints]`へ移して両crateが`lints.workspace = true`で継承し、`[profile]`の設定はワークスペースのルートのCargo.tomlへ移す。
CargoはメンバーのCargo.tomlにある`[profile]`を無視するためである。
`.cargo/config.toml`の`target-cpu=native`はワークスペース全体に適用され、変更しない。

### 文書にある第三者のコードと文章

`docs/`にライセンスを付けないため、`docs/`が再現している第三者のコードと文章は、元のライセンスの条件か、著作権法第32条の引用の要件で正当化する必要がある。
監査の所見F4は、`docs/`とルートの文書に第三者のコードや文章を再現した箇所を29件挙げ、そのうちGPL-3.0系のコードの再現が8件ある。
最大のものは、`docs/research/forward-pruning-prior-art.md`がStockfishの`src/search.cpp`から計12行を逐語で転記した箇所である。
ほかに、XBoardのエンジン仕様書（CC BY-ND 4.0）からの10行と、Chess Programming Wikiの和訳（CC BY-SA 3.0）がある。
和訳は翻案にあたり、CC BY-SAの許諾に頼るなら翻案部分に同じライセンスを付ける必要がある。ただし、著作権法第43条は第32条の引用に伴う翻訳を認めるので、引用の要件を満たす訳文は許諾に頼らずに置ける。
所見F2の`docs/measurements/magic-bitboard-prototype/diagonal.patch`は、Stockfishの`init_magics`に固有の工夫を含む不採用の試作である。

そこで、監査の一覧（`docs/audits/third-party-code-provenance-2026-09-28/quotes/quotes.tsv`）の29件を1件ずつ、引用の4要件（必然性、主従関係、明瞭区別、出所明示）に照らして直す。
利用者の決定により、調査メモと設計書の引用は、ソースコードの逐語の引用と英語の文章の訳を含めて、出所を明示すればそのまま残す。
出所の明示には、引用元のURL（版を固定したリンク）、版、および元のライセンスを含める。
欠けているものだけを補い、原文を照合できず出所を示せない引用は、引用符を外して要旨に改める。
`diagonal.patch`からは、Stockfishの`init_magics`と同じ工夫を持つ生成器`tools/generate_diagonal_magics.rs`の1ファイルだけを除く。
生成済みの定数表と残りの差分はminase自身のコードであり、測定の再現手順が適用する差分なので残す。
測定記録には、除いた経緯とStockfishの該当箇所へのリンクを書き足す。
測定記録は凍結する文書だが、この変更は記録の内容ではなく配布できない素材の除去なので、例外として扱う。

この処置は現在のファイルにだけ及び、git履歴に残る過去の版は書き換えない。
履歴を書き換えると、設計書と測定記録が参照するコミットハッシュがすべて無効になるからである。
過去の版にある転記は、履歴とともに配布され続ける。

## 文書のパス参照

`docs/`の監査報告と測定記録を除く現行の文書（`docs/`の残り、`AGENTS.md`、`CONTRIBUTING.md`、`README.md`）は、ライブラリへ移るパス（`src/core/`など）を26ファイルで計213回、エンジンに残るパス（`src/search/`など）を62ファイルで計424回、`nets/`と`tests/`のパスを24ファイルで参照している。
完了済みの設計書は現行設計の正として保守する規約なので（[README.md](README.md)）、これらを新しいパスへ機械的に置き換える。
`docs/audits/`の監査報告と`docs/measurements/`の測定記録は、ある時点の状態を記録して凍結する文書なので、書き換えない。

## 実装フェーズ

フェーズ1では、RULES.mdを`docs/rules/RULES.md`へ移し、リンクと`@RULES.md`を書き換え、docs/README.mdの分類表に行を加える。
あわせて、[文書にある第三者のコードと文章](#文書にある第三者のコードと文章)の29件を直し、`diagonal.patch`から生成器を除く。
文書だけの変更なので、Claudeが行う。
完了条件は、`docs/`、`AGENTS.md`、`src/`、`tests/`のどこにも旧パスへのリンクが残らず、29件のそれぞれについて、出所を補ったか要旨に改めたかとその理由が説明できることである。

フェーズ2では、仮想マニフェストのワークスペースを作り、`src/core/`、`src/notation/`、`src/rng.rs`、`src/test_util.rs`を`crates/minase-core/src/`へ、残りの`src/`、`nets/`、`tests/`を`crates/minase/`へ移し、エンジンの`use`文を`minase_core::`へ書き換え、`core::movegen::tests`の共有項目を`test_util`へ移し、`scripts/`と`tools/`のパスを改める。
実装はCodexに委任する。
完了条件は、[検証](#検証)の1から4と6が通ることである。

フェーズ3では、ライセンスファイル、`include`、メタデータの欄、release.toml、cliff.toml、およびCONTRIBUTING.mdのリリース手順を整え、`docs/`のパス参照を置き換える。
Cargo.tomlと設定ファイルの変更はCodexに委任し、CONTRIBUTING.mdと`docs/`の変更はClaudeが行う。
完了条件は、[検証](#検証)の5と7が通ることである。

## 検証

1. 深さ6のbench（`cargo run --release --bin bench -- --depth 6 --threads 1 --repetitions 1`）の局面ごとの`depth`、`nodes`、`best`、および`score`が、分割前のコミットと一致する。経過時間と毎秒ノード数は比較しない。比較には`scripts/bench_compare.py`を使う。
2. `cargo test --workspace`の全試験が通り、試験の名前の集合が分割前と一致する。比較の前に、分割前の試験のパスから`core::`と`notation::`の接頭辞の変化を対応づけ、crateの名前の違いを除く。
3. `cargo run --release --bin perft -- 4 --rules <規則セット>`の深さ1から4までの値が、規則セット`engine-default`と`lishogi`で分割前と一致する。
4. `cargo test -p minase-core`と`cargo test -p minase`がそれぞれ単独で通る。通常のビルドで`invariants`と`test-util`が有効にならないことを、開発依存を除いた`cargo tree -p minase -e normal,build,features`で確かめる。`search-stats`と`tuning`を有効にしたビルドと試験も通る。ワークスペース全体のフィーチャ統合が、単独のビルドの不備を隠すのを防ぐためである。
5. `cargo package --workspace`が両crateのパッケージ化と検証ビルドを通し、`--list`の出力が、`include`で指定したファイルと、Cargoが自動で加えるファイル（`Cargo.toml`、`Cargo.toml.orig`、`Cargo.lock`、`.cargo_vcs_info.json`）だけを含む。エンジンの出力には`nets/pst.bin`と`tests/fixtures/`が含まれる。
6. 学習ツールの試験（`tools/train/README.md`の手順）が通る。
7. `cargo release -p minase-core`と`cargo release -p minase`の予行（`--execute`なし）が、それぞれ正しい版数とタグ名（`minase-core-v0.1.0`と`v2.0.0`）、および各crateのディレクトリに触れたコミットだけを含むリリースノートを表示する。

## 完了条件

[検証](#検証)の7項目がすべて通り、分割したコードがmasterへ統合された時点で完了（採用）とする。
crates.ioへの登録は完了条件に含めない。

## 参考資料

- 第三者コード混入監査: [audits/third-party-code-provenance-2026-09-28.md](../audits/third-party-code-provenance-2026-09-28.md)
- rshogi（SH11235、2026年10月3日参照）: [https://github.com/SH11235/rshogi](https://github.com/SH11235/rshogi)。ワークスペースは`crates/`の下に10個のcrateを並べる仮想マニフェストであり、探索とNNUEを含む`rshogi-core`をGPL-3.0-or-laterで、CSA形式の解析器`rshogi-csa`をMITで提供する。CHANGELOG.mdの冒頭は、`vX.Y.Z`のタグをエンジン全体のリリースの目印とし、crates.io上の`rshogi-core`を0.x系の別系列として独立に公開すると定める。
- cargo-releaseの設定リファレンス: [https://github.com/crate-ci/cargo-release/blob/master/docs/reference.md](https://github.com/crate-ci/cargo-release/blob/master/docs/reference.md)。ワークスペースのメンバーの`tag-prefix`の既定値は`{{crate_name}}-`である。
- Cargoのマニフェストの`include`と公開時の`path`依存の扱い: [https://doc.rust-lang.org/cargo/reference/manifest.html](https://doc.rust-lang.org/cargo/reference/manifest.html)

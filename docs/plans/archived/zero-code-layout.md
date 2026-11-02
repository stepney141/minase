# AlphaZero型探索のコードの配置と試験専用コードの整理の設計書

## 先に読む要約

本マイルストーンでは、[AlphaZero型探索の計画](../alphazero.md)で追加したコードのうち、置き場所が責務に合わないものを移し、役目を終えた測定用のツールを削除し、試験専用のコードを本番のコードから分離する。
AlphaZero型探索は、方策と価値を出力するニューラルネットで導くモンテカルロ木探索（PV-MCTS）と、その探索の自己対局から学習する仕組みである。
移す主なものは、次の3つである。
1つ目は、自己対局生成器の中にある「王手限定の3手以内の勝ちの判定」（手番側が王手だけを続けて3手以内に勝ちを強制できるかを調べ、できれば対局を打ち切る処理）である。
2つ目は、αβ探索のディレクトリにあり、探索方式の間で共有されている調整係数の表である。
3つ目は、学習ツールの側にあり、エンジンの試験も読む固定局面集である。
削除するのは、実現性の測定のために作ったcrate `tools/train/zero/feasibility/` である。
ただし、毎サイクル使う集計ツールだけは`minase`のサブコマンドへ移す。
試験専用のコードの分離では、本番の型と関数の中にある`#[cfg(test)]`のフィールド、メソッド、および分岐を、試験用の子モジュールへ移すか、本番と同じ経路で検査する形に改める。
移したファイルを指す`docs/measurements/`のパスは、新しいパスへ書き換える。
探索の結果を変える変更は含まないので、棋力の測定は行わない。
完了条件は、αβ探索のbenchと自己対局の出力が整理の前後で一致し、ローカルの検査がすべて通り、対象の`#[cfg(test)]`の項目と古いパスが残っていないことである。

## 状態

> 本書は完了時点で凍結した記録であり、以後の変更を反映しない。現行の仕様はコードのdocコメントに、設計判断は[docs/adr/](../../adr/README.md)にある。

完了（採用）。2026年11月2日に完了した。
探索の結果を変える変更を含まず、αβ探索のbench（52,746ノード）と自己対局の出力が整理の前後で一致した（[挙動の一致の確認](../../measurements/zero-code-layout/zero-code-layout-behavior-check.md)）。
設計判断は[測定用の道具の削除のADR](../../adr/delete-finished-zero-measurement-tools-and-estimate-cycles-by-running-one.md)、[試験専用のコードのADR](../../adr/keep-test-only-code-out-of-production-types-and-functions.md)、および[3手の判定のADR](../../adr/stop-zero-self-play-at-check-only-wins-within-three-plies-found-under-the-search-module.md)にある。
本書が定めていなかった事項は、着手時にClaudeが決めて「着手時に補った決定」の節に記した。

## 目的

AlphaZero型探索の6本のサブタスクでは、それぞれの設計書が定めた場所へコードを追加した。
実装がそろった段階で見直すと、配置に3種類の問題がある。

第1は、責務と場所の不一致である。
王手限定の3手以内の勝ちの判定`zero_selfplay/mate.rs`は、minase-coreだけに依存する探索なのに、実行ファイルのソースの中にある。
共有の時間配分`search/budget.rs`は、αβ探索のモジュール`search/alphabeta/params.rs`の係数を読むので、MCTSがこれを経由してαβ探索に依存している。
固定局面集`tools/train/zero/fixtures/nn-positions.jsonl`は、生成器と試験がcrate `minase`にあるのに、crateの外に置かれているので、crate単体では試験の入力がそろわない。

第2は、役目を終えたコードである。
`tools/train/zero/feasibility/`には、入力を95面に固定したままの`trt_bench`のように、現行のネットと合わなくなったツールがある。
使う予定のないコードを残すと、開発者はそれを現行の手順と誤解し、保守の対象も増える。

第3は、試験専用のコードの混入である。
`search/mcts/engine.rs`のフィールド`reused`や、`search/mcts/tree.rs`の`#[cfg(not(test))]`の分岐は、試験と本番で異なるコードを作るので、試験が本番の挙動を確かめていない部分が生じる。

本マイルストーンの成果は、モジュールの場所から責務が分かる配置と、以後の実装が従う試験専用コードの規則である。

## 適用範囲

本マイルストーンでは、次の作業を行う。

- 「移動の一覧」節のファイルと項目を移す。
- 重複する`softmax`の実装を`crate::nn`の1か所にまとめる。
- `tools/train/zero/feasibility/`と`tools/train/zero/nn_torch_reference.py`を削除する（「ツールごとの判定」節）。
- 集計ツール`teacher_stats`を`minase data teacher-stats`へ移し、強化学習の実行器`minase_zero run-cycles`の引数を改める（「`teacher_stats`の移動」節）。
- 「試験専用のコードの分離」節の項目を分離し、規則を[テストの手引き](../../guides/testing.md)へ加える。
- [詰み探索（df-pn）の設計書](../mate-search.md)の置き場所を改める。
- 「文書のパスの書き換え」節のとおりに文書を書き換える。

次の項目は対象外とする。

- `tools/train/zero/mnsd_reader/src/scan.rs`が、教師データの形式MNSDのバイト列を`crates/minase/src/training/records.rs`とは別に解釈している重複は、直さない。解消には公開APIの設計が要るので、別の設計書で扱う。
- αβ探索などにある`#[cfg(test)]`の項目（`search/alphabeta/`の35か所と、`search/mod.rs`、`search/snapshot.rs`、`eval/`の5か所）は動かさない。AlphaZero型探索の計画より前のコードであり、同じ規則を別の設計書で適用する。
- `#[cfg(test)] mod tests`のファイル内モジュールは動かさない。[テストの手引き](../../guides/testing.md)が標準の置き方と定めており、リポジトリの95ファイルが使っている（`git grep -l '^mod tests {'`の件数）。
- `search/handle.rs`の公開関数`search`は動かさない。αβ探索専用だが、この計画より前からあり、公開パスを変える利点が小さい。
- `tools/train/zero/gumbel_reference.py`と`tools/train/zero/nn_compare.py`は動かさない。crateの配下へ移すと、crates.ioへ配布するPythonのファイルが増えるからである。
- `zero_selfplay/game.rs`の`gumbel_noise`（Gumbel AlphaZeroの乱数の変換）は動かさない。同じ変換の複製があった`driver_cost`を削除すると、呼び出し元が1つになるので、共有のために移す理由がない。

## 依存関係

本書はコードの配置だけを変え、探索、学習、および規則の定義を変えない。
配置の方針は、[補助ツールのサブコマンド化のADR](../../adr/run-developer-tools-as-engine-subcommands.md)と[探索方式の共存のADR](../../adr/select-the-search-method-at-startup-through-a-closed-enum.md)に従う。
公開パスの扱いは、[移動した項目を古いパスで再公開しないADR](../../adr/do-not-re-export-moved-items-at-old-paths.md)に従う。
試験専用の項目を別の実行ファイルの試験へ公開するフィーチャ`test-util`は、minase-coreが同じ目的で持つものであり（[ライブラリとエンジンの分割のADR](../../adr/split-rules-library-from-engine.md)）、本書はその形に合わせる。

マージには、実行中の処理との順序の制約がある。
強化学習の実行器は本体の作業ツリーの`tools/train/zero`から起動し、対局ハーネスは本体の`target/release/minase`を対局ごとに起動する。
実行中にファイルが置き換わると、1つの実行の中に異なる版が混ざるおそれがある。
そのため、実装は`data/worktrees/zero-code-layout`のworktreeで行い、本体の作業ツリーでは行わない。
masterへのマージは、次の処理が動いていないことを`pgrep`で確かめてから行う。

- 本体の作業ツリーから起動した`minase_zero run-cycles`と起動スクリプト`launch-session.sh`。
- 本体の`target/release/minase`を使う`minase match run`と、その起動元のスクリプト。
- vast.aiの係数調整（SPSA、同時摂動確率近似）のセッション。

マージの後、次の強化学習のセッションを起動する前に、本体で`cargo build --release --bin minase`を実行する。
起動する作業ツリーと、借りた機械へ展開するコミットの両方が、本書の整理を含んでいなければならない。
実行器は作業ツリーの版で動き、借りた機械では展開したコミットのソースを読むので、片方だけが古いと固定局面集のパスが合わない。

係数調整の結果を取得する`scripts/vast-spsa/fetch.py`は、調整の対象のコミットから`git show`で係数表を読む。
新しい版のパスは本書の整理を含むコミットにしか存在しないので、本書より前のコミットで始めたセッションは、そのコミットの`fetch.py`で取得を終える。
新しい版の`fetch.py`は新しい配置のコミットだけを受け付け、古いコミットにはエラーで終了する。

## 設計判断

| 項目 | 決定 | 棄却した代案 |
|---|---|---|
| 3手以内の勝ちの判定の場所 | `crates/minase/src/search/mate_in_three.rs`へ移し、`minase::search::mate_in_three`として公開する。依存はminase-coreだけのままとし、[詰み探索（df-pn）](../mate-search.md)とコードを共有しない | 生成器の中に残す。探索なのに`search`の下に見つからず、対局用の探索からも呼べない |
| df-pn（深さ優先の証明数探索）の場所 | [詰み探索の設計書](../mate-search.md)の置き場所を`crates/minase/src/mate/`から`crates/minase/src/search/mate/`へ改める | `src/mate/`のままにする。3手の判定と別の階層に分かれ、`crate::search`の王駒の判定を使うこととも合わない |
| 共有の調整係数 | `search/alphabeta/params.rs`をファイルごと`search/params.rs`へ移す。表の内容と順序は変えない | 時間配分の6係数だけを別の表へ分ける。`protocol/usi.rs`の列挙と設定、および`minase spsa apply`が2つの表を扱う必要が生じ、変更が大きい。ファイルごと移すとαβ探索専用の係数も共有の階層に置かれるが、このファイルの冒頭の説明は、探索と時間管理の両方の係数の表としている |
| 固定局面集の場所 | `crates/minase/tests/fixtures/nn-positions.jsonl`へ移し、Pythonの学習器もこのパスを読む。正本は1つとする | 学習ツールの側に残す（crate単体で試験できない）。両方に複製する（内容が食い違うおそれがある）。ファイルは2.0 MBでgzipでは0.35 MBであり、crates.ioの上限10 MBに対して既存の重みと合わせても余裕がある |
| 重複する`softmax` | `search/mcts/mod.rs`の実装を消し、`crates/minase/src/nn/policy.rs`の`softmax`を使う | 探索の側に残して`nn`の側を消す。ネットの出力の扱いは`nn`の責務であり、`search`から`nn`への依存は既にある |
| 測定用のcrate | `tools/train/zero/feasibility/`を全部削除する（「ツールごとの判定」節）。必要になれば履歴から取り出す | 名前を`measure`などへ改めて残す。残すツールに使う予定がない |
| `teacher_stats` | `minase data teacher-stats`へ移す | 測定用のcrateに残す（毎サイクル使うツールの置き場所として合わず、学習の前に別のビルドが要る）。Pythonで書き直す（MNSDを読む実装が増える） |
| 試験専用のコード | 本番の型と関数の中に`#[cfg(test)]`の項目を置かない。試験用の子モジュールへ移すか、本番と同じ経路で検査する | 試験のために本番の抽象を増やす（`Engine`の推論器の生成をトレイトにする案など）。子モジュールは親の非公開のフィールドを読めるので、抽象を増やさずに分離できる |
| 試験用の評価器`UniformEvaluator` | `crates/minase/src/nn/test_util.rs`へ移し、`#[cfg(any(test, feature = "test-util"))]`で公開する。実行ファイルの試験は開発依存のフィーチャで使う | `#[cfg(test)]`だけにする（実行ファイルの試験はライブラリの`cfg(test)`を見られない）。各試験に複製する（2つの試験の単位に同じコードが増える） |

## 移動の一覧

以下の`src/`は`crates/minase/src/`を指す。
コードの参照の漏れは、「検証」節の6の検索語で確かめる。

| 現在 | 移動先 | 一緒に直す参照 |
|---|---|---|
| `src/bin/minase/zero_selfplay/mate.rs` | `src/search/mate_in_three.rs` | `zero_selfplay`の`game.rs`、`error.rs`、`mod.rs` |
| `src/bin/minase/zero_selfplay/mate_tests.rs` | `src/search/mate_in_three/tests.rs` | |
| `src/search/alphabeta/params.rs` | `src/search/params.rs` | `search/alphabeta/`の各所、`protocol/usi.rs`の`tuning`限定の参照、`spsa_runner/tests.rs`の`include_str!`、`scripts/vast-spsa/`、`params.rs`の中の文書リンク（`super::searcher::Searcher`は移動後に別の型を指すので`alphabeta::searcher::Searcher`へ直す）、`docs/guides/spsa.md`。モジュールは`pub(crate)`なので、公開APIは変わらない |
| `src/search/budget_tests.rs` | `src/search/budget/tests.rs` | `*_tests.rs`の形は本計画で増えた3件だけなので、`X/tests.rs`の形へそろえる |
| `src/search/handle.rs`の`start_search_with_clock` | `src/search/alphabeta/engine.rs` | `search/alphabeta/tests/`の各所。この関数はαβ探索の`Engine`だけを受け取る。`SearchHandle::spawn`は今の可視性で呼べる |
| `src/nn/mod.rs`の`NnError`とその実装 | `src/nn/error.rs` | `search`と`mcts`の`error.rs`にそろえる。`pub use`で`minase::nn::NnError`のまま公開する |
| `tools/train/zero/fixtures/nn-positions.jsonl` | `crates/minase/tests/fixtures/nn-positions.jsonl` | `nn/control/tests.rs`、`bin/minase/nn_fixture.rs`、`minase_zero/runner.py`、`tools/train/zero/tests/` |
| `tools/train/zero/feasibility/src/bin/teacher_stats.rs` | `src/bin/minase/teacher_stats.rs` | 「`teacher_stats`の移動」節 |

`tools/train/zero/fixtures/zero-selfplay-small.mnsd`は、拡張モジュール`mnsd_reader`とPythonの試験が読み、`minase`の試験は読まないので動かさない。

## ツールごとの判定

`tools/train/zero/feasibility/`の各ツールが現行のコードと合っているかと、今後使う予定があるかを調べ、次のとおり判定した。
削除したツールは、そのツールを使った測定記録に書くコミットから取り出せる。
下の表の面は、ネットの入力を構成する12×12の盤面1枚を指す。

| ツール | 用途 | 判定 | 根拠 |
|---|---|---|---|
| `teacher_stats.rs` | 各サイクルの手数の分位点と終局理由の集計 | 移す | `minase_zero/runner.py`が毎サイクル呼ぶ |
| `trt_bench.rs` | GPUの推論ライブラリTensorRTの速さの測定 | 削除 | 入力を95面に固定しており（`trt_bench.rs`の定数`INPUT_ELEMENTS`）、現行の159面のネットと合わない |
| `sim_cost.rs` | 探索の速さの測定 | 削除 | MCTSの本番の駆動部`Driver`を使わず、独自の探索（`sim_cost.rs`の関数`search`）を測っている |
| `train_bench.py` | 学習の1ステップの時間の測定 | 削除 | 合成データで`model(inputs)`を学習し、現行の学習器が方策と価値のほかに学ぶ補助目標（`forward_logits`）を通らない |
| `make_nets.py`、`tools/train/zero/nn_torch_reference.py` | ランダム初期化のネットの生成と照合 | 削除 | 現行の初期ネットは`minase_zero init`が作り、照合は`minase_zero fixture-eval`が行う。2026年10月31日の学び直しでも、この2つで照合した |
| `batch_probe.rs` | `NnBatchSize`の比較 | 削除 | 最初の進捗測定で値を決め、以後は固定すると[強化学習の実行](../alphazero-rl.md)が定めており、決定は済んでいる |
| `driver_cost.rs` | 探索木の経路の再生の費用の測定 | 削除 | 使った2つのマイルストーンは完了している。再び使うのはCPUの探索が生成の速さを決めている場合だが、[試走](../../measurements/alphazero-rl/alphazero-rl-trial.md)ではGPUが速さを決めていた |
| `mate_cost.rs` | 3手の判定の時間の測定 | 削除 | 判定の方式を決める測定は完了している。より深い判定を調べるときは、履歴から取り出して広げる |

この判定により、[借りる機械の操作の手引き](../../guides/alphazero-machines.md)の6節と7節の手順を改める。
現在の手順は、`measure.sh`、`measure-trt.sh`、および`estimate.py`で処理ごとの速さを測り、式で1サイクルの時間を見積もる。
改めた後は、まだ測っていない機械の構成で`minase_zero run-cycles`の1サイクルを実際に回し（試走）、その時間と料金から見積もる。
現在の手引きも、部品からの見積もりには並列化の効率、推論との受け渡し、および記録の書き込みが含まれないと述べ、確定の判定を試走に委ねている。
[強化学習の実行](../alphazero-rl.md)の、手引きの6節の式を参照する記述も合わせて改める。

## `teacher_stats`の移動

`minase data teacher-stats <ファイル>...`は、現在の`teacher_stats`と同じJSONを標準出力へ出力する。
`run-cycles`の必須引数`--teacher-stats <実行ファイル>`を、必須引数`--minase <実行ファイル>`に置き換える。
実行器は、各サイクルの終わりに`<minase> data teacher-stats <ファイル>...`を実行する。
`cycles.jsonl`の記録のキー`teacher_stats`は変えない。
起動スクリプト`launch-session.sh`は、作業ツリーの`target/release/minase`を渡す。
この実行ファイルは作業機（開発に使う手元の機械）で集計に使うものであり、借りた機械で使う`/content/src/target/release/minase`（`runner.py`の`ENGINE`）とは別である。

古い実行ファイルや古いコミットのまま起動すると、1サイクルの学習を終えた後に初めて失敗し、借りた機械の時間と料金が無駄になる。
そこで実行器は、学習を始める前に次の2つを確かめ、失敗したら直ちにエラーで終了する。

- `<minase> data teacher-stats --help`が成功すること。サブコマンドを持たない古い実行ファイルでは、引数の解析が失敗する。
- 借りた機械に展開したソースに、新しいパスの固定局面集があること。

`tools/train/zero/tests/test_runner.py`の偽の実行ファイルは、全引数を入力ファイルとして検査しているので、サブコマンドと`--help`に対応させ、古い実行ファイルを起動時に拒否する試験を加える。

## 試験専用のコードの分離

規則は次のとおりである。
本番の型のフィールド、本番の関数の中の分岐、および本番の型のメソッドに`#[cfg(test)]`を付けない。
`#[cfg(test)]`を付けるのは、試験用の子モジュールの宣言（`#[cfg(test)] mod test_support;`）だけとする。
試験だけが使う補助関数は、その子モジュール（`X/tests.rs`または`X/test_support.rs`）に置く。
Rustの子モジュールは親モジュールの非公開のフィールドを読めるので、子モジュールに`impl`を書けば、本番の可視性を広げずに済む。
別の実行ファイルの試験から使う補助は、`test-util`フィーチャの`test_util`モジュールに置く。

| 場所 | 現状 | 分離の方法 |
|---|---|---|
| `src/search/mcts/engine.rs`のフィールド`reused`と`prepare_driver`の中の分岐 | 探索木を再利用したかを試験が読むためのフィールド | フィールドと分岐を消す。試験は`prepare_driver`を直接呼び、その直後の根の状態で判定する。`Driver::reuse`は`NodeState::Expanded`の節点だけを再利用し、新しい木の根は`NodeState::Pending`なので、根が`Expanded`なら再利用である。探索を終えた後に`reused`を読んでいる試験は、準備の検査と探索後の挙動の検査に分ける |
| `src/search/mcts/engine.rs`の`with_evaluator` | 試験用のコンストラクタ | `search/mcts/engine/test_support.rs`の`impl Engine`へ移す |
| `src/search/mcts/engine/options_tests.rs` | USIの入出力の試験とエンジンの内部の試験が混在し、`crate::search`から`crate::protocol`を読む唯一のコードになっている（`search/limits.rs`の文書コメントを除く） | USIの入出力を検査する試験は`src/protocol/usi/tests/mcts_options.rs`へ、エンジンの内部を検査する試験は`engine/tests.rs`へ移す。移動先の試験が変更する`cache_dir`と`evaluator`は、`engine/test_support.rs`の`pub(crate)`の関連関数で設定する |
| `src/search/mcts/tree.rs`のフィールド`max_ply`と`depth_limit`の中の分岐 | 試験だけ手数の上限を縮める | `#[cfg]`を外して常に存在するフィールド`depth_limit`とし、`Tree::new`が`MAX_PLY`を入れる。試験は`pub(super)`のこのフィールドを書き換える。本番は定数の代わりにフィールドを読むが、値は常に`MAX_PLY`であり、分岐が消えて試験と本番が同じコードを通る。木の容量の計算は`Tree`自身の大きさを含まないので、容量の上限も変わらない |
| `src/search/budget.rs`の`clock_budget` | 安全余裕30 msで時間配分を求める試験用の短縮形 | 消して、`budget/tests.rs`と`alphabeta/tests/mod.rs`に`ClockCoefficients::current().budget(clock, 30)`を呼ぶ補助関数を置く。`alphabeta/tests/mod.rs`の側は今の`tuning`の条件を保つ |
| `src/search/searcher.rs`の`histories_for_test`と`histories_mut_for_test` | 履歴表を試験が読み書きする | `search/searcher/test_support.rs`の`impl Searcher`へ移す |
| `src/nn/mod.rs`のファイル内モジュール`test_util` | ライブラリの試験だけが使う補助関数`rotate`と`random_positions` | `src/nn/test_support.rs`へ移し、`#[cfg(test)]`のまま使う。フィーチャで公開すると、`test-util`が有効で`cfg(test)`が偽のビルドで未使用の警告になり、`cargo clippy --workspace --all-targets -- -D warnings`が失敗する |
| `src/nn/evaluator.rs`の`UniformEvaluator`（全合法手に同じ値を返す試験用の評価器） | 既定で公開 | `src/nn/test_util.rs`へ移し、`#[cfg(any(test, feature = "test-util"))] pub mod test_util;`で公開する。`crates/minase/Cargo.toml`にフィーチャ`test-util`を足し、自分自身を指す開発依存`minase`で有効にする |

`UniformEvaluator`を既定の公開APIから外すので、このコミットは`refactor(nn)!`とし、本文に`BREAKING CHANGE:`フッターを書く。

## 文書のパスの書き換え

文書の種類ごとに次のとおり扱う。
扱いを分けるのは、機械が出力した値や、固定したコミットで動くスクリプトを書き換えると、記録の正確さが損なわれるからである。

| 対象 | 扱い |
|---|---|
| `docs/measurements/`のMarkdownとスクリプトにある、移したファイルのパス | 新しいパスへ書き換える |
| `docs/measurements/`のMarkdownにある、削除したファイルへのリンク | リンクをやめ、「コミット`<ハッシュ>`の`<パス>`」と書く。ハッシュは、その記録が測定に使ったコミットとする。記録にコミットがなければ、そのファイルを削除する直前のコミットとする |
| `docs/measurements/`のスクリプトのうち、削除したツールを呼ぶもの（`measure.sh`、`measure-trt.sh`、`longrun.sh`、`alphazero-network/random-net-output-check/run.sh`） | 移したファイルのパスも含めて本文を書き換えず、測定に使ったコミットのソースで動くことを先頭のコメントに書く。削除したツールは新しいパスを持たず、一部のパスだけを直すとどのコミットでも動かなくなるからである |
| 機械が出力した生データ（`.json`、`.out`、`.log`、`.patch`） | 書き換えない |
| 固定したコミットのworktreeを指すスクリプト（`run-c06.sh`、`run-c11.sh`、`run-c21.sh`、`launch-trial.sh`） | 書き換えない |
| `docs/guides/`、進行中の設計書、`docs/ROADMAP.md`、およびソースのdocコメント | 新しいパスと手順へ書き換える |
| `docs/adr/`（`archive/`を除く）のパス | 決定の内容は変えず、[ADRの規則](../../adr/README.md)のリンク切れの修正として、移したファイルのパスだけを直す。削除したツールを述べる文（`train-the-zero-network-in-pytorch-and-export-it-as-onnx.md`の`make_nets.py`など）は当時の事実の記述なので残す |
| `docs/plans/archived/`、`docs/adr/archive/`、`docs/audits/`、`docs/research/` | 凍結された記録なので書き換えない |

2026年11月2日の時点で、`docs/measurements/`の中のファイルを`git grep -l`で数えると、`zero/feasibility`を含むものは19件、`nn-positions`を含むものは8件、`alphabeta/params`を含むものは1件、`zero_selfplay/mate`を含むものは2件である。

[王手限定の3手以内の勝ちのADR](../../adr/archive/stop-zero-self-play-at-check-only-wins-within-three-plies.md)の「判定のモジュールは自己対局生成器の中に置き」は決定の変更に当たるので、完了時に新しいADRを書き、古いADRを置き換え済みとして`docs/adr/archive/`へ移す。

## 着手時に補った決定

- 実行器の起動時の確認のうち、固定局面集の確認はフェーズ2では移動前のパスで入れ、フェーズ7で定数`FIXTURE`の1行だけを新しいパスへ改めた。新しいパスのまま入れると、フェーズ7まで実行器が起動時の確認で止まるからである。
- `launch-session.sh`は、機械を借りる前に`minase data teacher-stats --help`を実行する。実行器の確認は環境の導入の後に走るので、実行器の確認だけに任せると導入にかかった時間の料金が無駄になるからである。
- `params.rs`のrustdocのリンクは、リンクにせず`alphabeta::searcher::Searcher::quiesce`の文字列とした。リンクにすると、文書のためだけにαβ探索の`searcher`モジュールの可視性を広げることになるからである。
- `docs/measurements/`のMarkdownのコマンド例にある削除したツールのパスは、表の「コミット`<ハッシュ>`の`<パス>`」の形に準じて扱い、そのコミットをコマンド例の前の文に書いた。コマンドの中には修飾語を入れられないからである。コマンド例にある移したファイルのパスは、表のとおり新しいパスへ書き換えた。
- フェーズ5の検証で、`crates/minase/tests/match_runner.rs`の`usi_options_are_forwarded_recorded_and_restored_for_each_engine`がmasterでも約30回に1回失敗する不安定なテストであることが分かった。[テストの手引き](../../guides/testing.md)の規則に従い、別のコミットで直した。先手が直ちに投了すると、後手が`usinewgame`を読む前にハーネスに止められることが原因なので、この行を比較から外した。`isready`と`usinewgame`の順序は`harness/engine/tests.rs`の試験で検証している。

## 実装フェーズ

各フェーズを1コミットとし、移動だけのコミットと挙動に関わるコミットを分ける。
実装は[codexへの実装委任の手引き](../../guides/codex-delegation.md)に従って委任する。
同じ手引きのとおりcodexはworktreeでコミットできないので、1回の委任を1フェーズとし、委任元が検証してコミットする。

後のフェーズで参照が壊れないように、測定用のcrateを先に消す。
このcrateは`#[path]`で`zero_selfplay/mate.rs`を取り込み、既定の依存で`UniformEvaluator`を使っているので、残したままRustの移動を進めるとビルドできなくなる。

1. 基準の記録。masterから切ったブランチのworktreeで、「検証」節の2と3の基準を記録し、現在の`teacher_stats`の実行ファイルを保存する。完了条件は、自己対局の出力が同じコミットの2回の実行で一致すること。一致しなければ、推論の決定性を調べ、検証の方法を決め直すまで先へ進まない。
2. `teacher_stats`の移動。サブコマンドの追加、`run-cycles`の引数と起動時の確認の変更、`launch-session.sh`の変更、および試験の変更を行う。完了条件は、同じMNSDのファイルについて出力が保存した`teacher_stats`とバイト単位で一致し、学習器のテストが通ること。
3. 測定用のcrateの削除。`tools/train/zero/feasibility/`と`nn_torch_reference.py`を削除し、手引きを改める。本体の作業ツリーの無視されたファイル`tools/train/zero/feasibility/target/`はこのコミットの対象ではなく、学習が動いている間は消さない。
4. Rustの移動。「移動の一覧」節のうち、固定局面集と`teacher_stats`以外を移す。完了条件は、各行の出現回数を比べた差分（行の多重集合の差分）が、可視性の修飾子と呼び出しパスだけであること。
5. `softmax`の統合。完了条件は、mctxとの照合試験を含むテストが通り、自己対局の出力が基準と一致すること。
6. 試験専用のコードの分離。`options_tests.rs`の分割はここで行う。完了条件は、「検証」節の7を満たし、`cargo build --release --bin minase`が`test-util`なしで通ること。
7. 固定局面集の移動。完了条件は、Rustの試験とPythonの試験が新しいパスで通ること。
8. 文書のパスの書き換え。[詰み探索の設計書](../mate-search.md)の置き場所もここで改める。完了条件は、「検証」節の6と8を満たすこと。

## 検証

マージの前に、次のすべてを確かめる。

1. [テストの手引き](../../guides/testing.md)の4つの検査（`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`cargo test -p minase --lib --features search-stats,tuning`）が通ること。
2. αβ探索の`minase dev bench`（固定の15局面を深さ6まで探索する検査）の探索ノード数が、整理の前と一致すること。
3. 自己対局の出力が、整理の前と後で一致すること。次のコマンドを、毎回新しい出力先で実行し、MNSDのファイルをバイト単位で比べる。完了済みの塊は再利用されるので、同じ出力先では比べられない。ネットは[学び直しの最初のランダム初期化のネット](../../measurements/alphazero-restart-techniques/alphazero-restart-techniques-random-net-output-check.md)を使う。対局が短いので所要時間は短いと見込むが、フェーズ1で実測する。学習に使わないので、シードの台帳には登録しない。このコマンドで、`softmax`、3手の判定の移動、および`depth_limit`の変更を確かめる。

   ```console
   target/release/minase data zero-selfplay --output-dir <新しい出力先> \
     --nn-model /mnt/synology_nas/minase/alphazero/nets/cycle-0000.onnx --nn-backend cpu \
     --generation-commit 0000000000000000000000000000000000000000 --seed 20261102 \
     --games 8 --chunk-games 8 --target-positions 1000000000 --simulations 32 \
     --playthrough-percent 100 --threads 1 --games-per-thread 8
   ```

4. `tools/train/zero`の学習器のテストと、`scripts/vast-spsa/test_spsa.py`が通ること。
5. 移動だけのコミットについて、旧ファイル群と新ファイル群の行の多重集合の差分（`use`、`mod`の宣言、空行を除く）が、可視性の修飾子と呼び出しパスだけであること。
6. 古いパスが、「文書のパスの書き換え」節が書き換えないとした場所以外に残らないこと。次の語を`git grep`する：`alphabeta/params`、`alphabeta::params`、`zero_selfplay/mate`、`zero_selfplay::mate`、`mate_tests`、`budget_tests`、`options_tests`、`zero/feasibility`、`feasibility/`、`nn_torch_reference`、`make_nets`、`zero/fixtures/nn-positions`、`"fixtures" / "nn-positions`、`SOURCE + "/fixtures/nn-positions`、`fixtures/nn-positions.jsonl`（`crates/minase/tests/fixtures/`を除く）、`--teacher-stats`。
7. `crates/minase/src/nn/`、`crates/minase/src/search/mcts/`、`crates/minase/src/search/mate_in_three.rs`、`crates/minase/src/search/budget.rs`、および`crates/minase/src/search/searcher.rs`の本番のファイルで、`#[cfg(test)]`の次の行がモジュールの宣言か`#[path]`以外であるものが0件であり、`#[cfg(not(test))]`が0件であること。
8. 書き換えた文書の相対リンクの行き先がすべて存在すること。

## 完了条件

「検証」節のすべてを満たし、「依存関係」節の条件を確かめてmasterへマージしたうえで、次の作業を終えたときに完了（採用）とする。

- 試験専用のコードの規則を[テストの手引き](../../guides/testing.md)へ加える。
- ツールの削除と試験専用のコードの規則の判断を新しいADRへ書き、3手の判定のADRを置き換える。
- 本書の完了後も有効な配置の説明を`crates/minase/src/lib.rs`と各モジュールのdocコメントへ移す。

## 参考資料

- [AlphaZero型探索と深層強化学習の統括計画](../alphazero.md)
- [強化学習の実行](../alphazero-rl.md)
- [詰み探索（df-pn）の設計書](../mate-search.md)
- [借りる機械の操作の手引き](../../guides/alphazero-machines.md)
- [ネットの出力の照合の手引き](../../guides/alphazero-network-check.md)
- [テストの手引き](../../guides/testing.md)
- [codexへの実装委任の手引き](../../guides/codex-delegation.md)
- [学習ツールの再配置](train-tools-layout.md)：測定スクリプトのパスを書き換えた先例

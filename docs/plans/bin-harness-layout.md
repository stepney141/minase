# 実行ファイルと対局ハーネスのモジュール再編の設計書

## 先に読む要約

対局ハーネス（`match_runner`と`spsa_runner`が共用する、外部エンジンの起動、通信、および先後入替ペアの対局実行の層）の`src/harness.rs`は2,493行ある。このファイルには、エンジン指定の解釈、コミットのビルド、USIとCECPのプロセス駆動、時計、対局の進行、保存形式への変換、開始局面の生成、および先読みの集計が同居している。
実行ファイルの側も、`src/bin/match_runner.rs`（1,814行）、`src/bin/selfplay_gen.rs`（2,367行）、`src/bin/match_report.rs`（993行）が、それぞれ引数の解釈から集計の出力までを1ファイルに持つ。
さらに`lishogi_import`は、`#[path = "selfplay_gen.rs"]`によって`selfplay_gen`の実行ファイル全体を自分の中へ取り込み、その一部の関数だけを使っている。
本書は、コードの目的ごとにファイルとディレクトリを分け、1ファイルが1つの関心事だけを持つ配置へ移す。
ハーネスは`src/harness/`の下に目的別の子モジュールを置き、エンジンのプロセス駆動を`src/harness/engine/`へまとめる。
複数のファイルを持つ実行ファイルは`src/bin/<名前>/main.rs`の形に改め、`#[path]`属性をなくす。
`selfplay_gen`と`lishogi_import`が共有するコードは、ハーネスと同じくライブラリへ移す。
再編は処理の中身を変えない移動であり、既存の試験が全件通り、固定シードの対局ハーネスと学習局面の生成の出力が再編の前後で一致することを完了条件とする。

## 状態

進行中。
2026年9月27日に起案および着手し、共有コードの置き場所と分割する実行ファイルの範囲を同日に利用者が決定した。
実装フェーズの1から7までを`bin-harness-layout`ブランチで完了し、各コードのコミットで「検証」の節の確認（fmt、clippyの警告0件、既定と`tuning`機能の全試験、および固定シードの出力の一致）がすべて通った。
試験の件数は788件から772件になったが、差の16件は`lishogi_import`の試験バイナリで重複実行されていた`selfplay_gen`の試験であり、試験名の末尾の集合は再編前と一致する。
次の一手はmasterへの統合である。

## 目的

対局ハーネス、自己対局の生成、SPSA、および測定の集計に手を入れるときに、変更すべきファイルを配置から一意に決められる状態にする。
あわせて、実行ファイルが別の実行ファイルのソースを`#[path]`で取り込む構造をなくし、複数の実行ファイルが共有するコードの置き場所を「ライブラリ」という1つの規則にそろえる。

## 適用範囲

対象は、`src/harness.rs`と`src/harness/`、`src/bin/`のうち次の6つの実行ファイル、新設するライブラリのモジュール、これらを参照する`src/lib.rs`と`src/rng.rs`、および`docs/`とリポジトリ直下の文書にあるパスの参照である。

| 実行ファイル | 現在の行数 | 再編 |
|---|---|---|
| `match_runner` | 本体1,814行、子2ファイル944行 | ディレクトリへ分割する。 |
| `match_report` | 993行 | ディレクトリへ分割する。 |
| `selfplay_gen` | 2,367行 | 共有部分をライブラリへ移し、残りをディレクトリへ分割する。 |
| `lishogi_import` | 778行 | ディレクトリへ分割し、`selfplay_gen`の取り込みをやめる。 |
| `spsa_runner` | 本体353行、子7ファイル3,484行 | `main.rs`の形へ改め、本体に残る処理を子へ移す。 |
| `random_play` | 665行 | ディレクトリへ分割する。 |

`bench`（398行）、`usi_random`（297行）、`minase`（175行）、`pst_probe`（168行）、および`perft`（165行）は、1ファイルのまま残す。
いずれも400行以下で、引数の解釈と1つの処理だけを持ち、分けても1ファイルあたり数十行の断片が増えるだけだからである。

関数、型、定数、および試験の名前は変えない。
処理の内容、引数、処理の順序、エラーの文言、および出力の書式も変えない。
ただし、移動に伴って次の4種類の記述は書き換える。
1つ目は、参照元が定義元の子孫でなくなる項目の可視性であり、「可視性の規則」の節に従って必要な最小の範囲へ広げる。
2つ目は、`use`文と、式の中に直接書かれたモジュールパスである。
3つ目は、`include_str!`が埋め込むファイルの相対パスであり、ファイルの階層が変わった分だけ`../`を増減する。対象は`selfplay_gen.rs`の`../../tests/fixtures/lishogi_import_cases.ndjson`、`spsa_runner/tests.rs`の`../../search/alphabeta/params.rs`（2か所）、および`spsa_runner/tests/simulation.rs`の`../../../search/alphabeta/params.rs`である。
4つ目は、ライブラリへ移す項目の可視性であり、`pub(crate)`を`pub`へ改め、欠けているdocコメントを補う（`Cargo.toml`の`missing_docs = "warn"`）。

次の名前と文字列は、外部から参照されるので変えない。
実行ファイル名`usi_random`、`minase`、`selfplay_gen`、`pst_probe`、`lishogi_import`、`match_runner`、`match_report`、および`spsa_runner`は、`resolve_player`の`current_exe`からの探索、`resolve_commit`の`cargo build --bin minase`、試験の`CARGO_BIN_EXE_*`、`tools/train/pst/`のPythonスクリプト、および`docs/guides/`の標準コマンドが使う。
`spsa_runner`の試験モジュールのパス`tests::simulation`は、凍結された測定記録`docs/measurements/spsa-gain-simulation.md`が`--exact tests::simulation::gain_simulation`として記録しているので保つ。
`match_runner/storage.rs`の`manifest does not match`という文言は、`tests/match_runner.rs`が標準エラー出力を照合する。
実行ディレクトリのファイル名（`.match_runner.lock`、`manifest.json`、`pairs/`、`summary.json`）と保存形式の版数も変えない。

対象外は次のとおりである。
実行ファイルの間やハーネスとの間には重複した処理が多く見つかったが、統合は本書では行わない。
統合の多くは出力、保存形式の厳密さ、またはエラーの文言を変え、「検証」の節の一致の確認を成り立たなくするからである。
見つかった重複は「後続の候補」の節に記録する。
`docs/measurements/`と`docs/audits/`は凍結する文書（docs/README.md）なので、パスの参照を書き換えない。

## 依存関係

[src構成の整理](src-layout.md)が定めた依存の向き（search → eval → core、trainingはcoreだけに依存）を保つ。
ハーネスは現在`notation`、`rng`、`search`（`MAX_PLY`だけ）、およびcrateの直下の再公開に依存し、再編後も同じである。
新設する学習局面の生成の支援モジュール`datagen`は、`training`と`core`に依存する。
教師の探索を行う処理は実行ファイルに残るので、`datagen`は`search`と`eval`に依存しない。
それでも`training`の下に置かないのは、`training`がMNSD、MNRS、および来歴JSONという交換形式だけを持つモジュールであり、gitの呼び出し、clapの引数の型、および生成の集計という生成の手続きの支援を混ぜないためである。

masterへ未統合のブランチのうち、本書の対象ファイルを変更しているものは次のとおりである。
`search-aware-evaluation`は、`bench.rs`と`pst_probe.rs`を変更し、実行ファイル`eval_teacher`、`search_aware_data`、および`search_aware_diagnostic`と、これらが`#[path]`で共有する`src/bin/teacher_common/`を追加している。
このブランチのコードをmasterへ統合するかは利用者の判断を待っている。統合する場合は、masterを取り込んだうえで3つの実行ファイルを`main.rs`の形へ改め、`teacher_common`を本書と同じ規則でライブラリへ移すかどうかを統合時に判断する。
`strength-stage9-teacher`は`lishogi_import.rs`と`selfplay_gen.rs`を、`nnue-gen1`は`bench.rs`、`match_runner.rs`、`minase.rs`、および`selfplay_gen.rs`を、`fm-eval`、`strength-stage9-diag`、および`strength-stage9-diag-b`は`pst_probe.rs`と`bench.rs`を変更している。
これらはいずれも採否が確定して作業を終えたブランチなので、再開する場合に本書の配置へ付け替える。

## 設計判断

| 論点 | 採用 | 棄却した代案 |
|---|---|---|
| ハーネスの形 | `src/harness.rs`を`src/harness/mod.rs`へ移し、子モジュールを目的別に並べる | `src/harness.rs`を残して子を増やす |
| エンジンのプロセス駆動 | `src/harness/engine/`にまとめ、`EngineProcess`の`impl`を目的別のファイルに分ける | USIとCECPを別の構造体に分ける |
| ハーネスの公開パス | `minase::harness::X`の平らな公開面を`mod.rs`の`pub use`で保つ | 呼び出し側の`use`を`minase::harness::engine::X`などへ書き換える |
| 試験の置き場所 | 1つの関心事だけを検査する試験は、検査対象のファイルの`mod tests`へ移す。先読みの試験のように複数の関心事にまたがる試験は、その関心事をまとめるディレクトリの`tests`へ置く | 現行の`tests.rs`を主題別のファイルに分けて`src/harness/tests/`へ置く |
| 複数ファイルの実行ファイル | `src/bin/<名前>/main.rs`の形に改め、`#[path]`をなくす | `src/bin/<名前>.rs`を残し、`#[path]`で子を取り込み続ける |
| `selfplay_gen`と`lishogi_import`の共有コード | ライブラリにトップレベルのモジュール`src/datagen/`を新設し、`#[doc(hidden)] pub mod datagen;`とする（2026年9月27日の利用者決定） | `src/bin/datagen_common/mod.rs`に置き、両方の実行ファイルから`#[path]`で取り込む |
| 分割する実行ファイル | 「適用範囲」の表の6つ（2026年9月27日の利用者決定） | `#[path]`をなくすのに必要な3つと`match_report`だけを分ける |
| 重複の統合 | 行わず、「後続の候補」に記録する | 移動と同時に統合する |

ハーネスの平らな公開面を保つのは、ハーネスの外から項目を参照する9ファイルがすべて`minase::harness::X`の形か`minase::harness::*`の一括取り込みで参照しており、この形がハーネスの公開面そのものだからである。
[探索部と評価関数のモジュール再編](search-eval-layout.md)も、境界の`src/search/mod.rs`が公開していた`minase::search::TranspositionTable`などを同じパスで公開し続けた。
これは旧パスの互換のための再公開ではなく、現行の`src/harness.rs`が`pub use environment::*;`などで採っている形をそのまま続けるものである。

試験を検査対象のファイルへ置くのは、ハーネスの試験の多くが非公開の欄や関数（`Clock`の残り時間、`EngineProcess`の応答期限、`normalize_commit`など）を直接検査しており、試験を定義元の子孫に置けば可視性を広げずに済むからである。
前例の探索部の再編は試験を`alphabeta/tests/`へ集めたが、あちらは1つの試験が制限の構築から置換表の確認までを通して検査しており、1つのファイルに帰属させられなかった。
ハーネスの試験の大半は1つの関心事だけを検査する。
例外は、`EngineProcess`と対局の進行`play_game`の両方を使う先読みの試験と、`SearchLimit`と`ThinkRequest`の欄も読む時計の試験`cecp_time_requests_include_byoyomi`である。
前者は`src/harness/engine/tests.rs`に置き、後者は`clock.rs`に置いて、読む欄を「可視性の規則」の節の範囲で広げる。

共有コードをライブラリへ置くのは、`match_runner`と`spsa_runner`が共有するコードが既にライブラリの`harness`にあり、同じ規則を学習局面の生成にも当てはめられるからである。
この配置では`#[path]`がなくなり、共有部分の試験が1つの試験バイナリで1回だけ実行される。
clapはライブラリの通常の依存に既に含まれ、`harness`も実行ファイルのための内部層として`#[doc(hidden)]`で公開している。
モジュール名の`datagen`は、Stockfish系のエンジンが学習局面の生成器に広く使う名前である。
代案の`src/bin/datagen_common/`は、`search-aware-evaluation`ブランチの`teacher_common`と同じ形でライブラリの公開面を増やさないが、`#[path]`が残り、片方だけが使う項目に`#[allow(dead_code)]`が要る。

`lishogi_import`には`#[global_allocator]`でmimallocを明示する。
`lishogi_import`は現在、取り込んだ`selfplay_gen.rs`の`#[global_allocator]`によって偶然mimallocを使っており、取り込みをやめると黙って標準のアロケータへ戻るからである。

6つの実行ファイルを分割し、400行以下の5つを1ファイルのまま残すのは、`random_play`（665行）が引数、シード、表記、対局の不変条件の検査、および集計の5つの関心事を、`lishogi_import`（778行）が入力の形式、除外の判定、再生、ラベル付け、および出力の5つの関心事を持つからである。

`src/bin/<名前>/main.rs`の形に改めるのは、Cargoがこの形の`main.rs`を実行ファイルの根とみなし、`mod storage;`が同じディレクトリの`storage.rs`を指すようになるからである。
Cargoは`src/bin/<名前>.rs`と`src/bin/<名前>/main.rs`が同時にあると「found duplicate binary name」として拒否する（Cargo 1.x、空のパッケージで確認済み）ので、同じコミットで前者を削除する。
`src/bin/`の直下に置いた`.rs`ファイルはすべて実行ファイルとして自動で検出されるので、子モジュールは必ず各実行ファイルのディレクトリの中に置く。

## 再編後の配置

表に挙げた型の`impl`（`Display`、`Error`、`Default`、`Drop`などのトレイト実装を含む）は、表で別のファイルを指定したメソッドを除き、型と同じファイルへ移す。
表にない補助関数は、それを使うファイルが1つならそのファイルへ置き、複数ならそのディレクトリの`mod.rs`（実行ファイルでは`main.rs`）へ置く。

### 対局ハーネス

| パス | 置くもの |
|---|---|
| `src/harness/mod.rs` | モジュールの宣言と、公開する名前の`pub use`。 |
| `src/harness/player.rs` | エンジンの指定と構成。`PlayerSpec`、`PlayerKind`、`Protocol`、`PlayerConfig`、`parse_player_spec`、`resolve_player`。 |
| `src/harness/commit.rs` | コミットのビルドとキャッシュ。`command_error`、`normalize_commit`、`resolve_commit`、`sha256_file`。 |
| `src/harness/limit.rs` | 思考制限とその構文。`SearchLimit`、`TimeControl`、`parse_search_limit`、`parse_nonnegative_u64`、`parse_positive_u64`、`parse_positive_u32`、`parse_search_depth`。 |
| `src/harness/clock.rs` | 対局時計。`Clock`、`GameClocks`、`zero_clock`。 |
| `src/harness/failure.rs` | エンジン異常の分類と件数。`EngineFailure`、`FailureCounts`。 |
| `src/harness/engine/mod.rs` | 子モジュールの宣言と、1回の思考の要求と応答の型。`ThinkRequest`、`EngineResponse`、`EngineEvaluation`、`EngineScore`、`ThinkResult`、`EngineDefaults`。 |
| `src/harness/engine/process.rs` | 子プロセスの起動、送受信、および後始末。`EngineProcess`と、その`spawn`、`start`、`send`、`wait_for`、`receive_until`、`Drop`。 |
| `src/harness/engine/think.rs` | 1手の思考。`EngineProcess`の`bestmove`、`send_position`、`receive_bestmove`。 |
| `src/harness/engine/ponder.rs` | 先読み。`EngineProcess`の`start_ponder`、`discard_bestmove`、`stop_ponder`。 |
| `src/harness/engine/usi.rs` | USIの`info`行と`option`行の解釈。`parse_usi_evaluation`、`observe_usi_evaluation`、`parse_usi_stop_reason`、`parse_usi_time`、`observe_usi_search_data`、`parse_usi_spin_default`、`observe_usi_default`、`EngineProcess`の`read_usi_defaults`。 |
| `src/harness/engine/cecp.rs` | CECPの送信文と応答の解釈。`CECP_MEMORY_MB`、`CECP_FIXED_TIME_CS`、`cecp_level_text`、`cecp_fixed_time_text`、`cecp_memory_text`、`validate_cecp_limit`、`is_cecp_response_line`、`interpret_cecp_response`、`receive_cecp_response`、`canonical_jitto`、`EngineProcess`の`bestmove_cecp`。 |
| `src/harness/engine/channel.rs` | 期限付きの行の受信。`receive_until`、`receive_until_deadline`（`EngineProcess`のメソッドではない自由関数）。 |
| `src/harness/engine/resources.rs` | プロセスのCPU時間と最大常駐メモリ。`EngineResourceUsage`、`process_resource_usage`（Linux版とそれ以外）、`EngineProcess`の`resource_usage`。 |
| `src/harness/engine/probe.rs` | 対局前のエンジンの既定値とオプションの確認。`probe_engine_defaults`、`probe_usi_options`。 |
| `src/harness/engine/tests.rs` | 現行の`src/harness/ponder_tests.rs`の全体と、`EngineProcess::start`のオプション送信順を検査する`usi_options_are_sent_in_order_before_isready`。 |
| `src/harness/referee.rs` | エンジンの着手の審判層による検査。`validate_bestmove`、`ponder_move`。 |
| `src/harness/opening.rs` | ペアの開始局面。`Opening`、`generate_opening`。 |
| `src/harness/game.rs` | 1局の進行。`GameOutcome`、`PlayedGame`、`play_game`。 |
| `src/harness/pair.rs` | 先後入替ペアの実行と得点。`PairResult`、`CompletedPair`、`run_pair`、`half_points`、`record_game_failure`、`played_game_text`、`square_text`、`move_text`。 |
| `src/harness/ponder_stats.rs` | 保存記録の再生による先読みの集計。`PonderCounts`、`count_ponder_game`。 |
| `src/harness/records/mod.rs` | 現行の`src/harness/records.rs`の全体（保存形式の型と試験）。 |
| `src/harness/records/convert.rs` | 対局結果から保存形式への変換。`RecordedGame`、`stored_color`、`stored_failure`、`failure_from_stored`、`evaluation_record`、`duration_ns`、`termination_record`、`recorded_game`。 |
| `src/harness/storage.rs` | 変更しない。 |
| `src/harness/environment.rs` | 変更しない（試験の追加を除く）。 |

`HarnessRecord`と`CpuRecord`は、保存形式の型だが`environment.rs`に残す。
2つの型は環境の調査結果を表し、`HarnessRecord`は`harness_record`が作り、`CpuRecord`は各runnerが`cpu_model`、`physical_core_count`、および`physical_memory_bytes`の調査結果から組み立てるからである。

現行の`src/harness/tests.rs`の試験は、次の規則で移す。

| 試験 | 移動先 |
|---|---|
| `engine_specs_*`、`cecp_resolution_*`、`hash_resolution_*` | `player.rs` |
| `unknown_commit_revision_fails_resolution` | `commit.rs` |
| `search_limits_*` | `limit.rs` |
| `time_forfeit_*`、`go_time_arguments_*`、`cecp_time_requests_include_byoyomi` | `clock.rs` |
| `cecp_level_*`、`cecp_fixed_time_*`、`cecp_memory_*`、`cecp_null_move_*`、`cecp_response_lines_*`、`cecp_pong_*` | `engine/cecp.rs` |
| `usi_resource_probe_*`、`usi_evaluation_*`、`usi_search_data_*`、`invalid_usi_search_data_*` | `engine/usi.rs` |
| `response_channel_*` | `engine/channel.rs` |
| `linux_resource_probe_*` | `engine/resources.rs` |
| `usi_options_are_sent_in_order_before_isready` | `engine/tests.rs` |
| `referee_rejects_*` | `referee.rs` |
| `openings_stay_*` | `opening.rs` |
| `failure_reasons_*` | `failure.rs` |
| `game_outcomes_map_*` | `pair.rs` |
| `pair_seed_replaces_the_zero_output` | `src/rng.rs`（`derive_seed`だけを検査しているため） |

`match_runner.rs`の試験のうち、ライブラリの`physical_core_count`と`physical_memory_bytes`だけを検査する`linux_host_resource_probe_reports_cores_and_memory`は、`src/harness/environment.rs`の`mod tests`へ移す。

### 学習局面の生成（ライブラリ）

| パス | 置くもの |
|---|---|
| `src/datagen/mod.rs` | モジュールの宣言、`MATE_BAND_START`、`current_position_is_repeated`、`data_error`、`invalid_data`、`training_error`。 |
| `src/datagen/game.rs` | 生成した局面と対局のまとまり、および書き出し順の整列。`CompletedRecord`、`CompletedGame`、`merge_completed_games`。 |
| `src/datagen/statistics.rs` | 生成の集計。`INJECTION_HISTOGRAM_BINS`、`SCORE_VALUE_COUNT`、`Statistics`、`RecordedStatistics`、`score_index`。 |
| `src/datagen/provenance.rs` | 来歴JSONの書き出し。`ProvenanceArguments`、`write_mapped_provenance`、`hex`。 |
| `src/datagen/git.rs` | 生成コミットの取得と検査。`git_output`、`validate_commit_hash`。 |

`src/lib.rs`に`#[doc(hidden)] pub mod datagen;`を加え、crateのdocコメントのモジュール一覧に加える。
`lishogi_import`が現在`selfplay_gen`から使う項目は、`MATE_BAND_START`、`ProvenanceArguments`、`CompletedRecord`、`CompletedGame`、`Statistics`、`merge_completed_games`、`current_position_is_repeated`、`data_error`、`write_mapped_provenance`、`git_output`、および`validate_commit_hash`の11項目である。
これらは`selfplay_gen.rs`の中で次の項目に依存しているので、依存先もあわせて`datagen`へ移す。
`Statistics`は`INJECTION_HISTOGRAM_BINS`を、`RecordedStatistics`は`SCORE_VALUE_COUNT`と`score_index`を、`merge_completed_games`は`training_error`と`RecordedStatistics`を、`git_output`と`validate_commit_hash`は`invalid_data`を、`write_mapped_provenance`は`hex`を使う。
`RecordedStatistics`は`lishogi_import`から直接は使われていないが、`merge_completed_games`の戻り値の型なので移す。
移した項目のうち、`selfplay_gen`の残りの処理も使うもの（2つの定数、`score_index`、`invalid_data`、`training_error`、`hex`）は、`selfplay_gen`が`datagen`から参照する。
`RescoreCommandError`は、`write_provenance`と付け直しだけが使うので`selfplay_gen`に残す。

`datagen`の中で、`selfplay_gen`または`lishogi_import`から使う項目、欄、およびメソッドは`pub`とする。
具体的には、上の表の全項目、`ProvenanceArguments`、`CompletedRecord`、`CompletedGame`、`Statistics`、および`RecordedStatistics`の欄（`selfplay_gen`の生成の要約が`RecordedStatistics`の`score_frequencies`と`duplicate_positions`を読む）、ならびに`selfplay_gen`の`play_game_from_opening`が呼ぶ`Statistics::record_result`である。
`datagen`の中だけで呼ぶ`Statistics::merge`と`RecordedStatistics::record`は`pub(super)`とする。

`merge_completed_games`の試験2件（`completed_games_are_merged_*`、`completed_game_merge_rejects_*`）は、`datagen`へ移さず`selfplay_gen`の`generate.rs`に置く。
試験の補助`completed_game`が`selfplay_gen`の`engine_default_rules`を使っており、`datagen`へ移すには補助の依存も移す必要があるが、この2件は`pub`の`merge_completed_games`だけを検査するので、呼び出し側の`generate.rs`から検査できるからである。

### 実行ファイル

各実行ファイルのディレクトリの`main.rs`には、`#[global_allocator]`、モジュールの宣言、`main`、および複数の子モジュールが使う定数と補助関数を置く。
子モジュールの項目の可視性は`pub(super)`とする（既存の`match_runner/storage.rs`と同じ）。
各子モジュールの試験は、そのファイルの`mod tests`へ置く。

`match_runner`は次のとおりとする。

| パス | 置くもの |
|---|---|
| `main.rs` | `main`。 |
| `cli.rs` | 引数と既定値。`DEFAULT_MAX_PLY`、`DEFAULT_MAX_PAIRS`、`DEFAULT_RESPONSE_TIMEOUT_SECONDS`、`Arguments`と`validate_ponder`、`Mode`、`RuleSetArgument`、`parse_rule_set_argument`、`parse_positive_usize`、`time_seed`。 |
| `scheduler.rs` | ペアの配布と番号順の取り込み。`accept_completed_pair`、`continue_after_decision`、`run_worker_loop`。 |
| `manifest.rs` | 実行条件の構築。`stored_search_limit`、`default_concurrency`、`run_manifest`、`rules_text`。 |
| `replay.rs` | 再開時の保存記録の再生と検査。`ponder_summary`、`color_from_stored`、`adjudication_matches`、`SavedGameConditions`、`validate_saved_game`、`invalid_pair_record`、`completed_pair_from_record`。 |
| `summary.rs` | 逐次の統計と最終サマリの出力。`decision_text`、`elo_text`、`print_failure_summary`、`print_gsprt_summary`、`print_elo_summary`、`integrate_pair_statistics`。 |
| `storage.rs` | 現行の`match_runner/storage.rs`をそのまま移す。 |

現行の`ponder_tests.rs`の3件は、`ponder_arguments_*`を`cli.rs`へ、`ponder_concurrency_*`を`manifest.rs`へ、`saved_predictions_*`を`replay.rs`へ移す。
現行の`mod tests`の試験も、検査している関数のファイルへ移す。
試験の補助`completed_pair`と`SetAtomicOnDrop`は、使う試験が1つのファイルに収まればそのファイルの`mod tests`へ、複数にまたがれば`main.rs`の`#[cfg(test)]`の補助モジュールへ置く。

`match_report`は次のとおりとする。

| パス | 置くもの |
|---|---|
| `main.rs` | `Arguments`、`main`。 |
| `run_dir.rs` | 実行ディレクトリの読み取り用の型と読み出し。`FORMAT_VERSION`、`Manifest`、`EngineRecord`、`Mode`、`SearchLimit`、`ThreadCounts`、`CpuRecord`、`RunSummary`、`read_json`。 |
| `report.rs` | 1つの実行ディレクトリの集計。`Report`、`FailureCounts`、`MissingResourceCounts`、`MissingEngineThreads`、`report`、`nearest_rank`、`median`、`record_failure`、`elo_text`、`candidate_half_points`。 |
| `compare.rs` | 2つの時間制御の比較。`Comparison`、`compare`。 |

`match_report`の試験は、`percentile_and_median_*`と`report_*`を`report.rs`へ、`comparison_accepts_*`を`compare.rs`へ移す。
`comparison_accepts_*`は`Report`と`MissingEngineThreads`の欄を直接組み立てるので、`compare`が読むために`pub(super)`とする欄に加えて、試験が組み立てる欄も`pub(super)`とする。

`run_dir.rs`の型は`match_runner/storage.rs`の型の読み取り専用の写しだが、統合しない。
`match_runner`の型は未知の欄を拒否し、`match_report`の写しは受け入れるので、統合すると`match_report`が読める実行ディレクトリの範囲が変わるからである。

`selfplay_gen`は次のとおりとする。

| パス | 置くもの |
|---|---|
| `main.rs` | `GLOBAL`、`datagen`へ移すものを除く定数、`main`、`run`、`engine_default_rules`、および試験の共通の補助を置く`#[cfg(test)] mod test_support`（`RescoreFixture`など）。 |
| `cli.rs` | `Arguments`、`Operation`、`RescoreArguments`、`GenerateArguments`、`InspectArguments`、`parse_positive_u32`、`parse_positive_u16`、`parse_positive_usize`。 |
| `opening.rs` | 開始局面。人間の棋譜から作る開始局面の一覧とランダムな開始局面の生成。`Opening`、`OpeningError`、`read_openings`、`starting_game`、`generate_opening`。 |
| `play.rs` | 1局の生成。`PlaySettings`、`Candidate`、`InjectionPlan`、`play_game`、`play_game_from_opening`、`plan_injections`、`plan_injections_with_rng`。 |
| `generate.rs` | 生成コマンド。`generate`、`write_training_data`、`print_generation_summary`。 |
| `statistics.rs` | 生成の集計の表示用の計算。`score_mean_and_std`、`score_percentile`、`format_rate_percent`。 |
| `inspect.rs` | 検査コマンド。`InspectionSummary`、`inspect`、`print_record`。 |
| `provenance.rs` | 来歴コマンド。`write_provenance`。 |
| `rescore.rs` | 付け直しコマンド。`RescoreCommandError`、`RescoreSummary`、`open_rescore_output`、`rescore_record`、`write_rescore_records`、`rescore`。 |

`generate_opening`を`opening.rs`に置くのは、`starting_game`が`generate_opening`を呼び、`play.rs`の`play_game_from_opening`が`starting_game`を呼ぶので、`play.rs`に置くと2つのファイルが互いを参照するからである。

`selfplay_gen`の試験16件は次のとおり移す。

| 試験 | 移動先 |
|---|---|
| `rescore_*` | `rescore.rs` |
| `provenance_command_*`（引数の解釈も検査する） | `provenance.rs` |
| `generate_emits_*`、`completed_games_are_merged_*`、`completed_game_merge_rejects_*` | `generate.rs` |
| `human_opening_*` | `opening.rs` |
| `play_game_is_deterministic_*`、`injection_*`、`zero_random_moves_*`、`omitted_max_ply_*` | `play.rs` |
| `random_moves_accepts_only_*` | `cli.rs` |
| `score_distribution_*`、`rate_percent_*` | `statistics.rs` |

`RescoreFixture`は付け直し、来歴、生成、および開始局面の試験が共用するので、`main.rs`の`test_support`に置く。

`lishogi_import`は次のとおりとする。

| パス | 置くもの |
|---|---|
| `main.rs` | `#[global_allocator]`（新設、mimalloc）、`main`、`run`。 |
| `cli.rs` | `Arguments`、`Operation`、`Common`。 |
| `error.rs` | `ImportError`。 |
| `input.rs` | lishogiのNDJSONの型。`InputGame`、`time_control`、`present_field`、`Players`、`Player`、`User`。 |
| `filter.rs` | 対局の除外と報告。`Exclusion`、`Excluded`、`Report`、`metadata_exclusion`、`winner`。 |
| `replay.rs` | 棋譜の再生と検査。`ReplayPosition`、`Replay`、`replay`。 |
| `label.rs` | 局面ごとの探索とラベル付け。`GameDetails`、`Job`、`Processed`、`process_job`。 |
| `output.rs` | 付随ファイルの書き出し。`sidecar`、`create`、`write_json`。 |

`spsa_runner`は、既存の子モジュールの分け方が関心事と一致しているので、`main.rs`の形へ改めたうえで、本体に残る処理だけを次のとおり移す。

| パス | 置くもの |
|---|---|
| `main.rs` | `main`、`execute`。 |
| `cli.rs` | `Arguments`、`Command`。 |
| `engine.rs` | 調整用ビルドのエンジンの解決。`tuning_spec`、`resolve_engine`。 |
| `summary.rs` | 係数と異常件数の表示。`print_theta`、`parameter_line`、`failure_text`。 |
| `model.rs` | 既存の内容に`validate_settings`を加える。`apply.rs`が現在`super::validate_settings`で本体を参照しているのを解消する。 |
| `params.rs`、`runner.rs`、`storage.rs`、`apply.rs` | そのまま移す。 |
| `tests.rs`、`tests/simulation.rs` | そのまま移す。`tests::simulation`のパスを保つため、試験は分割しない。 |

`spsa_runner`の試験を分割しないのは、分割すると`tests.rs`の33件の完全修飾名が変わる一方、`tests::simulation`だけは凍結された測定記録のために残す必要があり、`tests`モジュールの一部だけが移る中途半端な形になるからである。

`random_play`は次のとおりとする。

| パス | 置くもの |
|---|---|
| `main.rs` | `main`、定数`DEFAULT_MAX_PLY`。 |
| `cli.rs` | `Arguments`、`RuleSetArgument`、`parse_rule_set_argument`、`parse_positive_u64`、`parse_positive_u32`。 |
| `seed.rs` | シードの派生。`SPLITMIX_*`、`time_seed`、`splitmix64`、`game_seed`。 |
| `text.rs` | 表記。`rule_code_text`、`rules_text`、`square_text`、`move_text`、`outcome_text`。 |
| `game.rs` | 1局の実行と不変条件の検査。`CompletedGame`、`Outcome`、`Failure`、`fail`、`run_game`。 |
| `summary.rs` | 集計。`WIN_REASONS`、`DRAW_REASONS`、`Summary`、`win_reason_index`、`draw_reason_index`、`report_game`。 |

## 可視性の規則

Rustでは、非公開の項目と欄は、定義したモジュールとその子孫からしか見えない。
移動によって参照元が定義元の子孫でなくなる項目は、参照に必要な最小の可視性へ広げる。
一般の規則は次の3つである。
`src/harness/`の直下の子モジュールで定義し、ハーネスの他の子モジュールから使う項目と欄は`pub(super)`とする。
`src/harness/engine/`の子モジュールで定義し、`engine`の中だけで使う項目と欄は`pub(super)`、`engine`の外のハーネスから使う項目と欄は`pub(in crate::harness)`とする。
実行ファイルの子モジュールで定義し、同じ実行ファイルの他の子モジュールから使う項目、欄、およびメソッドは`pub(super)`とする。たとえば`match_runner`の`RuleSetArgument`の欄`source`と`codes`、`random_play`の`CompletedGame`の欄、および`match_report`の`Report`の欄がこれに当たる。
同じファイルの中だけで使う項目は、試験が使う場合も含めて非公開のまま残す。

ハーネスの関心事をまたぐ参照は、推測に任せず次のとおり指定する。

| 項目 | 可視性 | 理由 |
|---|---|---|
| `engine`モジュール | `mod engine;`（非公開）とし、公開する名前は`mod.rs`から`pub use` | 外部の公開パスを`minase::harness::X`に保つ。 |
| `EngineProcess`、その`start`、`bestmove`、`start_ponder`、`stop_ponder`、`resource_usage`、欄`protocol` | `pub(in crate::harness)` | `game.rs`の`play_game`と`records/convert.rs`の`recorded_game`が使う。 |
| `EngineProcess`のその他の欄（`timeout`、`pondering`、`child`、`input`など）と、定義と別のファイルから呼ぶメソッド | `pub(super)` | `engine`の子のファイルに分けた`impl`と`engine/tests.rs`が使う。`receive_bestmove`のように同じファイルの中だけで呼ぶメソッドは非公開のまま残す。 |
| `CECP_MEMORY_MB`、`cecp_level_text`、`cecp_fixed_time_text`、`cecp_memory_text`、`observe_usi_evaluation`、`observe_usi_search_data`、自由関数の`receive_until`と`receive_until_deadline` | `pub(super)` | `engine`の中の別のファイル（`process.rs`、`think.rs`、`ponder.rs`、`probe.rs`）が使う。 |
| `ThinkRequest`、`ThinkResult`、`EngineEvaluation`、`EngineScore`、`EngineResponse`とそれらの欄 | `pub(in crate::harness)` | `clock.rs`の`GameClocks::think_request`が`ThinkRequest`を作り、`game.rs`が`ThinkResult`を分解し、`records/convert.rs`が`EngineEvaluation`を読む。 |
| `EngineResourceUsage`とその欄 | `pub(in crate::harness)` | `records/convert.rs`が読む。 |
| `CECP_FIXED_TIME_CS`、`validate_cecp_limit`、`canonical_jitto` | `pub(in crate::harness)` | それぞれ`clock.rs`、`player.rs`、`referee.rs`が使う。 |
| `Clock`の欄`remaining`と`Clock::new`、`GameClocks`の`get`と`think_request` | `pub(super)` | `engine/think.rs`が残り時間を書き換えて`think_request`を呼び、`limit.rs`の`SearchLimit::clock`が`Clock::new`を呼び、`game.rs`も`think_request`を呼び、`engine/tests.rs`が`get`で残り時間を読む。`Clock`の単位換算のメソッド、`GameClocks::go_text`、および`zero_clock`は呼び出し元と同じ`clock.rs`に残るので非公開のままとする。 |
| `SearchLimit`の`fixed_go_text`と`clock` | `pub(super)` | `clock.rs`が使う。 |
| `ponder_move` | `pub(super)` | `engine/ponder.rs`が使う。 |
| `recorded_game`、`stored_failure`、`evaluation_record`、`duration_ns` | `pub(in crate::harness)` | `records::convert`は`records`の子なので、`game.rs`の`play_game`と`engine/tests.rs`から使うには`records`の外まで広げる。`recorded_game`だけが呼ぶ`termination_record`は非公開のままとし、現在`pub`の`RecordedGame`はそのまま`pub`とする。 |

表にない項目で可視性の拡大が必要になった場合は、上の3つの一般の規則に従う。
`pub(crate)`を超える拡大と、ハーネスの公開面への新しい`pub`の追加は行わない。
ライブラリへ移す`datagen`の項目の可視性は、「学習局面の生成（ライブラリ）」の節が定める。

## 文書のパス参照

`docs/`（`docs/measurements/`と`docs/audits/`を除く）、`AGENTS.md`、`CLAUDE.md`、`CONTRIBUTING.md`、`README.md`、`RULES.md`、およびコードのdocコメントにある、移動したファイルへの参照を新しい配置へ書き換える。
起案時点で書き換えの対象となる参照を持つ文書は、`src/harness.rs`について`docs/plans/usi-resignation.md`と`docs/plans/alphazero.md`、実行ファイルについて`docs/plans/`の`match-harness.md`、`ponder.md`、`spsa.md`、`random-play.md`、`search-eval-layout.md`、`strength-stage9.md`、および`spec-first-tests/ledgers/d8.md`、ならびに`docs/research/`の3件である。
`src/bin/spsa_runner/`の下の`tests.rs`、`model.rs`、および`tests/simulation.rs`は再編後もパスが変わらないので、これらへの参照は書き換えない。
参照先のファイルは、同じ文にある関数名、型名、または試験名から「再編後の配置」の表に従って決める。
行番号の付いた参照は、移動先の行を特定できれば新しい行番号へ改め、特定できなければ行番号を削ってパスだけを残す。

`docs/plans/spec-first-tests/ledgers/d8.md`は、再編の前から先読みの試験の大半を`src/bin/match_runner/ponder_tests.rs`にあると記しているが、実際には`src/harness/ponder_tests.rs`にある。
この参照も、再編後の実際の置き場所（`src/harness/engine/tests.rs`）へ改める。
一方、同じ台帳の冒頭と各節の見出しにある`src/bin/match_runner.rs`と`src/bin/random_play.rs`は、台帳を作成した2026年8月15日に旧テストが置かれていた場所の記録なので、書き換えない。

[src構成の整理](src-layout.md)の「探索部・評価関数はcoreの外にトップレベルで並べる」の節に、`datagen`の追加と依存の向きを反映する。

## 後続の候補

次の重複は、本書では統合せずに記録だけを残す。
統合するには、出力、保存形式の厳密さ、エラーの文言、または生成の再現値のいずれかを変える判断が要る。

1. 盤上の升と着手の表記`square_text`と`move_text`が、`perft.rs`、`random_play`、およびハーネスの`pair.rs`に3つある。
2. 正の整数の引数の解釈（`parse_positive_u32`など）が、ハーネス、`random_play`、`selfplay_gen`、`match_runner`、および`bench.rs`に散在し、エラーの文言が一部異なる。
3. 規則セットの引数`RuleSetArgument`と`parse_rule_set_argument`が、`minase.rs`、`random_play`、および`match_runner`に3つある。
4. `random_play`の`splitmix64`と`game_seed`は、ライブラリの`rng::splitmix64`と`rng::derive_seed`と同じ計算である。
5. JSONの読み出し`read_json`と不正データのエラーの作成が、`match_runner/storage.rs`、`match_report`、`spsa_runner/storage.rs`、および`selfplay_gen`にある。
6. 実行ディレクトリの型が、`match_runner/storage.rs`（未知の欄を拒否する）と`match_report`（受け入れる）に二重にあり、実行ディレクトリの排他ロックも`match_report`だけが共有ロックを自前で取る。
7. 異常件数の集計`FailureCounts`が、ハーネスと`match_report`に二重にある。
8. Eloの表示`elo_text`が`match_runner`（小数6桁）と`match_report`（小数12桁）で異なる。
9. 異常件数の1行表示が、`match_runner`と`spsa_runner`で同じ書式を別々に持つ。
10. 開始局面のランダム生成が、ハーネス（8〜12手）と`selfplay_gen`（8〜16手）に別々にあり、シードから手数への写像も異なる。
11. 並列実行の方式が、`match_runner`、`selfplay_gen`の生成と付け直し、`lishogi_import`、および`spsa_runner`で4通りある。
12. `bench.rs`の固定局面と、試験用の`src/test_util.rs`の`BENCH_SFENS`が同じ15局面を別々に持つ。
13. 探索局面キーの計算が、`selfplay_gen`（`search_key_history`の末尾）と`lishogi_import`（`zobrist() ^ rights_zobrist()`）で異なる書き方をしている。

## 実装フェーズ

masterから`bin-harness-layout`ブランチを切り、次の順にコミットする。
各コミットは単独でビルドと試験が通る状態にする。

1. `refactor(harness): split the harness module by purpose`。ハーネスを「対局ハーネス」の表の配置へ移し、`pair_seed_replaces_the_zero_output`を`src/rng.rs`へ、`linux_host_resource_probe_reports_cores_and_memory`を`src/harness/environment.rs`へ移す。
2. `refactor(datagen): move code shared by selfplay_gen and lishogi_import into the library`。共有する項目を`src/datagen/`へ移し、`lishogi_import`の`#[path]`による取り込みをやめて`#[global_allocator]`を明示する。
3. `refactor(selfplay): split selfplay_gen and lishogi_import by purpose`。2つの実行ファイルをディレクトリへ分割する。
4. `refactor(match): split match_runner and match_report by purpose`。
5. `refactor(spsa): move spsa_runner into its own directory`。
6. `refactor(random_play): split random_play by purpose`。
7. `docs: update source paths after the harness and binary split`。文書のパス参照を書き換える。

どのコミットも公開パスをなくさないので、`!`と`BREAKING CHANGE:`は付けない。
`minase::harness::X`は平らな公開面を保ち、`datagen`は新しい公開パスを加えるだけだからである。

コードの移動（1から6）はcodexへ委任し、文書の書き換え（7）はClaudeが行う。
codexへの指示には、名前の変更、シグネチャの変更、処理の並べ替え、エラーの文言と出力の書式の変更、重複の統合、および旧パスの再公開を禁じることを含める。
あわせて、「適用範囲」の節の4種類の書き換えを行うことと、新しいファイルの先頭に既存のファイルと同じ体裁の日本語の`//!`コメントを置くことを指示する。

## 検証

基点コミットは、`bin-harness-layout`ブランチを切ったmasterのコミットとする。
各コードのコミットで、`cargo fmt --check`、`cargo clippy --all-targets`（警告0件）、`cargo test`、および`cargo test --features tuning`を実行し、全件通ることを確かめる。
`cargo test`の出力にある試験名の一覧を基点と比べ、名前の末尾（モジュールパスを除いた部分）の集合が一致することも確かめる。
件数は一致しない。基点では`selfplay_gen`の試験16件が`lishogi_import`の試験バイナリでも`selfplay::tests::*`として実行されており、第2フェーズでこの重複がなくなるからである。
名前の末尾の集合は重複の有無によらないので、件数ではなく集合で比べる。

ハーネスと実行ファイルは探索の速さに関わらないので、前例のbenchのNPSの計測は行わず、固定シードの出力の一致で移動の誤りを検出する。
比較用の実行は、基点と各コードのコミットの後で、毎回新しい出力先を指定し、`cargo build --release`で`match_runner`の隣に`usi_random`も作ってから行う。
対局ハーネスは`random`の指定を、`match_runner`と同じディレクトリの`usi_random`に解決するからである。
作業ツリーが未コミットの状態でも比べられるように、`selfplay_gen`と`lishogi_import`には`--allow-dirty`を指定する。
比較は次の4つとする。

1. 対局ハーネスの完全再現契約（[SPRTの手引き](../guides/sprt.md)の「ペア対局と再現性」）の範囲で、`match_runner --run-dir <新しい一時ディレクトリ> --seed 1 --candidate "target/release/minase --protocol usi --rules engine-default" --baseline random --each depth=2 elo --pairs 4`を、同じ作業ディレクトリから実行する。
   標準出力は、経過時間の行と`run_dir`の行を除いて一致することを確かめる。
   `pairs/`の各JSONは、局の実時間、各手の`think_time_ns`とエンジンが報告する`completed_time_ms`、両エンジンのCPU時間、および両エンジンの最大常駐メモリの欄を除いて一致することを確かめる。
   `manifest.json`は、runnerのSHA-256（`runner.sha256`）と基準側`random`のバイナリのSHA-256（`baseline.identity.sha256`）を除いて一致することを確かめる。
2. `match_report`は、時間制御が等しくペア得点の分散が0でない実行ディレクトリしか集計しない。
   そこで基点で1回だけ、`--candidate`と`--baseline`に同じ`target/release/minase`を指定し、`--each time=1000+10 elo --pairs 8`で実行ディレクトリを作る。
   得点の分散が0になった場合はシードを変えて作り直す。
   この同じ実行ディレクトリに対して、基点と各コミットの`match_report`を実行し、出力のJSONが完全に一致することを確かめる。
3. `selfplay_gen`で、固定シード、固定ノード数、`Threads=1`の小さな生成（2局程度）を行う。
   MNSDファイルは、ヘッダの生成コミットの欄（`src/training/records.rs`の`Header::encode`が定めるバイト位置44から83までの40バイト）を除いてバイト単位で一致することを確かめる。
   来歴JSONは、`teacher.generation_commit`と`mnsd_sha256`を除いて一致することを確かめる。`mnsd_sha256`は生成コミットの欄を含むMNSD全体のハッシュ値だからである。
   代わりに、各実行の`mnsd_sha256`がその実行のMNSDファイルのSHA-256と一致することを別に確かめる。
   同じファイルに`inspect`を実行した出力は、生成コミットの行を除いて一致させる。
4. `lishogi_import`を、`tests/lishogi_import.rs`が使う引数（固定シードと固定ノード数）で`tests/fixtures/lishogi_import_cases.ndjson`に対して実行し、出力のMNSD、付随ファイル、および標準出力の報告を、3と同じ欄を除いて比べる。

各コマンドの正確な引数は、基点で実行できることを確かめてから確定し、同じ引数を各コミットで使う。

`src/bin/`と`src/harness/`から`#[path]`が消えたこと（`grep -rn '#\[path' src/bin src/harness`が0件）を確かめる。`src/core/movegen/search_captures.rs`の`#[path]`は本書の対象外なので残る。
文書の書き換えの後に、`docs/`（`docs/measurements/`と`docs/audits/`を除く）とリポジトリ直下の文書に、再編で消えたパス（`src/harness.rs`、`src/harness/tests.rs`、`src/harness/ponder_tests.rs`、`src/harness/records.rs`、および`src/bin/`の`match_runner.rs`、`match_runner/ponder_tests.rs`、`match_report.rs`、`selfplay_gen.rs`、`lishogi_import.rs`、`spsa_runner.rs`、`random_play.rs`）への参照が、前節で記録として残すと定めた台帳d8.mdの3か所を除いて残っていないことを`grep`で確かめる。

## 完了条件

7つのコミットが`bin-harness-layout`ブランチにあり、「検証」の節の確認がすべて満たされ、ブランチがmasterへ統合されたときに完了とする。
本書は独立したリファクタリングなので、[src構成の整理](src-layout.md)および[探索部と評価関数のモジュール再編](search-eval-layout.md)と同じく、ROADMAP.mdのマイルストーン状態表には行を追加しない。

## 参考資料

- [src構成の整理](src-layout.md)。crate構成、coreの責務基準、および依存の向きを定める。
- [探索部と評価関数のモジュール再編](search-eval-layout.md)。本書と同じ方針で`src/search/`と`src/eval/`を分けた前例である。
- [対局ハーネス](match-harness.md)、[SPRTの手引き](../guides/sprt.md)。ハーネスの契約と完全再現契約を定める。
- The Cargo Book, “Target auto-discovery”。`src/bin/*.rs`と`src/bin/*/main.rs`を実行ファイルとして自動で検出する規則を定める。
